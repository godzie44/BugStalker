use crate::ui;
use crate::ui::tui::Msg;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::{CommonAction, SpecialAction};
use crate::ui::tui::utils::flex_radio;
use strum_macros::FromRepr;
use tuirealm::command::{Cmd, CmdResult, Direction};
use tuirealm::props::{
    Alignment, BorderSides, BorderType, Borders, Color, Layout, PropPayload, PropValue,
};
use tuirealm::ratatui::layout::Rect;
use tuirealm::{AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, props};

#[derive(FromRepr, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum ViewSize {
    Default,
    Expand,
    Compacted,
}

impl From<ViewSize> for AttrValue {
    fn from(layout: ViewSize) -> Self {
        AttrValue::Payload(PropPayload::One(PropValue::U8(layout as u8)))
    }
}

impl From<AttrValue> for ViewSize {
    fn from(value: AttrValue) -> Self {
        ViewSize::from_repr(value.unwrap_payload().unwrap_one().unwrap_u8())
            .expect("invalid attribute value")
    }
}

/// Tabs and related windows
pub struct TabWindow {
    props: Props,
    /// Tab choice
    choices: flex_radio::Radio,
    /// Active window
    active_idx: Option<usize>,
    /// Visible window
    visible_idx: usize,
    /// All windows
    windows: Vec<Box<dyn Component<Msg, UserEvent>>>,
    /// Window size in relation to others
    view_size: ViewSize,
    /// If specified - disable default rewinding and
    /// returning a special message instead of rewind
    on_rewind: Option<fn(Direction) -> Msg>,
}

impl TabWindow {
    pub fn new(
        title: &str,
        tabs: &[&str],
        windows: Vec<Box<dyn Component<Msg, UserEvent>>>,
        msg_on_rewind: Option<fn(Direction) -> Msg>,
    ) -> Self {
        debug_assert!(tabs.len() == windows.len());

        let choices = flex_radio::Radio::default()
            .borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(Color::LightGreen),
            )
            .foreground(Color::LightGreen)
            .title(title, Alignment::Center)
            .rewind(msg_on_rewind.is_none())
            .choices(tabs);

        let tabs: Vec<_> = windows;

        let this = Self {
            active_idx: None,
            visible_idx: 0,
            choices,
            windows: tabs,
            props: Props::default(),
            view_size: ViewSize::Default,
            on_rewind: msg_on_rewind,
        };

        this.background(Color::Yellow)
            .foreground(Color::Yellow)
            .layout(
                Layout::default()
                    .direction(tuirealm::ratatui::layout::Direction::Vertical)
                    .constraints(
                        [
                            tuirealm::ratatui::layout::Constraint::Length(3),
                            tuirealm::ratatui::layout::Constraint::Min(3),
                        ]
                        .as_ref(),
                    ),
            )
    }
}

impl TabWindow {
    pub const VIEW_SIZE_ATTR: Attribute = Attribute::Custom("VIEW_SIZE");
    pub const RESET_CHOICE_ATTR: Attribute = Attribute::Custom("RESET_CHOICE");
    pub const ACTIVATE_TAB: Attribute = Attribute::Custom("ACTIVATE_TAB");

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
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) != AttrValue::Flag(true) {
            return;
        }

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
        let div = tui_realm_stdlib::utils::get_block(borders, title.as_ref(), focus, None);
        // Render block
        frame.render_widget(div, area);
        // Render children
        if let Some(layout) = self.props.get(Attribute::Layout).map(|x| x.unwrap_layout()) {
            if self.view_size == ViewSize::Compacted {
                self.choices.view(frame, area);
            } else {
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

        match attr {
            Self::VIEW_SIZE_ATTR => {
                self.view_size = crate::ui::tui::utils::tab::ViewSize::from(value);
                match self.view_size {
                    ViewSize::Default | ViewSize::Expand => {
                        self.choices.states.set_horizontal_render()
                    }
                    ViewSize::Compacted => self.choices.states.set_vertical_layout(),
                }
            }
            Self::RESET_CHOICE_ATTR => {
                debug_assert!(matches!(
                    value,
                    AttrValue::Direction(props::Direction::Left)
                        | AttrValue::Direction(props::Direction::Right)
                ));

                match value {
                    AttrValue::Direction(props::Direction::Left) => {
                        self.choices.states.select(0);
                    }
                    AttrValue::Direction(props::Direction::Right) => {
                        self.choices
                            .states
                            .select(self.choices.states.choices.len() - 1);
                    }
                    _ => {}
                }
            }
            Self::ACTIVATE_TAB => {
                let res = self.perform(Cmd::Submit);
                if let CmdResult::Submit(state) = res {
                    let tab_idx = state.unwrap_one().unwrap_usize();
                    self.set_active_idx(tab_idx);
                }
            }
            Attribute::Custom(_) => {
                // all other custom attributes redirect to tab windows
                for comp in self.windows.iter_mut() {
                    comp.attr(attr, value.clone());
                }
            }
            _ => {
                self.props.set(attr, value.clone());
            }
        }
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
        let cmd_res = match ev {
            Event::Keyboard(key_event) => {
                let keymap = &ui::config::current().tui_keymap;
                if let Some(action) = keymap.get_common(&key_event) {
                    match action {
                        CommonAction::Left => self.perform(Cmd::Move(Direction::Left)),
                        CommonAction::Right => self.perform(Cmd::Move(Direction::Right)),
                        CommonAction::Submit => {
                            let res = self.perform(Cmd::Submit);
                            if let CmdResult::Submit(state) = res {
                                let tab_idx = state.unwrap_one().unwrap_usize();
                                self.set_active_idx(tab_idx);
                                return Some(Msg::None);
                            }

                            CmdResult::None
                        }
                        _ => CmdResult::None,
                    }
                } else if let Some(SpecialAction::SwitchWindowTab) = keymap.get_special(&key_event)
                {
                    self.deactivate_window();
                    self.attr(Attribute::Focus, AttrValue::Flag(true));
                    self.perform(Cmd::Move(Direction::Right))
                } else {
                    CmdResult::None
                }
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

        if let CmdResult::Invalid(Cmd::Move(dir)) = cmd_res {
            return self.on_rewind.map(|cb| cb(dir));
        }

        if let Some(window) = self.active_window_mut() {
            return window.on(ev);
        }
        Some(Msg::None)
    }
}
