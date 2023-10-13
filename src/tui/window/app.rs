use super::specialized::logs::Logs;
use crate::debugger::Debugger;
use crate::fire;
use crate::tui::window::app::AppMode::Default as DefaultMode;
use crate::tui::window::general::alert::Alert;
use crate::tui::window::general::deck::WindowDeck;
use crate::tui::window::general::help::ContextHelp;
use crate::tui::window::general::input::UserInput;
use crate::tui::window::message::{ActionMessage, Exchanger};
use crate::tui::window::specialized::breakpoint::Breakpoints;
use crate::tui::window::specialized::debugee_out::DebugeeOut;
use crate::tui::window::specialized::debugee_view::DebugeeView;
use crate::tui::window::specialized::trace::ThreadTrace;
use crate::tui::window::specialized::variable::Variables;
use crate::tui::window::{RenderOpts, TuiComponent};
use crate::tui::{AppState, DebugeeStreamBuffer};
use crossterm::event::KeyEvent;
use std::collections::HashMap;
use std::default::Default;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::Frame;

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
    pub fn new(stream_buff: DebugeeStreamBuffer) -> Self {
        let breakpoints: Box<dyn TuiComponent> = Box::new(Breakpoints::new());
        let variables: Box<dyn TuiComponent> = Box::new(Variables::new());
        let threads: Box<dyn TuiComponent> = Box::new(ThreadTrace::new());
        let debugee_view: Box<dyn TuiComponent> = Box::new(DebugeeView::new());
        let logs: Box<dyn TuiComponent> = Box::<Logs>::default();
        let debugee_out: Box<dyn TuiComponent> = Box::new(DebugeeOut::new(stream_buff));

        let left_deck_states = HashMap::from([
            (variables.name(), AppState::DebugeeBreak),
            (threads.name(), AppState::DebugeeBreak),
        ]);

        Self {
            left_deck: WindowDeck::new(
                "left_deck",
                vec![breakpoints, threads, variables],
                left_deck_states,
            ),
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
        debugger: &mut Debugger,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(0)
            .constraints([Constraint::Percentage(25), Constraint::Percentage(75)].as_ref())
            .split(rect);

        self.left_deck.render(frame, chunks[0], opts, debugger);
        self.right_deck.render(frame, chunks[1], opts, debugger);
    }
}

impl TuiComponent for AppWindow {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
        debugger: &mut Debugger,
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

        self.render_work_windows(frame, chunks[0], opts, debugger);

        if self.mode == AppMode::UserInput {
            self.user_input.render(frame, chunks[1], opts, debugger);
            self.context_help.render(frame, chunks[2], opts, debugger);
        } else {
            self.context_help.render(frame, chunks[1], opts, debugger);
        }

        self.alert.render(frame, rect, opts, debugger);
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

    fn update(&mut self, debugger: &mut Debugger) -> anyhow::Result<()> {
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

        self.left_deck.update(debugger)?;
        self.right_deck.update(debugger)?;
        self.user_input.update(debugger)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "app_window"
    }
}
