use super::types::TaskIdValue;
use crate::debugger::TypeDeclaration;
use crate::debugger::r#async::context::TokioAnalyzeContext;
use crate::debugger::r#async::tokio::task::Task;
use crate::debugger::r#async::tokio::task::task_from_header;
use crate::debugger::r#async::{AsyncError, TaskBacktrace};
use crate::debugger::utils::PopIf;
use crate::debugger::variable::dqe::DataCast;
use crate::debugger::variable::dqe::{Dqe, Literal, Selector};
use crate::debugger::variable::execute::DqeExecutor;
use crate::debugger::variable::execute::QueryResult;
use crate::debugger::variable::value::{SupportedScalar, Value};
use crate::debugger::variable::r#virtual::VirtualVariableDie;
use crate::debugger::{Debugger, Error, ThreadSnapshot, Tracee, utils};
use crate::type_from_cache;
use crate::ui::command::parser::expression;
use crate::version::RustVersion;
use crate::weak_error;
use chumsky::Parser;

/// Async worker tasks local queue representation.
pub(super) struct LocalQueue {
    pub _head: u32,
    pub _tail: u32,
    pub buff: Vec<Task>,
}

fn extract_u32_from_atomic_u64(val: Value) -> Option<u32> {
    let value = val.field("v")?.field("value")?;
    if let Value::Scalar(value) = value
        && let Some(SupportedScalar::U64(u)) = value.value
    {
        return Some((u & u32::MAX as u64) as u32);
    }
    None
}

fn extract_u32_from_atomic_32(val: Value) -> Option<u32> {
    let value = val
        .field("inner")?
        .field("value")?
        .field("v")?
        .field("value")?;
    if let Value::Scalar(value) = value
        && let Some(SupportedScalar::U32(u)) = value.value
    {
        return Some(u);
    }

    None
}

impl LocalQueue {
    fn from_query_result(
        debugger: &Debugger,
        local_queue_inner: QueryResult,
    ) -> Option<LocalQueue> {
        let head = local_queue_inner.clone().into_value().field("head")?;
        let head = extract_u32_from_atomic_u64(head)?;

        let tail = local_queue_inner.clone().into_value().field("tail")?;
        let tail = extract_u32_from_atomic_32(tail)?;

        let mut task_buffer = Vec::with_capacity((tail - head) as usize);
        let buffer = local_queue_inner
            .clone()
            .modify_value(|pcx, val| val.field("buffer")?.deref(pcx))?;

        let mut start = head;
        while start < tail {
            let task_header_ptr = buffer.clone().modify_value(|_, val| {
                // extract pointer to `Header` from value of
                // `UnsafeCell<core::mem::maybe_uninit::MaybeUninit<tokio::runtime::task::Notified
                // <alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::Handle,
                // alloc::alloc::Global>>>>` type
                val.index(&Literal::Int(head as i64))?
                    .field("__0")?
                    .field("value")?
                    .field("value")?
                    .field("value")?
                    .field("__0")?
                    .field("raw")?
                    .field("ptr")?
                    .field("pointer")
            })?;
            let task = task_from_header(debugger, task_header_ptr).unwrap();
            task_buffer.push(task);

            start += 1;
        }

        Some(LocalQueue {
            _head: head,
            _tail: tail,
            buff: task_buffer,
        })
    }
}

/// Async worker known states.
pub(super) enum WorkerState {
    RunTask(usize),
    Park,
    Unknown,
}

/// Async worker internal information
pub(super) struct WorkerInternal {
    pub(super) state: WorkerState,
    pub(super) local_queue: LocalQueue,
}

impl WorkerInternal {
    /// Analyze a thread candidate to tokio multy_thread worker.
    /// Return `None` if the thread is definitely not a worker, otherwise return [`WorkerInternal`].
    ///
    /// # Arguments
    ///
    /// * `thread`: thread information
    pub(super) fn analyze(ctx: &mut TokioAnalyzeContext, thread: &ThreadSnapshot) -> Option<Self> {
        let debugger = ctx.debugger_mut();
        let context = debugger
            .read_variable(Dqe::Variable(Selector::by_name("CONTEXT", false)))
            .ok()?
            .pop_if_single_el()?;

        let backtrace = thread.bt.as_ref()?;

        let mut state = None;
        // find frame numer where run_task function executed
        let run_task_frame_num = backtrace.iter().position(|frame| {
            let Some(fn_name) = frame.func_name.as_ref() else {
                return false;
            };
            fn_name.ends_with("Context::run_task")
        });
        if let Some(frame_num) = run_task_frame_num {
            state = Some(WorkerState::RunTask(frame_num));
        }

        let park_frame_num = backtrace.iter().position(|frame| {
            let Some(fn_name) = frame.func_name.as_ref() else {
                return false;
            };
            fn_name.ends_with("Context::park")
        });
        if park_frame_num.is_some() {
            state = Some(WorkerState::Park);
        }

        let worker_run_frame_num = backtrace.iter().position(|frame| {
            let Some(fn_name) = frame.func_name.as_ref() else {
                return false;
            };
            fn_name.ends_with("multi_thread::worker::run")
        });
        if worker_run_frame_num.is_none() {
            state = Some(WorkerState::Unknown);
        }
        let state = state?;

        use utils::PopIf;

        // local queue DQE: var (*(*(*CONTEXT.scheduler.inner).0.core.value.0).run_queue.inner).data
        let mut core_run_queue_inner = context.modify_value(|c, v: Value| {
            v.field("scheduler")?
                .field("inner")?
                .deref(c)?
                .field("__0")?
                .field("core")?
                .field("value")?
                .field("__0")?
                .deref(c)?
                .field("run_queue")?
                .field("inner")
        })?;

        let rustc_version = core_run_queue_inner
            .unit()
            .rustc_version()
            .unwrap_or_default();

        // WAITFORFIX: https://github.com/rust-lang/rust/issues/143241
        // It seems that at the moment (after rustc 1.88), when compiling, the dwarf is not generated quite correctly.
        // The type of field `core_run_queue_inner` in it is not fully described, so BS have to look for the same type
        // in other compilation units and replace it.
        if rustc_version >= RustVersion::new(1, 88, 0) {
            let inner_field = core_run_queue_inner.into_value();

            let raw_addr = match inner_field {
                Value::Struct(struct_value) => struct_value.raw_address,
                Value::Specialized { original, .. } => original.raw_address,
                _ => return None,
            }?;

            let type_info = debugger
            .debugee
            .debug_info_all()
            .iter()
            .find_map(|&debug_info| {
                debug_info
                    .find_type_die_ref_all("Arc<tokio::runtime::scheduler::multi_thread::queue::Inner<alloc::sync::Arc<tokio::runtime::scheduler::multi_thread::handle::Handle, alloc::alloc::Global>>, alloc::alloc::Global>")
                    .into_iter()
                    .find_map(|(offset_of_unit, offset_of_die)| {
                        let mut var_die = VirtualVariableDie::workpiece();
                        let var_die_ref = weak_error!(var_die.init_with_known_type(debug_info, offset_of_unit, offset_of_die))?;
                        let r#type = debugger.gcx().with_type_cache(|tc| weak_error!(type_from_cache!(var_die_ref, tc)))? ;
                        let root_type = r#type.types.get(&r#type.root())?;

                        let TypeDeclaration::Structure { members, .. } = root_type else {
                            return None;
                        };

                        if !members.is_empty() {
                            Some((debug_info.pathname().to_path_buf(), offset_of_unit, offset_of_die))
                        } else {
                            None
                        }
                    })
            })?;

            let executor = DqeExecutor::new(debugger);

            let data_cast = DataCast::new(raw_addr, type_info.0, type_info.1, type_info.2);
            core_run_queue_inner = executor
                .query(&Dqe::DataCast(data_cast))
                .ok()?
                .pop_if_single_el()?;
        }

        let local_queue = core_run_queue_inner.modify_value(|c, v| v.deref(c)?.field("data"))?;

        Some(Self {
            state,
            local_queue: LocalQueue::from_query_result(debugger, local_queue)?,
        })
    }
}

/// Tokio async worker (https://github.com/tokio-rs/tokio/blob/tokio-1.39.x/tokio/src/runtime/scheduler/multi_thread/worker.rs#L91) representation.
#[derive(Debug, Clone)]
pub struct Worker {
    /// Active task number.
    pub active_task: Option<u64>,
    /// Active task taken directly from the stack trace (as an argument to the run function).
    pub active_task_standby: Option<TaskBacktrace>,
    /// Worker worker-local run queue.
    pub queue: Vec<u64>,
    /// A thread that holding a worker.
    pub thread: Tracee,
    /// True if thread in focus. This how `bs` choose an "active worker".
    pub in_focus: bool,
}

/// If thread `thread` is a worker return it, return `Ok(None)` if it's not.
pub fn try_as_worker(
    context: &mut TokioAnalyzeContext,
    thread: &ThreadSnapshot,
) -> Result<Option<Worker>, Error> {
    let debugger = context.debugger_mut();
    debugger.ecx_switch_thread(thread.thread.pid)?;

    let main_debug_info = debugger
        .debugee
        .program_debug_info()?
        .pathname()
        .to_path_buf();
    for i in 0..thread.bt.as_ref().map(|bt| bt.len()).unwrap_or_default() {
        let ecx = debugger.ecx();
        let debug_info = debugger.debugee.debug_info(ecx.location().pc)?;
        if debug_info.pathname() == main_debug_info {
            break;
        }
        debugger.set_frame_into_focus(i as u32)?;
    }

    let Some(worker) = WorkerInternal::analyze(context, thread) else {
        return Ok(None);
    };

    let WorkerState::RunTask(frame_num) = worker.state else {
        return Ok(Some(Worker {
            active_task: None,
            active_task_standby: None,
            queue: Vec::default(),
            thread: thread.thread.clone(),
            in_focus: thread.in_focus,
        }));
    };

    // first switch to run_task frame
    context
        .debugger_mut()
        .set_frame_into_focus(frame_num as u32)?;

    let active_task_from_frame = || -> Option<TaskBacktrace> {
        let task_header_ptr_dqe = expression::parser()
            .parse("task.__0.raw.ptr.pointer")
            .into_output()?;
        let task_header_ptr = context
            .debugger()
            .read_argument(task_header_ptr_dqe)
            .ok()?
            .pop_if_single_el()?;

        let task = task_from_header(context.debugger(), task_header_ptr).ok()?;
        task.backtrace().ok()
    };
    let task_bt_standby = active_task_from_frame();

    let context_initialized = context
        .debugger()
        .read_variable(Dqe::Variable(Selector::by_name("CONTEXT", false)))?
        .pop_if_single_el()
        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
            "CONTEXT not found",
        )))?;

    let current_task_id = context_initialized
        .value()
        .clone()
        .field("current_task_id")
        .and_then(|v| v.field("__0"));

    let mb_task_id = current_task_id
        .and_then(|tid| TaskIdValue::from_value(context_initialized.unit(), tid).ok());

    let worker_bt = Worker {
        active_task: mb_task_id.map(|t| t.into()),
        active_task_standby: task_bt_standby,
        queue: worker.local_queue.buff.into_iter().map(|t| t.id).collect(),
        thread: thread.thread.clone(),
        in_focus: thread.in_focus,
    };

    Ok(Some(worker_bt))
}

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
            .modify_value(|pcx, val| {
                val.field("data_ptr")?
                    .slice(pcx, None, Some(lists_len as usize))
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
                    next_ptr_qr = ptr_qr.clone().modify_value(|pcx, val| {
                        val.deref(pcx)?
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
