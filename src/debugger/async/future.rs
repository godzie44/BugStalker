use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::r#type::TypeIdentity;
use crate::debugger::r#async::AsyncError;
use crate::debugger::variable::value::{RustEnumValue, SpecializedValue, StructValue, Value};
use std::num::ParseIntError;

#[derive(Debug, thiserror::Error)]
pub enum ParseFutureStateError {
    #[error("unexpected future structure representation")]
    UnexpectedStructureRepr,
    #[error("parse suspend state: {0}")]
    ParseSuspendState(ParseIntError),
    #[error("unexpected future state: {0}")]
    UnexpectedState(String),
}

#[derive(Debug, Clone)]
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
    /// Future already in a completed state.
    Ok,
}

#[derive(Debug, Clone)]
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
        const OK_STATE: &str = "Ok";

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
            OK_STATE => Ok(AsyncFnFutureState::Ok),
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

#[derive(Debug, Clone)]
pub struct CustomFuture {
    pub name: TypeIdentity,
}

impl From<&StructValue> for CustomFuture {
    fn from(repr: &StructValue) -> Self {
        let name = repr.type_ident.clone();
        Self { name }
    }
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct TokioJoinHandleFuture {
    pub name: TypeIdentity,
    pub wait_for_task: RelocatedAddress,
}

impl TryFrom<StructValue> for TokioJoinHandleFuture {
    type Error = AsyncError;

    fn try_from(val: StructValue) -> Result<Self, Self::Error> {
        let name = val.type_ident.clone();

        let header_field = val
            .field("raw")
            .and_then(|raw| raw.field("ptr")?.field("pointer"));
        let Some(header) = header_field else {
            return Err(AsyncError::IncorrectAssumption(
                "JoinHandle future should contains `raw` field",
            ));
        };

        let Value::Pointer(ref ptr) = header else {
            return Err(AsyncError::IncorrectAssumption(
                "JoinHandle::raw.ptr.pointer not a pointer",
            ));
        };
        let wait_for_task = ptr
            .value
            .map(|p| RelocatedAddress::from(p as usize))
            .ok_or(AsyncError::IncorrectAssumption(
                "JoinHandle::raw.ptr.pointer not a pointer",
            ))?;

        Ok(Self {
            name,
            wait_for_task,
        })
    }
}

#[derive(Debug, Clone)]
pub enum Future {
    AsyncFn(AsyncFnFuture),
    TokioSleep(TokioSleepFuture),
    TokioJoinHandleFuture(TokioJoinHandleFuture),
    Custom(CustomFuture),
    UnknownFuture,
}
