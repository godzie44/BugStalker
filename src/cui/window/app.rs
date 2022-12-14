use crate::cui::hook::CuiHook;
use crate::cui::window::alert::Alert;
use crate::cui::window::app::AppMode::Default as DefaultMode;
use crate::cui::window::help::ContextHelp;
use crate::cui::window::input::UserInput;
use crate::cui::window::main::{DebugeeOut, DebugeeView, Logs};
use crate::cui::window::message::{ActionMessage, Exchanger};
use crate::cui::window::tabs::{TabVariant, Tabs};
use crate::cui::window::{main, tabs, CuiComponent, RenderOpts};
use crate::cui::{context, AppState, DebugeeStreamBuffer};
use crate::debugger::Debugger;
use crate::fire;
use crossterm::event::KeyEvent;
use std::collections::HashMap;
use std::default::Default;
use std::io::StdoutLock;
use std::rc::Rc;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::Frame;

struct WindowDeck {
    name: &'static str,
    visible_window: &'static str,
    in_focus_window: Option<&'static str>,
    tabs: Tabs,
    windows: HashMap<&'static str, Box<dyn CuiComponent>>,
}

impl WindowDeck {
    fn new(
        name: &'static str,
        windows: Vec<Box<dyn CuiComponent>>,
        state_asserts: HashMap<&'static str, AppState>,
    ) -> Self {
        let tab_variants = windows
            .iter()
            .map(|component| {
                let c_name = component.name();

                if let Some(state) = state_asserts.get(c_name) {
                    TabVariant::contextual(
                        c_name,
                        ActionMessage::ActivateComponent { activate: c_name },
                        *state,
                        name,
                    )
                } else {
                    TabVariant::new(
                        c_name,
                        ActionMessage::ActivateComponent { activate: c_name },
                        name,
                    )
                }
            })
            .collect::<Vec<_>>();

        let tabs = tabs::Tabs::new("deck_tabs", "", tab_variants);

        Self {
            name,
            visible_window: windows[0].name(),
            in_focus_window: None,
            tabs,
            windows: windows.into_iter().map(|c| (c.name(), c)).collect(),
        }
    }

    fn drop_focus(&mut self) {
        self.in_focus_window = None
    }
}

impl CuiComponent for WindowDeck {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        mut opts: RenderOpts,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(3), Constraint::Min(2)])
            .split(rect);

        if self.in_focus_window.is_some() {
            opts.in_focus = true;
        }

        self.tabs.render(frame, chunks[0], opts);
        self.windows[self.visible_window].render(frame, chunks[1], opts);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        self.tabs.handle_user_event(e);
        if let Some(in_focus_component) = self.in_focus_window {
            self.windows
                .get_mut(in_focus_component)
                .unwrap()
                .handle_user_event(e);
        }
    }

    fn update(&mut self) -> anyhow::Result<()> {
        for msg in Exchanger::current().pop(self.name) {
            match msg {
                ActionMessage::ActivateComponent { activate } => {
                    self.visible_window = activate;
                    fire!(ActionMessage::FocusOnComponent {focus_on: activate} => "app_window");
                }
                ActionMessage::FocusOnComponent { focus_on } => {
                    if self.windows.get(focus_on).is_some() {
                        self.in_focus_window = Some(focus_on);
                    }
                }
                _ => {}
            }
        }

        self.windows.iter_mut().try_for_each(|(_, w)| w.update())
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppMode {
    Default,
    UserInput,
}

pub(super) struct AppWindow {
    left_deck: WindowDeck,
    right_deck: WindowDeck,
    user_input: UserInput,
    context_help: ContextHelp,
    alert: Alert,
    mode: AppMode,
}

impl AppWindow {
    pub fn new(debugger: Rc<Debugger<CuiHook>>, stream_buff: DebugeeStreamBuffer) -> Self {
        let breakpoints: Box<dyn CuiComponent> =
            Box::new(main::breakpoint::Breakpoints::new(debugger.clone()));
        let variables: Box<dyn CuiComponent> = Box::new(main::variable::Variables::new(debugger));
        let debugee_view: Box<dyn CuiComponent> = Box::new(DebugeeView::new());
        let logs: Box<dyn CuiComponent> = Box::new(Logs {});
        let debugee_out: Box<dyn CuiComponent> = Box::new(DebugeeOut::new(stream_buff));

        let left_deck_states = HashMap::from([(variables.name(), AppState::DebugeeBreak)]);
        Self {
            left_deck: WindowDeck::new("left_deck", vec![breakpoints, variables], left_deck_states),
            right_deck: WindowDeck::new(
                "right_deck",
                vec![debugee_view, debugee_out, logs],
                HashMap::default(),
            ),
            context_help: ContextHelp {},
            alert: Alert::default(),
            user_input: UserInput::new(),
            mode: DefaultMode,
        }
    }

    fn render_work_windows(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(0)
            .constraints([Constraint::Percentage(25), Constraint::Percentage(75)].as_ref())
            .split(rect);

        self.left_deck.render(frame, chunks[0], opts);
        self.right_deck.render(frame, chunks[1], opts);
    }
}

impl CuiComponent for AppWindow {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    ) {
        let mut constrains = vec![Constraint::Min(2), Constraint::Length(3)];
        if self.mode == AppMode::UserInput {
            constrains.push(Constraint::Length(3));
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints(constrains.as_ref())
            .split(rect);

        self.render_work_windows(frame, chunks[0], opts);

        if self.mode == AppMode::UserInput {
            self.user_input.render(frame, chunks[1], opts);
            self.context_help.render(frame, chunks[2], opts);
        } else {
            self.context_help.render(frame, chunks[1], opts);
        }

        self.alert.render(frame, rect, opts);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        match self.mode {
            AppMode::UserInput => {
                self.user_input.handle_user_event(e);
            }
            AppMode::Default => {
                self.alert.handle_user_event(e);
                self.left_deck.handle_user_event(e);
                self.right_deck.handle_user_event(e);
            }
        }
    }

    fn update(&mut self) -> anyhow::Result<()> {
        Exchanger::current()
            .pop(self.name())
            .into_iter()
            .for_each(|act| match act {
                ActionMessage::FocusOnComponent { focus_on } => {
                    self.right_deck.drop_focus();
                    self.left_deck.drop_focus();
                    fire!(ActionMessage::FocusOnComponent {focus_on} => self.left_deck.name());
                    fire!(ActionMessage::FocusOnComponent {focus_on} => self.right_deck.name());
                }
                ActionMessage::ActivateUserInput { sender } => {
                    self.mode = AppMode::UserInput;
                    fire!(ActionMessage::ActivateUserInput {sender} => self.user_input.name());
                }
                ActionMessage::CancelUserInput { .. } => self.mode = AppMode::Default,
                _ => {}
            });

        self.left_deck.update()?;
        self.right_deck.update()?;
        self.user_input.update()?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "app_window"
    }
}
