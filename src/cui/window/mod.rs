use crate::cui::hook::CuiHook;
use crate::cui::window::complex::ComplexComponent;
use crate::cui::window::help::ContextHelp;
use crate::cui::window::input::UserInput;
use crate::cui::window::main::{DebugeeView, MainLogs, Variables};
use crate::cui::window::tabs::TabVariant;
use crate::cui::{Event, SharedRenderData};
use crate::debugger::command::Continue;
use crate::debugger::Debugger;
use crate::tab_switch_action;
use crossterm::event::{DisableMouseCapture, KeyCode, KeyEvent};
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
use std::collections::HashMap;
use std::io::StdoutLock;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::{Frame, Terminal};

mod complex;
mod help;
mod input;
mod main;
mod tabs;

#[derive(Clone)]
pub(super) struct RenderContext {
    data: SharedRenderData,
}

impl RenderContext {
    pub(super) fn new(data: SharedRenderData) -> Self {
        Self { data }
    }
}

trait CuiComponent {
    fn render(
        &self,
        ctx: RenderContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
    );
    fn handle_user_event(&mut self, e: KeyEvent) -> Vec<Action>;
    #[allow(unused)]
    fn apply_app_action(&mut self, actions: &[Action]) {}
    fn name(&self) -> &'static str;
}

#[derive(Clone, Debug)]
enum Action {
    #[allow(unused)]
    Nothing,
    ActivateComponent(&'static str),
    DeActivateComponent(&'static str),
    HideComponent(&'static str),
    ShowComponent(&'static str),
    ActivateUserInput(/* activate requester */ &'static str),
    HandleUserInput(/* activate requester */ &'static str, String),
    CancelUserInput,
}

impl Action {
    fn target(&self) -> Option<&'static str> {
        match self {
            Action::Nothing => None,
            Action::ActivateComponent(t) => Some(t),
            Action::DeActivateComponent(t) => Some(t),
            Action::HideComponent(t) => Some(t),
            Action::ShowComponent(t) => Some(t),
            Action::ActivateUserInput(_) => Some("user-input"),
            Action::CancelUserInput => Some("user-input"),
            Action::HandleUserInput(t, _) => Some(t),
        }
    }
}

pub(super) fn run(
    ctx: RenderContext,
    mut terminal: Terminal<CrosstermBackend<StdoutLock>>,
    debugger: Rc<Debugger<CuiHook>>,
    rx: Receiver<Event<KeyEvent>>,
) -> anyhow::Result<()> {
    let main_right_tabs: Box<dyn CuiComponent> = Box::new(tabs::Tabs::new(
        "main.right.tabs",
        "Code walker, bug stalker!",
        vec![
            TabVariant::new(
                "Debugee",
                tab_switch_action!("main.right.logs", "main.right.debugee"),
            ),
            TabVariant::new(
                "Logs",
                tab_switch_action!("main.right.debugee", "main.right.logs"),
            ),
        ],
    ));
    let main_right_debugee_view: Box<dyn CuiComponent> = Box::new(DebugeeView {});
    let main_right_logs: Box<dyn CuiComponent> = Box::new(MainLogs {});

    let main_right = ComplexComponent::new(
        "main.right",
        |rect| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(0)
                .constraints([Constraint::Length(3), Constraint::Min(2)].as_ref())
                .split(rect);

            HashMap::from([
                ("main.right.tabs", chunks[0]),
                ("main.right.debugee", chunks[1]),
                ("main.right.logs", chunks[1]),
            ])
        },
        vec![main_right_tabs, main_right_debugee_view, main_right_logs],
        vec!["main.right.tabs", "main.right.debugee"],
        vec!["main.right.tabs", "main.right.debugee"],
    );

    let main_left_tabs: Box<dyn CuiComponent> = Box::new(tabs::Tabs::new(
        "main.left.tabs",
        "DDD",
        vec![
            TabVariant::new(
                "Breakpoints",
                tab_switch_action!("main.left.variables", "main.left.breakpoints"),
            ),
            TabVariant::new(
                "Variables",
                tab_switch_action!("main.left.breakpoints", "main.left.variables"),
            ),
        ],
    ));
    let main_left_breakpoints: Box<dyn CuiComponent> =
        Box::new(main::breakpoint::Breakpoints::new(debugger.clone()));
    let main_left_variables: Box<dyn CuiComponent> = Box::new(Variables::new(debugger.clone()));

    let main_left = ComplexComponent::new(
        "main.left",
        |rect| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(0)
                .constraints([Constraint::Length(3), Constraint::Min(2)].as_ref())
                .split(rect);

            HashMap::from([
                ("main.left.tabs", chunks[0]),
                ("main.left.breakpoints", chunks[1]),
                ("main.left.variables", chunks[1]),
            ])
        },
        vec![main_left_tabs, main_left_breakpoints, main_left_variables],
        vec!["main.left.tabs", "main.left.breakpoints"],
        vec!["main.left.tabs", "main.left.breakpoints"],
    );

    let main_right: Box<dyn CuiComponent> = Box::new(main_right);
    let main_left: Box<dyn CuiComponent> = Box::new(main_left);

    let main_window = ComplexComponent::new(
        "main",
        |rect| {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(0)
                .constraints([Constraint::Percentage(25), Constraint::Percentage(75)].as_ref())
                .split(rect);
            HashMap::from([("main.left", chunks[0]), ("main.right", chunks[1])])
        },
        vec![main_left, main_right],
        vec!["main.left", "main.right"],
        vec!["main.left", "main.right"],
    );

    let user_input = UserInput::new();

    let mut app_window = ComplexComponent::new(
        "app",
        |rect| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Min(2),
                        Constraint::Length(3),
                        Constraint::Length(3),
                    ]
                    .as_ref(),
                )
                .split(rect);

            HashMap::from([
                ("main", chunks[0]),
                ("user-input", chunks[1]),
                ("context-help", chunks[2]),
            ])
        },
        vec![
            Box::new(main_window),
            Box::new(user_input),
            Box::new(ContextHelp {}),
        ],
        vec!["main"],
        vec!["main", "user-input", "context-help"],
    );

    loop {
        terminal.draw(|frame| {
            let rect = frame.size();
            app_window.render(ctx.clone(), frame, rect);
        })?;

        match rx.recv()? {
            Event::Input(e) => match e {
                KeyEvent {
                    code: KeyCode::Char('c'),
                    ..
                } => {
                    Continue::new(&debugger).run()?;
                }
                KeyEvent {
                    code: KeyCode::Char('q'),
                    ..
                } => {
                    disable_raw_mode()?;
                    crossterm::execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        DisableMouseCapture,
                    )?;
                    terminal.show_cursor()?;
                    return Ok(());
                }
                _ => {
                    let behaviour = app_window.handle_user_event(e);
                    app_window.apply_app_action(&behaviour);
                }
            },
            Event::Tick => {}
        }
    }
}
