use super::{AsyncError, Future, TaskBacktrace, types};
use crate::{
    debugger::{
        Debugger, Error,
        address::RelocatedAddress,
        r#async::future::{AsyncFnFuture, CustomFuture, TokioJoinHandleFuture, TokioSleepFuture},
        debugee::dwarf::unit::DieVariant,
        utils::PopIf,
        variable::{
            dqe::{Dqe, PointerCast},
            execute::QueryResult,
            value::{RustEnumValue, Value},
        },
    },
    resolve_unit_call, weak_error,
};
use core::str;

pub struct Task {
    pub id: u64,
    repr: RustEnumValue,
    raw_ptr: RelocatedAddress,
}

impl Task {
    pub fn from_enum_repr(raw_ptr: RelocatedAddress, id: u64, repr: RustEnumValue) -> Self {
        Self { raw_ptr, id, repr }
    }

    pub fn backtrace(self) -> Result<TaskBacktrace, AsyncError> {
        Ok(TaskBacktrace {
            task_id: self.id,
            raw_ptr: self.raw_ptr,
            futures: self.future_stack()?,
        })
    }

    fn future_stack(self) -> Result<Vec<Future>, AsyncError> {
        const AWAITEE_FIELD: &str = "__awaitee";

        let mut result = vec![];

        let mut next_future_repr = Some(self.repr);
        while let Some(next_future) = next_future_repr.take() {
            let future = AsyncFnFuture::try_from(&next_future)?;
            result.push(Future::AsyncFn(future));

            let Some(member) = next_future.value else {
                break;
            };
            let Value::Struct(val) = member.value else {
                break;
            };

            let awaitee = val.field(AWAITEE_FIELD);
            match awaitee {
                Some(Value::RustEnum(next_future)) => {
                    next_future_repr = Some(next_future);
                }
                Some(Value::Struct(next_future)) => {
                    let type_ident = &next_future.type_ident;
                    let fmt_name = type_ident.name_fmt();
                    match fmt_name {
                        "Sleep" => {
                            let future = weak_error!(TokioSleepFuture::try_from(next_future))
                                .map(Future::TokioSleep)
                                .unwrap_or(Future::UnknownFuture);
                            result.push(future);
                        }
                        _ if fmt_name.contains("JoinHandle") => {
                            let future = weak_error!(TokioJoinHandleFuture::try_from(next_future))
                                .map(Future::TokioJoinHandleFuture)
                                .unwrap_or(Future::UnknownFuture);
                            result.push(future);
                        }
                        _ => {
                            let future: CustomFuture = CustomFuture::from(&next_future);
                            result.push(Future::Custom(future));
                        }
                    }

                    break;
                }
                _ => {}
            }
        }

        Ok(result)
    }
}

/// Return task header state value and point pair.
pub fn task_header_state_value_and_ptr(
    debugger: &Debugger,
    header_ptr: RelocatedAddress,
) -> Result<(usize, usize), Error> {
    let dqe: Dqe = Dqe::Field(
        Dqe::Deref(
            Dqe::Field(
                Dqe::PtrCast(PointerCast {
                    ptr: header_ptr.as_usize(),
                    ty: types::header_type_name().to_string(),
                })
                .boxed(),
                "pointer".to_string(),
            )
            .boxed(),
        )
        .boxed(),
        "state".to_string(),
    );

    let state = debugger
        .read_variable(dqe)?
        .pop_if_single_el()
        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
            "Header::state field not found in structure",
        )))?;

    let state = state
        .modify_value(|_, state| {
            state
                .field("val")?
                .field("inner")?
                .field("value")?
                .field("v")?
                .field("value")
        })
        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
            "Unexpected Header::state layout",
        )))?;

    let value = state.into_value();
    let addr = value
        .in_memory_location()
        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
            "Header::state without address",
        )))?;
    let value = value
        .into_scalar()
        .and_then(|s| s.try_as_number())
        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
            "Header::state should be usize",
        )))? as usize;

    Ok((value, addr))
}

/// Get task information using `Header` structure.
/// See https://github.com/tokio-rs/tokio/blob/tokio-1.38.0/tokio/src/runtime/task/core.rs#L150
pub fn task_from_header<'a>(
    debugger: &'a Debugger,
    task_header_ptr: QueryResult<'a>,
) -> Result<Task, Error> {
    let Value::Pointer(ptr) = task_header_ptr.value() else {
        return Err(Error::Async(AsyncError::IncorrectAssumption(
            "task.__0.raw.ptr.pointer not a pointer",
        )));
    };

    let vtab_ptr = task_header_ptr
        .clone()
        .modify_value(|ctx, val| val.deref(ctx)?.field("vtable")?.deref(ctx)?.field("poll"))
        .unwrap();
    let Value::Pointer(fn_ptr) = vtab_ptr.value() else {
        return Err(Error::Async(AsyncError::IncorrectAssumption(
            "(*(*task.__0.raw.ptr.pointer).vtable).poll should be a pointer",
        )));
    };
    let poll_fn_addr = fn_ptr
        .value
        .map(|a| RelocatedAddress::from(a as usize))
        .ok_or(AsyncError::IncorrectAssumption(
            "(*(*task.__0.raw.ptr.pointer).vtable).poll fn pointer should contain a value",
        ))?;

    // Now using the value of fn pointer finds poll function of this task
    let poll_fn_addr_global = poll_fn_addr.into_global(&debugger.debugee)?;
    let debug_info = debugger.debugee.debug_info(poll_fn_addr)?;
    let poll_fn_die = debug_info.find_function_by_pc(poll_fn_addr_global)?.ok_or(
        AsyncError::IncorrectAssumption("poll function for a task not found"),
    )?;

    // poll function should have `T: Future` and `S: Schedule` type parameters
    let t_tpl_die =
        poll_fn_die
            .get_template_parameter("T")
            .ok_or(AsyncError::IncorrectAssumption(
                "poll function should have `T` type argument",
            ))?;
    let s_tpl_die =
        poll_fn_die
            .get_template_parameter("S")
            .ok_or(AsyncError::IncorrectAssumption(
                "poll function should have `S` type argument",
            ))?;

    // Now we try to find suitable `tokio::runtime::task::core::Cell<T, S>` type
    let unit = poll_fn_die.unit();
    let iter = resolve_unit_call!(debug_info.dwarf(), unit, type_iter);
    let mut cell_type_die = None;
    for (typ, offset) in iter {
        if typ.starts_with("Cell") {
            let typ_entry = resolve_unit_call!(debug_info.dwarf(), unit, find_entry, *offset);
            if let Some(typ_entry) = typ_entry {
                if let DieVariant::StructType(ref struct_type) = typ_entry.die {
                    let mut s_tpl_found = false;
                    let mut t_tpl_found = false;

                    typ_entry.node.children.iter().for_each(|&idx| {
                        let entry = resolve_unit_call!(debug_info.dwarf(), unit, entry, idx);
                        if let DieVariant::TemplateType(ref tpl) = entry.die {
                            if tpl.type_ref == t_tpl_die.type_ref {
                                t_tpl_found = true;
                            }
                            if tpl.type_ref == s_tpl_die.type_ref {
                                s_tpl_found = true;
                            }
                        }
                    });

                    if s_tpl_found & t_tpl_found {
                        cell_type_die = Some(struct_type.clone());
                        break;
                    }
                }
            }
        }
    }

    let cell_type_die = cell_type_die.ok_or(AsyncError::IncorrectAssumption(
        "tokio::runtime::task::core::Cell<T, S> type not found",
    ))?;

    // Cell type found, not cast task pointer to this type
    let ptr = RelocatedAddress::from(ptr.value.unwrap() as usize);
    let typ = format!(
        "NonNull<tokio::runtime::task::core::{}>",
        cell_type_die.name.unwrap()
    );

    let dqe = Dqe::Deref(
        Dqe::Field(
            Dqe::PtrCast(PointerCast {
                ptr: ptr.as_usize(),
                ty: typ,
            })
            .boxed(),
            "pointer".to_string(),
        )
        .boxed(),
    );

    // having this type now possible to take underlying future and task_id
    let task_id_dqe = Dqe::Field(
        Dqe::Field(dqe.clone().boxed(), "core".to_string()).boxed(),
        "task_id".to_string(),
    );
    let future_dqe = Dqe::Field(
        Dqe::Field(
            Dqe::Field(
                Dqe::Field(
                    Dqe::Field(
                        Dqe::Field(dqe.clone().boxed(), "core".to_string()).boxed(),
                        "stage".to_string(),
                    )
                    .boxed(),
                    "stage".to_string(),
                )
                .boxed(),
                "__0".to_string(),
            )
            .boxed(),
            "value".to_string(),
        )
        .boxed(),
        "__0".to_string(),
    );

    let task_id = debugger
        .read_variable(task_id_dqe)?
        .pop_if_single_el()
        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
            "task_id field not found in task structure",
        )))?;
    let task_id: u64 = types::TaskIdValue::from_value(unit, task_id.into_value())?.into();

    let mut future = debugger.read_variable(future_dqe)?;
    let Some(QueryResult {
        value: Some(Value::RustEnum(future)),
        ..
    }) = future.pop()
    else {
        return Err(Error::Async(AsyncError::IncorrectAssumption(
            "task root future not found",
        )));
    };
    let task = Task::from_enum_repr(ptr, task_id, future);
    Ok(task)
}
