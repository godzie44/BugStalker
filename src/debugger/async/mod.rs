mod context;
mod future;
mod park;
mod types;
mod worker;

pub use crate::debugger::r#async::future::AsyncFnFutureState;
pub use crate::debugger::r#async::future::Future;
pub use crate::debugger::r#async::park::BlockThread;
pub use crate::debugger::r#async::worker::Worker;

use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::unit::DieVariant;
use crate::debugger::r#async::context::TokioAnalyzeContext;
use crate::debugger::r#async::future::OwnedList;
use crate::debugger::r#async::future::ParseFutureStateError;
use crate::debugger::r#async::future::{AsyncFnFuture, CustomFuture, TokioSleepFuture};
use crate::debugger::r#async::park::try_as_park_thread;
use crate::debugger::r#async::worker::try_as_worker;
use crate::debugger::utils::PopIf;
use crate::debugger::variable::dqe::{Dqe, PointerCast, Selector};
use crate::debugger::variable::execute::QueryResult;
use crate::debugger::variable::value::RustEnumValue;
use crate::debugger::variable::value::Value;
use crate::debugger::{Debugger, Error};
use crate::{disable_when_not_stared, resolve_unit_call, weak_error};
use nix::unistd::Pid;
use std::rc::Rc;

#[derive(Debug)]
pub struct TaskBacktrace {
    /// Tokio task id.
    pub task_id: u64,
    /// Futures stack.
    pub futures: Vec<Future>,
}

/// Async backtrace - represent information about current async runtime state.
#[derive(Debug)]
pub struct AsyncBacktrace {
    /// Async workers information.
    pub workers: Vec<Worker>,
    /// Blocking (parked) threads information.
    pub block_threads: Vec<BlockThread>,
    /// Known tasks. Each task has own backtrace, where root is an async function.
    pub tasks: Rc<Vec<TaskBacktrace>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AsyncError {
    #[error("Backtrace for thread {0} not found")]
    BacktraceShouldExist(Pid),
    #[error("Parse future state: {0}")]
    ParseFutureState(ParseFutureStateError),
    #[error("Incorrect assumption about async runtime: {0}")]
    IncorrectAssumption(&'static str),
}

struct Task {
    id: u64,
    repr: RustEnumValue,
}

impl Task {
    fn from_enum_repr(id: u64, repr: RustEnumValue) -> Self {
        Self { id, repr }
    }

    fn backtrace(self) -> Result<TaskBacktrace, AsyncError> {
        Ok(TaskBacktrace {
            task_id: self.id,
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

                    match type_ident.name_fmt() {
                        "Sleep" => {
                            let future = weak_error!(TokioSleepFuture::try_from(next_future))
                                .map(Future::TokioSleep)
                                .unwrap_or(Future::UnknownFuture);
                            result.push(future);
                        }
                        _ => {
                            let future = CustomFuture::from(&next_future);
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

/// Get task information using `Header` structure.
/// See https://github.com/tokio-rs/tokio/blob/tokio-1.38.0/tokio/src/runtime/task/core.rs#L150
fn task_from_header<'a>(
    debugger: &'a Debugger,
    task_header_ptr: QueryResult<'a>,
) -> Result<Task, Error> {
    let Value::Pointer(ref ptr) = task_header_ptr.value() else {
        return Err(Error::Async(AsyncError::IncorrectAssumption(
            "task.__0.raw.ptr.pointer not a pointer",
        )));
    };

    let vtab_ptr = task_header_ptr
        .clone()
        .modify_value(|ctx, val| val.deref(ctx)?.field("vtable")?.deref(ctx)?.field("poll"))
        .unwrap();
    let Value::Pointer(ref fn_ptr) = vtab_ptr.value() else {
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
    let iter = resolve_unit_call!(debug_info.inner, unit, type_iter);
    let mut cell_type_die = None;
    for (typ, offset) in iter {
        if typ.starts_with("Cell") {
            let typ_entry = resolve_unit_call!(debug_info.inner, unit, find_entry, *offset);
            if let Some(typ_entry) = typ_entry {
                if let DieVariant::StructType(ref struct_type) = typ_entry.die {
                    let mut s_tpl_found = false;
                    let mut t_tpl_found = false;

                    typ_entry.node.children.iter().for_each(|&idx| {
                        let entry = resolve_unit_call!(debug_info.inner, unit, entry, idx);
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
        cell_type_die.base_attributes.name.unwrap()
    );
    // let dqe = format!("*(({typ}){}).pointer", ptr);
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
        .pop_if_cond(|v| v.len() == 1)
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
    let task = Task::from_enum_repr(task_id, future);
    Ok(task)
}

impl Debugger {
    pub fn async_backtrace(&mut self) -> Result<AsyncBacktrace, Error> {
        disable_when_not_stared!(self);

        let expl_ctx = self.exploration_ctx().clone();

        let threads = self.debugee.thread_state(&expl_ctx)?;
        let mut analyze_context = TokioAnalyzeContext::new(self);
        let mut backtrace = AsyncBacktrace {
            workers: vec![],
            block_threads: vec![],
            tasks: Rc::new(vec![]),
        };

        let mut tasks = Rc::new(vec![]);

        for thread in threads {
            let worker = weak_error!(try_as_worker(&mut analyze_context, &thread));

            if let Some(Some(w)) = worker {
                // if this is an async worker we need to extract whole future list once
                if tasks.is_empty() {
                    let mut context_initialized_var = analyze_context
                        .debugger()
                        .read_variable(Dqe::Variable(Selector::by_name("CONTEXT", false)))?;
                    let context_initialized = context_initialized_var
                        .pop_if_cond(|results| results.len() == 1)
                        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
                            "CONTEXT not found",
                        )))?;

                    tasks = Rc::new(
                        OwnedList::try_extract(&analyze_context, context_initialized)?
                            .into_iter()
                            .map(|t| t.backtrace().unwrap())
                            .collect(),
                    );
                    backtrace.tasks = tasks.clone();
                }

                backtrace.workers.push(w);
            } else {
                // maybe thread block on future?
                let thread = weak_error!(try_as_park_thread(&mut analyze_context, &thread));
                if let Some(Some(pt)) = thread {
                    backtrace.block_threads.push(pt);
                }
            }
        }

        self.expl_ctx_swap(expl_ctx);

        Ok(backtrace)
    }
}
