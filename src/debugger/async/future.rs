use crate::debugger::debugee::dwarf::r#type::TypeIdentity;
use crate::debugger::r#async::context::TokioAnalyzeContext;
use crate::debugger::r#async::task_from_header;
use crate::debugger::r#async::AsyncError;
use crate::debugger::r#async::Task;
use crate::debugger::variable::execute::QueryResult;
use crate::debugger::variable::value::{RustEnumValue, SpecializedValue, StructValue, Value};
use crate::debugger::Error;
use std::num::ParseIntError;

/// Container for storing the tasks spawned on a scheduler.
pub struct OwnedList {}

impl OwnedList {
    pub fn try_extract<'a>(
        analyze_ctx: &'a TokioAnalyzeContext,
        context: QueryResult<'a>,
    ) -> Result<Vec<Task>, Error> {
        let list = context
            .modify_value(|ctx, val| {
                val.field("current")?
                    .field("handle")?
                    .field("value")?
                    .field("__0")?
                    .field("__0")?
                    .deref(ctx)?
                    .field("data")?
                    .field("shared")?
                    .field("owned")?
                    .field("list")
            })
            .ok_or(AsyncError::IncorrectAssumption("error while extract field (*CONTEXT.current.handle.value.__0.__0).data.shared.owned.list"))?;

        let lists =
            list.modify_value(|_, l| l.field("lists"))
                .ok_or(AsyncError::IncorrectAssumption(
                    "error while extract field `list.lists`",
                ))?;
        let lists_len = lists
            .clone()
            .into_value()
            .field("length")
            .ok_or(AsyncError::IncorrectAssumption(
                "error while extract field `list.lists.length`",
            ))?
            .into_scalar()
            .and_then(|scalar| scalar.try_as_number())
            .ok_or(AsyncError::IncorrectAssumption(
                "`list.lists.length` should be number",
            ))?;

        let data_qr = lists
            .modify_value(|ctx, val| {
                val.field("data_ptr")?
                    .slice(ctx, None, Some(lists_len as usize))
            })
            .ok_or(AsyncError::IncorrectAssumption(
                "error while extract field `list.lists.data_ptr`",
            ))?;

        let data =
            data_qr
                .clone()
                .into_value()
                .into_array()
                .ok_or(AsyncError::IncorrectAssumption(
                    "`list.lists.data_ptr` should be an array",
                ))?;

        let mut tasks = vec![];
        for el in data.items.unwrap_or_default() {
            let value = el.value;

            let is_parking_lot_mutex = value
                .clone()
                .field("__0")
                .ok_or(AsyncError::IncorrectAssumption("`__0` field not found"))?
                .field("data")
                .is_none();
            let field = if is_parking_lot_mutex { "__1" } else { "__0" };

            let maybe_head = value
                .field(field)
                .and_then(|f| {
                    f.field("data")
                        .and_then(|f| f.field("value").and_then(|f| f.field("head")))
                })
                .ok_or(AsyncError::IncorrectAssumption(
                    "error while extract field `__0(__1).data.value.head` of OwnedList element",
                ))?;

            if let Some(ptr) = maybe_head.field("__0") {
                let ptr = ptr.field("pointer").ok_or(AsyncError::IncorrectAssumption(
                    "`pointer` field not found in OwnedList element",
                ))?;
                let mut next_ptr_qr = data_qr.clone().modify_value(|_, _| Some(ptr));

                while let Some(ptr_qr) = next_ptr_qr {
                    next_ptr_qr = ptr_qr.clone().modify_value(|ctx, val| {
                        val.deref(ctx)?
                            .field("queue_next")?
                            .field("__0")?
                            .field("value")?
                            .field("__0")?
                            .field("pointer")
                    });

                    tasks.push(task_from_header(analyze_ctx.debugger(), ptr_qr)?);
                }
            }
        }

        Ok(tasks)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseFutureStateError {
    #[error("unexpected future structure representation")]
    UnexpectedStructureRepr,
    #[error("parse suspend state: {0}")]
    ParseSuspendState(ParseIntError),
    #[error("unexpected future state: {0}")]
    UnexpectedState(String),
}

#[derive(Debug)]
pub enum AsyncFnFutureState {
    /// A future in this state is suspended at the await point in the code.
    /// The compiler generates a special type to indicate a stop at such await point -
    /// `SuspendX` where X is an integer number of such a point.
    Suspend(u32),
    /// The state of async fn that has been panicked on a previous poll.
    Panicked,
    /// Already resolved async fn. In other words, this future has been
    /// polled and returned Poll::Ready(result) from the poll function.
    Returned,
    /// Already created async fn future but not yet polled (using await or select! or any other
    /// async operation).
    Unresumed,
}

#[derive(Debug)]
pub struct AsyncFnFuture {
    /// Future name (from debug info).
    pub name: String,
    /// Async function name.
    pub async_fn: String,
    /// Async function state.
    pub state: AsyncFnFutureState,
}

impl TryFrom<&RustEnumValue> for AsyncFnFuture {
    type Error = AsyncError;

    fn try_from(repr: &RustEnumValue) -> Result<Self, Self::Error> {
        const UNRESUMED_STATE: &str = "Unresumed";
        const RETURNED_STATE: &str = "Returned";
        const PANICKED_STATE: &str = "Panicked";
        const SUSPEND_STATE: &str = "Suspend";

        let async_fn = repr.type_ident.namespace().join("::").to_string();
        let name = repr.type_ident.name_fmt().to_string();

        let Some(Value::Struct(state)) = repr.value.as_deref().map(|m| &m.value) else {
            return Err(AsyncError::ParseFutureState(
                ParseFutureStateError::UnexpectedStructureRepr,
            ));
        };

        let state = match state.type_ident.name_fmt() {
            UNRESUMED_STATE => Ok(AsyncFnFutureState::Unresumed),
            RETURNED_STATE => Ok(AsyncFnFutureState::Returned),
            PANICKED_STATE => Ok(AsyncFnFutureState::Panicked),
            str => {
                if str.starts_with(SUSPEND_STATE) {
                    let str = str.trim_start_matches(SUSPEND_STATE);
                    let num: u32 = str.parse().map_err(|e| {
                        AsyncError::ParseFutureState(ParseFutureStateError::ParseSuspendState(e))
                    })?;
                    Ok(AsyncFnFutureState::Suspend(num))
                } else {
                    return Err(AsyncError::ParseFutureState(
                        ParseFutureStateError::UnexpectedState(str.to_string()),
                    ));
                }
            }
        }?;

        Ok(Self {
            async_fn,
            name,
            state,
        })
    }
}

#[derive(Debug)]
pub struct CustomFuture {
    pub name: TypeIdentity,
}

impl From<&StructValue> for CustomFuture {
    fn from(repr: &StructValue) -> Self {
        let name = repr.type_ident.clone();
        Self { name }
    }
}

#[derive(Debug)]
pub struct TokioSleepFuture {
    pub name: TypeIdentity,
    pub instant: (i64, u32),
}

impl TryFrom<StructValue> for TokioSleepFuture {
    type Error = AsyncError;

    fn try_from(val: StructValue) -> Result<Self, Self::Error> {
        let name = val.type_ident.clone();

        let Some(Value::Struct(entry)) = val.field("entry") else {
            return Err(AsyncError::IncorrectAssumption(
                "Sleep future should contains `entry` field",
            ));
        };

        let Some(Value::Struct(deadline)) = entry.field("deadline") else {
            return Err(AsyncError::IncorrectAssumption(
                "Sleep future should contains `entry.deadline` field",
            ));
        };

        let Some(Value::Specialized {
            value: Some(SpecializedValue::Instant(instant)),
            ..
        }) = deadline.field("std")
        else {
            return Err(AsyncError::IncorrectAssumption(
                "Sleep future should contains `entry.deadline.std` field",
            ));
        };

        Ok(Self { name, instant })
    }
}

#[derive(Debug)]
pub enum Future {
    AsyncFn(AsyncFnFuture),
    TokioSleep(TokioSleepFuture),
    Custom(CustomFuture),
    UnknownFuture,
}
