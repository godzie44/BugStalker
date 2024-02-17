use crate::debugger::unwind::{Backtrace, FrameSpan};
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::variable::{ScalarVariable, StructVariable, SupportedScalar, VariableIR};
use crate::debugger::CreateTransparentBreakpointRequest;
use crate::debugger::{Debugger, Error};
use crate::oracle::{ConsolePlugin, Oracle, TuiPlugin};
use crate::ui::console::print::style::KeywordView;
use crate::ui::console::print::ExternalPrinter;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::Msg;
use chrono::Duration;
use indexmap::IndexMap;
use log::warn;
use std::borrow::Cow;
use std::mem::size_of;
use std::sync::Arc;
use std::time::Instant;
use strum_macros::{Display, EnumString};
use tuirealm::Component;

#[derive(Debug, Display, EnumString, Clone, Copy)]
enum TaskTarget {
    #[strum(serialize = "tokio::task::blocking")]
    Blocking,
    #[strum(serialize = "tokio::task")]
    Task,
    #[strum(serialize = "unknown")]
    Unknown,
}

impl TaskTarget {
    /// Return task target and a frame of possible caller.
    fn from_backtrace(bt: &Backtrace) -> (Self, Option<&FrameSpan>) {
        for (i, frame) in bt.iter().enumerate() {
            if let Some(ref fn_name) = frame.func_name {
                match fn_name.as_str() {
                    "tokio::runtime::runtime::Runtime::spawn"
                    | "tokio::task::builder::Builder::spawn"
                    | "tokio::task::spawn::spawn" => return (Self::Task, bt.get(i + 1)),
                    "tokio::runtime::blocking::pool::spawn_blocking"
                    | "tokio::runtime::blocking::pool::spawn_mandatory_blocking"
                    | "tokio::runtime::runtime::Runtime::spawn_blocking" => {
                        return (Self::Blocking, bt.get(i + 1))
                    }
                    _ => {}
                }
            }
        }

        (Self::Unknown, None)
    }
}

#[derive(Debug, Display, EnumString, Clone, Copy)]
enum State {
    Initial,
    Idle,
    Notified,
    Running,
    Cancelled,
    Complete,
}

#[derive(Clone)]
struct Task {
    _id: u64,
    ptr: Option<*const ()>,
    polls: u64,
    created_at: Instant,
    state: State,
    target: TaskTarget,
    caller: Option<String>,
    dropped_at: Option<Instant>,
}

impl Task {
    fn new(id: u64, bt: Option<&Backtrace>) -> Self {
        let (target, caller_frame) = if let Some(bt) = bt {
            TaskTarget::from_backtrace(bt)
        } else {
            (TaskTarget::Unknown, None)
        };

        Self {
            _id: id,
            polls: 0,
            created_at: Instant::now(),
            state: State::Initial,
            ptr: None,
            target,
            caller: caller_frame.and_then(|f| f.func_name.to_owned()),
            dropped_at: None,
        }
    }

    fn inc_poll(&mut self) {
        self.polls += 1;
    }

    fn update_state(&mut self, state: usize) {
        // list of tokio states
        const RUNNING: usize = 0b0001;
        const COMPLETE: usize = 0b0010;
        const NOTIFIED: usize = 0b100;
        const CANCELLED: usize = 0b100_000;

        self.state = if state & RUNNING == RUNNING {
            State::Running
        } else if state & NOTIFIED == NOTIFIED {
            State::Notified
        } else if state & CANCELLED == CANCELLED {
            State::Cancelled
        } else if state & COMPLETE == COMPLETE {
            State::Complete
        } else if state & (RUNNING | COMPLETE) == 0 {
            State::Idle
        } else {
            State::Initial
        };
    }

    /// Calls when a tokio runtime drop this task.
    fn set_drop(&mut self) {
        self.dropped_at = Some(Instant::now());
    }

    fn task_time(&self) -> Duration {
        if let Some(dropped_at) = self.dropped_at {
            Duration::from_std(dropped_at.duration_since(self.created_at)).expect("infallible")
        } else {
            Duration::from_std(self.created_at.elapsed()).expect("infallible")
        }
    }
}

/// [`TokioOracle`] collect and represent a tokio metrics (like task count, etc.).
#[derive(Default)]
pub struct TokioOracle {
    tasks: std::sync::Mutex<IndexMap<u64, Task>>,
}

// SAFETY: this is safe to use tokio oracle from any thread until someone try to
// dereference task pointers (this lead reading of tracee threads memory),
// dereference may be done only from tracer (ptrace) thread
unsafe impl Send for TokioOracle {}
unsafe impl Sync for TokioOracle {}

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

    fn watch_points(self: Arc<Self>) -> Vec<CreateTransparentBreakpointRequest> {
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

        //there are two ways when a tokio task may be dropped
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
        let tasks = self.tasks.lock().unwrap().clone();
        let tasks: IndexMap<_, _> = tasks
            .into_iter()
            .filter(|(_, t)| t.dropped_at.is_none())
            .collect();
        printer.println(format!(
            "{} tasks running\n",
            KeywordView::from(tasks.len())
        ));

        if !tasks.is_empty() {
            let header = format!(
                "{task:<5} {state:<10} {time:<5} {target:<25} {caller:<40} {polls}",
                task = "task",
                state = "state",
                time = "time",
                target = "target",
                caller = "caller",
                polls = "polls",
            );

            printer.println(header);
            for (task_id, task) in tasks.iter() {
                let state = task.state;
                let elapsed = task.task_time();
                let minutes = elapsed.num_minutes();
                let seconds = elapsed.num_seconds() % 60;
                let time = format!("{minutes}m{seconds}s");

                fn max_n_symbols(s: &str, n: usize) -> Cow<'_, str> {
                    if s.len() > n {
                        Cow::Owned(s[..n - 3].to_string() + "...")
                    } else {
                        Cow::Borrowed(s)
                    }
                }

                printer.println(format!(
                    "{task_id:<5} {state:<10} {time:<5} {target:<25} {caller:<40} {polls}",
                    target = max_n_symbols(&task.target.to_string(), 25),
                    caller = max_n_symbols(task.caller.as_deref().unwrap_or("unknown"), 40),
                    polls = task.polls,
                ));
            }
        }
    }

    fn help(&self) -> &str {
        "tokio - tokio runtime metrics"
    }
}

impl TokioOracle {
    /// Return underline value of loom `AtomicUsize` structure.
    fn extract_value_from_atomic_usize(&self, val: &StructVariable) -> Option<usize> {
        if let VariableIR::Struct(inner) = val.members.first()? {
            if let VariableIR::Struct(value) = inner.members.first()? {
                if let VariableIR::Struct(v) = value.members.first()? {
                    if let VariableIR::Scalar(value) = v.members.first()? {
                        if let Some(SupportedScalar::Usize(usize)) = value.value {
                            return Some(usize);
                        }
                    }
                }
            }
        }

        None
    }

    /// Refresh all non-dropped tasks by reading tracee memory and reflect tasks.
    fn refresh_tasks(&self, dbg: &mut Debugger) {
        let mut tasks = self.tasks.lock().unwrap();

        tasks
            .iter_mut()
            .filter(|(_, task)| task.dropped_at.is_none())
            .for_each(|(_, task)| {
                if let Some(ptr) = task.ptr {
                    let var = dbg.read_variable(Expression::Deref(
                        Expression::PtrCast(
                            ptr as usize,
                            "*const tokio::runtime::task::core::Header".to_string(),
                        )
                        .boxed(),
                    ));

                    if let Ok(Some(VariableIR::Struct(header_struct))) =
                        var.as_ref().map(|v| v.first())
                    {
                        for member in &header_struct.members {
                            if let VariableIR::Struct(state_member) = member {
                                if state_member.identity.name.as_deref() != Some("state") {
                                    continue;
                                }

                                let val = state_member.members.first();

                                if let Some(VariableIR::Struct(val)) = val {
                                    if let Some(state) = self.extract_value_from_atomic_usize(val) {
                                        task.update_state(state)
                                    }
                                }
                            }
                        }
                    }
                }
            });
    }

    /// Read `self` function argument, interpret it as a task and return (task_id, task pointer) pair.
    fn get_header_from_self(dbg: &mut Debugger) -> Result<Option<(u64, *const ())>, Error> {
        let header_pointer_expr = Expression::Field(
            Expression::Field(
                Expression::Variable(VariableSelector::Name {
                    var_name: "self".to_string(),
                    only_local: true,
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

        let Some(VariableIR::Scalar(ScalarVariable {
            value: Some(SupportedScalar::Usize(id_offset)),
            ..
        })) = &id_offset_args.first()
        else {
            return Ok(None);
        };

        if let Some(header_ptr) = header_pointer.value {
            let id_addr = header_ptr as usize + *id_offset;

            if let Ok(memory) = dbg.read_memory(id_addr, size_of::<u64>()) {
                let task_id = u64::from_ne_bytes(memory.try_into().unwrap());
                return Ok(Some((task_id, header_ptr)));
            }
        }

        Ok(None)
    }

    fn on_poll(&self, debugger: &mut Debugger) -> Result<(), Error> {
        if let Some((task_id, task_ptr)) = Self::get_header_from_self(debugger)? {
            let mut tasks = self.tasks.lock().unwrap();
            let entry = tasks.entry(task_id).or_insert_with(|| {
                let bt = debugger
                    .backtrace(debugger.exploration_ctx().pid_on_focus())
                    .ok();
                Task::new(task_id, bt.as_ref())
            });
            entry.ptr = Some(task_ptr);
            entry.inc_poll();
        }

        self.refresh_tasks(debugger);

        Ok(())
    }

    fn on_new(&self, debugger: &mut Debugger) -> Result<(), Error> {
        let id_args = debugger.read_argument(Expression::Field(
            Box::new(Expression::Variable(VariableSelector::Name {
                var_name: "id".to_string(),
                only_local: true,
            })),
            "0".to_string(),
        ))?;

        if let VariableIR::Scalar(scalar) = &id_args[0] {
            if let Some(SupportedScalar::U64(id_value)) = scalar.value {
                let bt = debugger
                    .backtrace(debugger.exploration_ctx().pid_on_focus())
                    .ok();

                self.tasks
                    .lock()
                    .unwrap()
                    .insert(id_value, Task::new(id_value, bt.as_ref()));
            }
        }

        self.refresh_tasks(debugger);

        Ok(())
    }

    fn on_drop(&self, debugger: &mut Debugger) -> Result<(), Error> {
        if let Some((task_id, _)) = Self::get_header_from_self(debugger)? {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.set_drop();
            }
        }

        self.refresh_tasks(debugger);

        Ok(())
    }
}

impl TuiPlugin for TokioOracle {
    fn make_tui_component(self: Arc<Self>) -> Box<dyn Component<Msg, UserEvent>> {
        Box::new(tui::TokioComponent::new(self))
    }
}

pub mod tui {
    use crate::oracle::builtin::tokio::TokioOracle;
    use crate::ui::tui::app::port::UserEvent;
    use crate::ui::tui::Msg;
    use std::sync::Arc;
    use tui_realm_stdlib::Table;
    use tuirealm::command::{Cmd, Direction, Position};
    use tuirealm::event::{Key, KeyEvent};
    use tuirealm::props::{Alignment, BorderType, Borders, Color, Style, TableBuilder, TextSpan};
    use tuirealm::{AttrValue, Attribute, Component, Event, MockComponent};

    #[derive(MockComponent)]
    pub struct TokioComponent {
        component: Table,
        oracle: Arc<TokioOracle>,
    }

    impl TokioComponent {
        pub fn new(oracle: Arc<TokioOracle>) -> Self {
            let list = Table::default()
                .borders(
                    Borders::default()
                        .modifiers(BorderType::Rounded)
                        .color(Color::LightYellow),
                )
                .title("Active tasks", Alignment::Center)
                .inactive(Style::default().fg(Color::Gray))
                .scroll(true)
                .highlighted_color(Color::LightYellow)
                .highlighted_str("▶")
                .rewind(true)
                .step(4)
                .widths(&[5, 5, 5, 15, 30, 5])
                .headers(&["Task ID", "State", "Time", "Target", "Caller", "Polls"])
                .table(
                    TableBuilder::default()
                        .add_col(TextSpan::from(""))
                        .add_col(TextSpan::from(""))
                        .add_col(TextSpan::from(""))
                        .add_col(TextSpan::from(""))
                        .add_col(TextSpan::from(""))
                        .add_col(TextSpan::from(""))
                        .add_row()
                        .build(),
                );

            Self {
                component: list,
                oracle,
            }
        }

        fn refresh_list(&mut self) {
            let mut tasks_table_builder = TableBuilder::default();

            let tasks = self.oracle.tasks.lock().unwrap().clone();

            if tasks.is_empty() {
                tasks_table_builder
                    .add_col(TextSpan::from(""))
                    .add_col(TextSpan::from(""))
                    .add_col(TextSpan::from(""))
                    .add_col(TextSpan::from(""))
                    .add_col(TextSpan::from(""))
                    .add_col(TextSpan::from(""))
                    .add_row();
            }
            for (id, task) in tasks {
                let fg = if task.dropped_at.is_some() {
                    Color::Gray
                } else {
                    Color::Reset
                };

                let elapsed = task.task_time();
                let minutes = elapsed.num_minutes();
                let seconds = elapsed.num_seconds() % 60;

                tasks_table_builder
                    .add_col(TextSpan::from(id.to_string()).fg(fg))
                    .add_col(TextSpan::from(task.state.to_string()).fg(fg))
                    .add_col(TextSpan::from(format!("{minutes}m{seconds}s")).fg(fg))
                    .add_col(TextSpan::from(task.target.to_string()).fg(fg))
                    .add_col(
                        TextSpan::from(task.caller.as_deref().unwrap_or("unknown").to_string())
                            .fg(fg),
                    )
                    .add_col(TextSpan::from(task.polls.to_string()).fg(fg))
                    .add_row();
            }

            self.component.attr(
                Attribute::Content,
                AttrValue::Table(tasks_table_builder.build()),
            );
        }
    }

    impl Component<Msg, UserEvent> for TokioComponent {
        fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
            match ev {
                Event::Keyboard(KeyEvent {
                    code: Key::Down, ..
                }) => {
                    self.perform(Cmd::Move(Direction::Down));
                }
                Event::Keyboard(KeyEvent { code: Key::Up, .. }) => {
                    self.perform(Cmd::Move(Direction::Up));
                }
                Event::Keyboard(KeyEvent {
                    code: Key::PageDown,
                    ..
                }) => {
                    self.perform(Cmd::Scroll(Direction::Down));
                }
                Event::Keyboard(KeyEvent {
                    code: Key::PageUp, ..
                }) => {
                    self.perform(Cmd::Scroll(Direction::Up));
                }
                Event::Keyboard(KeyEvent {
                    code: Key::Home, ..
                }) => {
                    self.perform(Cmd::GoTo(Position::Begin));
                }
                Event::Keyboard(KeyEvent { code: Key::End, .. }) => {
                    self.perform(Cmd::GoTo(Position::End));
                }
                Event::Tick => {
                    self.refresh_list();
                }
                _ => {}
            }

            Some(Msg::None)
        }
    }
}
