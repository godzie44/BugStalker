use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::Msg;
use tui_realm_stdlib::Radio;
use tuirealm::command::{Cmd, CmdResult, Direction};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Alignment, BorderSides, BorderType, Borders, Color, Layout};
use tuirealm::tui::layout::Rect;
use tuirealm::{AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State};

/// Tabs and related windows
pub struct TabWindow {
    props: Props,
    /// Tab choice
    choices: Radio,
    /// Active window
    active_idx: Option<usize>,
    /// Visible window
    visible_idx: usize,
    /// All windows
    windows: Vec<Box<dyn Component<Msg, UserEvent>>>,
}

impl TabWindow {
    pub fn new(
        title: &str,
        tabs: &[&str],
        windows: Vec<Box<dyn Component<Msg, UserEvent>>>,
    ) -> Self {
        debug_assert!(tabs.len() == windows.len());

        let choices = Radio::default()
            .borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(Color::LightGreen),
            )
            .foreground(Color::LightGreen)
            .title(title, Alignment::Center)
            .rewind(true)
            .choices(tabs);

        let tabs: Vec<_> = windows;

        let this = Self {
            active_idx: None,
            visible_idx: 0,
            choices,
            windows: tabs,
            props: Props::default(),
        };

        this.background(Color::Yellow)
            .foreground(Color::Yellow)
            .layout(
                Layout::default()
                    .direction(tuirealm::tui::layout::Direction::Vertical)
                    .constraints(
                        [
                            tuirealm::tui::layout::Constraint::Length(3),
                            tuirealm::tui::layout::Constraint::Min(3),
                        ]
                        .as_ref(),
                    ),
            )
    }
}

impl TabWindow {
    pub fn foreground(mut self, fg: Color) -> Self {
        self.attr(Attribute::Foreground, AttrValue::Color(fg));
        self
    }

    pub fn background(mut self, bg: Color) -> Self {
        self.attr(Attribute::Background, AttrValue::Color(bg));
        self
    }

    pub fn title<S: AsRef<str>>(mut self, t: S, a: Alignment) -> Self {
        self.attr(
            Attribute::Title,
            AttrValue::Title((t.as_ref().to_string(), a)),
        );
        self
    }

    pub fn layout(mut self, layout: Layout) -> Self {
        self.attr(Attribute::Layout, AttrValue::Layout(layout));
        self
    }

    pub fn windows(mut self, tabs: Vec<Box<dyn Component<Msg, UserEvent>>>) -> Self {
        self.windows = tabs;
        self
    }

    fn set_active_idx(&mut self, idx: usize) {
        self.attr(Attribute::Focus, AttrValue::Flag(false));
        self.active_idx = Some(idx);
        self.visible_idx = idx;
        if let Some(new_active_tab) = self.active_window_mut() {
            new_active_tab.attr(Attribute::Focus, AttrValue::Flag(true));
        }
    }

    fn deactivate_window(&mut self) {
        if let Some(old_active_tab) = self.active_window_mut() {
            old_active_tab.attr(Attribute::Focus, AttrValue::Flag(false));
        }
        self.active_idx = None;
    }

    fn active_window(&self) -> Option<&dyn Component<Msg, UserEvent>> {
        self.windows.get(self.active_idx?).map(|b| &**b)
    }

    fn active_window_mut(&mut self) -> Option<&mut (dyn Component<Msg, UserEvent> + 'static)> {
        self.windows
            .get_mut(self.active_idx?)
            .map(move |b| &mut **b)
    }

    fn visible_window(&self) -> Option<&dyn Component<Msg, UserEvent>> {
        self.windows.get(self.visible_idx).map(|b| &**b)
    }

    fn visible_window_mut(&mut self) -> Option<&mut (dyn Component<Msg, UserEvent> + 'static)> {
        self.windows
            .get_mut(self.visible_idx)
            .map(move |b| &mut **b)
    }
}

impl MockComponent for TabWindow {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let borders = self
                .props
                .get_or(
                    Attribute::Borders,
                    AttrValue::Borders(Borders::default().sides(BorderSides::NONE)),
                )
                .unwrap_borders();
            let focus = self
                .props
                .get_or(Attribute::Focus, AttrValue::Flag(false))
                .unwrap_flag();
            self.choices.attr(Attribute::Focus, AttrValue::Flag(focus));

            let title = self.props.get(Attribute::Title).map(|x| x.unwrap_title());
            let div = tui_realm_stdlib::utils::get_block(borders, title, focus, None);
            // Render block
            frame.render_widget(div, area);
            // Render children
            if let Some(layout) = self.props.get(Attribute::Layout).map(|x| x.unwrap_layout()) {
                let chunks = layout.chunks(area);
                debug_assert!(chunks.len() == 2);
                self.choices.view(frame, chunks[0]);
                if let Some(window) = self.visible_window_mut() {
                    window.view(frame, chunks[1]);
                }
            }
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if matches!(attr, Attribute::Focus) && value == AttrValue::Flag(false) {
            self.deactivate_window();
        }

        // all custom attributes redirect to tab windows
        if matches!(attr, Attribute::Custom(_)) {
            for comp in self.windows.iter_mut() {
                comp.attr(attr, value.clone());
            }
            return;
        }

        self.props.set(attr, value.clone());
    }

    fn state(&self) -> State {
        if let Some(window) = self.visible_window() {
            return window.state();
        }

        self.choices.state()
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        if self.active_window().is_some() {
            return CmdResult::None;
        }
        self.choices.perform(cmd)
    }
}

impl Component<Msg, UserEvent> for TabWindow {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Left, ..
            }) => self.perform(Cmd::Move(Direction::Left)),
            Event::Keyboard(KeyEvent {
                code: Key::Right, ..
            }) => self.perform(Cmd::Move(Direction::Right)),
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => {
                let res = self.perform(Cmd::Submit);
                if let CmdResult::Submit(state) = res {
                    let tab_idx = state.unwrap_one().unwrap_usize();
                    self.set_active_idx(tab_idx);
                    return Some(Msg::None);
                }

                CmdResult::None
            }
            Event::User(_) | Event::Tick => {
                // user events and tick send to all windows
                for (i, window) in self.windows.iter_mut().enumerate() {
                    if !self.active_idx.map(|idx| idx == i).unwrap_or_default() {
                        _ = window.on(ev.clone());
                    }
                }
                CmdResult::None
            }
            _ => CmdResult::None,
        };

        if let Some(window) = self.active_window_mut() {
            return window.on(ev);
        }
        Some(Msg::None)
    }
}
