use crate::oracle::Oracle;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::Msg;
use std::sync::Arc;
use tui_realm_stdlib::Radio;
use tuirealm::command::{Cmd, CmdResult, Direction};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Alignment, BorderType, Borders, Color, Layout};
use tuirealm::tui::layout::Rect;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
};

pub struct Oracles {
    props: Props,

    choices: Radio,
    active_idx: usize,
    tabs: Vec<Box<dyn Component<Msg, UserEvent>>>,
}

impl Oracles {
    pub fn new(oracles: &[Arc<dyn Oracle>]) -> Self {
        let ora_names: Vec<_> = oracles.iter().map(|oracle| oracle.name()).collect();

        let oracle_choices = Radio::default()
            .borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(Color::LightGreen),
            )
            .foreground(Color::LightGreen)
            .title("Choose your oracle", Alignment::Center)
            .rewind(true)
            .choices(&ora_names);

        let tabs: Vec<_> = oracles
            .iter()
            .map(|o| o.clone().make_tui_component())
            .collect();

        let this = Self {
            active_idx: 0,
            choices: oracle_choices,
            tabs,
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

impl Oracles {
    pub fn foreground(mut self, fg: Color) -> Self {
        self.attr(Attribute::Foreground, AttrValue::Color(fg));
        self
    }

    pub fn background(mut self, bg: Color) -> Self {
        self.attr(Attribute::Background, AttrValue::Color(bg));
        self
    }

    pub fn borders(mut self, b: Borders) -> Self {
        self.attr(Attribute::Borders, AttrValue::Borders(b));
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

    pub fn tabs(mut self, tabs: Vec<Box<dyn Component<Msg, UserEvent>>>) -> Self {
        self.tabs = tabs;
        self
    }

    fn active_tab(&mut self) -> &mut dyn Component<Msg, UserEvent> {
        &mut *self.tabs[self.active_idx]
    }
}

impl MockComponent for Oracles {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let borders = self
                .props
                .get_or(Attribute::Borders, AttrValue::Borders(Borders::default()))
                .unwrap_borders();
            let title = self.props.get(Attribute::Title).map(|x| x.unwrap_title());
            let div = tui_realm_stdlib::utils::get_block(borders, title, true, None);
            // Render block
            frame.render_widget(div, area);
            // Render children
            if let Some(layout) = self.props.get(Attribute::Layout).map(|x| x.unwrap_layout()) {
                let chunks = layout.chunks(area);
                debug_assert!(chunks.len() == 2);
                self.choices.view(frame, chunks[0]);
                self.active_tab().view(frame, chunks[1]);
            }
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value.clone());
    }

    fn state(&self) -> State {
        State::One(StateValue::Usize(self.active_idx))
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.choices.perform(cmd)
    }
}

impl Component<Msg, UserEvent> for Oracles {
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
                let CmdResult::Submit(state) = res else {
                    unreachable!()
                };

                let tab_idx = state.unwrap_one().unwrap_usize();
                self.active_idx = tab_idx;
                CmdResult::None
            }
            _ => CmdResult::None,
        };

        self.active_tab().on(ev)
    }
}
