use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::variable::{ScalarVariable, SupportedScalar, VariableIR};
use crate::debugger::CreateTransparentBreakpointRequest;
use crate::debugger::{Debugger, Error};
use crate::oracle::{ConsolePlugin, Oracle};
use crate::ui::console::print::style::KeywordView;
use crate::ui::console::print::ExternalPrinter;
use indexmap::IndexMap;
use log::warn;
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;

/// [`TokioOracle`] collect and represent a tokio metrics (like task count, etc.).
#[derive(Default)]
pub struct TokioOracle {
    tasks: RefCell<IndexMap<u64, u64>>,
}

impl TokioOracle {
    /// Create a new oracle.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Oracle for TokioOracle {
    fn name(&self) -> &'static str {
        "tokio"
    }

    fn ready_for_install(&self, dbg: &Debugger) -> bool {
        let poll_symbols = dbg
            .get_symbols("tokio::runtime::task::raw::RawTask::poll*")
            .unwrap_or_default();
        if poll_symbols.is_empty() {
            return false;
        }

        let new_symbols = dbg
            .get_symbols("tokio::runtime::task::raw::RawTask::new*")
            .unwrap_or_default();
        if new_symbols.is_empty() {
            return false;
        }

        let shutdown_symbols = dbg
            .get_symbols("tokio::runtime::task::raw::RawTask::shutdown*")
            .unwrap_or_default();
        if shutdown_symbols.is_empty() {
            return false;
        }

        true
    }

    fn watch_points(self: Rc<Self>) -> Vec<CreateTransparentBreakpointRequest> {
        let oracle = self.clone();
        let poll_handler = move |dbg: &mut Debugger| {
            if let Err(e) = oracle.on_poll(dbg) {
                warn!(target: "tokio oracle", "poll task: {e}")
            }
        };

        let poll_brkpt = CreateTransparentBreakpointRequest::function(
            "tokio::runtime::task::raw::RawTask::poll",
            poll_handler,
        );

        let oracle = self.clone();
        let new_handler = move |dbg: &mut Debugger| {
            if let Err(e) = oracle.on_new(dbg) {
                warn!(target: "tokio oracle", "new task: {e}")
            }
        };
        let new_brkpt = CreateTransparentBreakpointRequest::function(
            "tokio::runtime::task::raw::RawTask::new",
            new_handler,
        );

        let oracle = self.clone();
        let drop_handler = move |dbg: &mut Debugger| {
            if let Err(e) = oracle.on_drop(dbg) {
                warn!(target: "tokio oracle", "drop task: {e}")
            }
        };

        //there is two way when tokio task may be dropped
        let dealloc_brkpt = CreateTransparentBreakpointRequest::function(
            "tokio::runtime::task::raw::RawTask::dealloc",
            drop_handler.clone(),
        );
        let shutdown_brkpt = CreateTransparentBreakpointRequest::function(
            "tokio::runtime::task::raw::RawTask::shutdown",
            drop_handler,
        );

        vec![poll_brkpt, new_brkpt, dealloc_brkpt, shutdown_brkpt]
    }
}

impl ConsolePlugin for TokioOracle {
    fn print(&self, printer: &ExternalPrinter, _: Option<&str>) {
        let tasks = self.tasks.borrow();
        printer.print(format!(
            "{} tasks running\n\n",
            KeywordView::from(tasks.len())
        ));

        if !tasks.is_empty() {
            printer.print("task    poll count");
            for (task_id, poll_cnt) in tasks.iter() {
                printer.print(format!("{task_id:<5}   {poll_cnt}"));
            }
        }
    }

    fn help(&self) -> &str {
        "tokio - tokio runtime metrics"
    }
}

impl TokioOracle {
    fn get_id_from_self(dbg: &mut Debugger) -> Result<Option<u64>, Error> {
        let header_pointer_expr = Expression::Field(
            Expression::Field(
                Expression::Variable(VariableSelector::Name {
                    var_name: "self".to_string(),
                    local: true,
                })
                .boxed(),
                "ptr".to_string(),
            )
            .boxed(),
            "pointer".to_string(),
        );

        let header_args = dbg.read_argument(header_pointer_expr.clone())?;
        let VariableIR::Pointer(header_pointer) = &header_args[0] else {
            return Ok(None);
        };

        let id_offset_args = dbg.read_argument(Expression::Field(
            Expression::Deref(
                Expression::Field(
                    Expression::Deref(header_pointer_expr.boxed()).boxed(),
                    "vtable".to_string(),
                )
                .boxed(),
            )
            .boxed(),
            "id_offset".to_string(),
        ))?;

        let VariableIR::Scalar(ScalarVariable {
            value: Some(SupportedScalar::Usize(id_offset)),
            ..
        }) = &id_offset_args[0]
        else {
            return Ok(None);
        };

        if let Some(header_ptr) = header_pointer.value {
            let id_addr = header_ptr as usize + *id_offset;

            if let Ok(memory) = dbg.read_memory(id_addr, size_of::<u64>()) {
                let task_id = u64::from_ne_bytes(memory.try_into().unwrap());
                return Ok(Some(task_id));
            }
        }

        Ok(None)
    }

    fn on_poll(&self, debugger: &mut Debugger) -> Result<(), Error> {
        if let Some(task_id) = Self::get_id_from_self(debugger)? {
            let mut tasks = self.tasks.borrow_mut();
            let entry = tasks.entry(task_id).or_default();
            *entry += 1;
        }

        Ok(())
    }

    fn on_new(&self, debugger: &mut Debugger) -> Result<(), Error> {
        let id_args = debugger.read_argument(Expression::Field(
            Box::new(Expression::Variable(VariableSelector::Name {
                var_name: "id".to_string(),
                local: true,
            })),
            "0".to_string(),
        ))?;

        if let VariableIR::Scalar(scalar) = &id_args[0] {
            if let Some(SupportedScalar::U64(id_value)) = scalar.value {
                self.tasks.borrow_mut().insert(id_value, 0);
            }
        }

        Ok(())
    }

    fn on_drop(&self, debugger: &mut Debugger) -> Result<(), Error> {
        if let Some(task_id) = Self::get_id_from_self(debugger)? {
            let mut tasks = self.tasks.borrow_mut();
            tasks.remove(&task_id);
        }

        Ok(())
    }
}
