use tuirealm::command::{Cmd, CmdResult, Direction};
use tuirealm::props::{
    Alignment, AttrValue, Attribute, Borders, Color, PropPayload, PropValue, Props, Style,
    TextModifiers,
};
use tuirealm::ratatui::text::Line as Spans;
use tuirealm::ratatui::widgets::{ListDirection, ListItem, ListState};
use tuirealm::ratatui::{layout::Rect, widgets::Tabs};
use tuirealm::{Frame, MockComponent, State, StateValue};

#[derive(PartialEq, Clone, Copy, Default)]
enum LayoutType {
    Vertical,
    #[default]
    Horizontal,
}

#[derive(Default)]
pub struct RadioStates {
    pub choice: usize,
    pub choices: Vec<String>,
    layout: LayoutType,
}

impl RadioStates {
    /// Move choice index to next choice.
    /// If rewind disabled and the next choice is (last choice + 1) - no moving occurred.
    /// Return false if no moving occurred, true otherwise.
    pub fn next_choice(&mut self, rewind: bool) -> bool {
        if rewind && self.choice + 1 >= self.choices.len() {
            self.choice = 0;
            true
        } else if self.choice + 1 >= self.choices.len() {
            false
        } else {
            self.choice += 1;
            true
        }
    }

    /// Move choice index to previous choice.
    /// If rewind disabled and the next choice is (first choice - 1) - no moving occurred.
    /// Return false if no moving occurred, true otherwise.
    pub fn prev_choice(&mut self, rewind: bool) -> bool {
        if rewind && self.choice == 0 && !self.choices.is_empty() {
            self.choice = self.choices.len() - 1;
            true
        } else if self.choice == 0 && !self.choices.is_empty() {
            false
        } else {
            self.choice -= 1;
            true
        }
    }

    pub fn set_choices(&mut self, spans: &[String]) {
        self.choices = spans.to_vec();
        // Keep index if possible
        if self.choice >= self.choices.len() {
            self.choice = match self.choices.len() {
                0 => 0,
                l => l - 1,
            };
        }
    }

    pub fn select(&mut self, i: usize) {
        if i < self.choices.len() {
            self.choice = i;
        }
    }

    pub fn set_vertical_layout(&mut self) {
        self.layout = LayoutType::Vertical
    }

    pub fn set_horizontal_render(&mut self) {
        self.layout = LayoutType::Horizontal
    }
}

/// Radio component represents a group of tabs to select from
///
/// Like [`tui_realm_stdlib::Radio`] but with two available layout types:
/// horizontal (default) and vertical.
#[derive(Default)]
pub struct Radio {
    props: Props,
    pub states: RadioStates,
}

impl Radio {
    pub fn foreground(mut self, fg: Color) -> Self {
        self.attr(Attribute::Foreground, AttrValue::Color(fg));
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

    pub fn choices<S: AsRef<str>>(mut self, choices: &[S]) -> Self {
        self.attr(
            Attribute::Content,
            AttrValue::Payload(PropPayload::Vec(
                choices
                    .iter()
                    .map(|x| PropValue::Str(x.as_ref().to_string()))
                    .collect(),
            )),
        );
        self
    }

    pub fn rewind(mut self, r: bool) -> Self {
        self.attr(Attribute::Rewind, AttrValue::Flag(r));
        self
    }

    fn is_rewind(&self) -> bool {
        self.props
            .get_or(Attribute::Rewind, AttrValue::Flag(false))
            .unwrap_flag()
    }
}

impl MockComponent for Radio {
    fn view(&mut self, render: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let foreground = self
                .props
                .get_or(Attribute::Foreground, AttrValue::Color(Color::Reset))
                .unwrap_color();
            let background = self
                .props
                .get_or(Attribute::Background, AttrValue::Color(Color::Reset))
                .unwrap_color();
            let borders = self
                .props
                .get_or(Attribute::Borders, AttrValue::Borders(Borders::default()))
                .unwrap_borders();
            let title = self.props.get(Attribute::Title).map(|x| x.unwrap_title());
            let focus = self
                .props
                .get_or(Attribute::Focus, AttrValue::Flag(false))
                .unwrap_flag();
            let inactive_style = self
                .props
                .get(Attribute::FocusStyle)
                .map(|x| x.unwrap_style());
            let div = tui_realm_stdlib::utils::get_block(borders, title, focus, inactive_style);

            let (fg, block_color): (Color, Color) = match focus {
                true => (foreground, foreground),
                false => (foreground, Color::Reset),
            };
            let modifiers = match focus {
                true => TextModifiers::REVERSED,
                false => TextModifiers::empty(),
            };

            match self.states.layout {
                LayoutType::Vertical => {
                    let list_items: Vec<_> = self
                        .states
                        .choices
                        .iter()
                        .map(|s| ListItem::new(s.to_string()))
                        .collect();
                    let radio = tuirealm::ratatui::widgets::List::new(list_items)
                        .block(div)
                        .highlight_style(Style::default().fg(fg).add_modifier(modifiers))
                        .direction(ListDirection::TopToBottom);

                    let mut state: ListState = ListState::default();
                    state.select(Some(self.states.choice));
                    render.render_stateful_widget(radio, area, &mut state);
                }
                LayoutType::Horizontal => {
                    let choices: Vec<Spans> = self
                        .states
                        .choices
                        .iter()
                        .map(|x| Spans::from(x.clone()))
                        .collect();

                    let radio: Tabs = Tabs::new(choices)
                        .block(div)
                        .select(self.states.choice)
                        .style(Style::default().fg(block_color).bg(background))
                        .highlight_style(Style::default().fg(fg).add_modifier(modifiers));
                    render.render_widget(radio, area);
                }
            }
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        match attr {
            Attribute::Content => {
                let choices: Vec<String> = value
                    .unwrap_payload()
                    .unwrap_vec()
                    .iter()
                    .map(|x| x.clone().unwrap_str())
                    .collect();
                self.states.set_choices(&choices);
            }
            Attribute::Value => {
                self.states
                    .select(value.unwrap_payload().unwrap_one().unwrap_usize());
            }
            attr => {
                self.props.set(attr, value);
            }
        }
    }

    fn state(&self) -> State {
        State::One(StateValue::Usize(self.states.choice))
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Right) => {
                if self.states.next_choice(self.is_rewind()) {
                    CmdResult::Changed(self.state())
                } else {
                    // if no moving occurred, return [`CmdResult::Invalid`]
                    // with the last direction where the choice went
                    CmdResult::Invalid(Cmd::Move(Direction::Right))
                }
            }
            Cmd::Move(Direction::Left) => {
                if self.states.prev_choice(self.is_rewind()) {
                    CmdResult::Changed(self.state())
                } else {
                    // if no moving occurred, return [`CmdResult::Invalid`]
                    // with the last direction where the choice went
                    CmdResult::Invalid(Cmd::Move(Direction::Left))
                }
            }
            Cmd::Submit => CmdResult::Submit(self.state()),
            _ => CmdResult::None,
        }
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use tuirealm::props::{PropPayload, PropValue};

    #[test]
    fn test_components_radio_states() {
        let mut states: RadioStates = RadioStates::default();
        assert_eq!(states.choice, 0);
        assert_eq!(states.choices.len(), 0);
        let choices: &[String] = &[
            "lemon".to_string(),
            "strawberry".to_string(),
            "vanilla".to_string(),
            "chocolate".to_string(),
        ];
        states.set_choices(choices);
        assert_eq!(states.choice, 0);
        assert_eq!(states.choices.len(), 4);
        // Move
        states.prev_choice(false);
        assert_eq!(states.choice, 0);
        states.next_choice(false);
        assert_eq!(states.choice, 1);
        states.next_choice(false);
        assert_eq!(states.choice, 2);
        // Forward overflow
        states.next_choice(false);
        states.next_choice(false);
        assert_eq!(states.choice, 3);
        states.prev_choice(false);
        assert_eq!(states.choice, 2);
        // Update
        let choices: &[String] = &["lemon".to_string(), "strawberry".to_string()];
        states.set_choices(choices);
        assert_eq!(states.choice, 1); // Move to first index available
        assert_eq!(states.choices.len(), 2);
        let choices: &[String] = &[];
        states.set_choices(choices);
        assert_eq!(states.choice, 0); // Move to first index available
        assert_eq!(states.choices.len(), 0);
        // Rewind
        let choices: &[String] = &[
            "lemon".to_string(),
            "strawberry".to_string(),
            "vanilla".to_string(),
            "chocolate".to_string(),
        ];
        states.set_choices(choices);
        assert_eq!(states.choice, 0);
        states.prev_choice(true);
        assert_eq!(states.choice, 3);
        states.next_choice(true);
        assert_eq!(states.choice, 0);
        states.next_choice(true);
        assert_eq!(states.choice, 1);
        states.prev_choice(true);
        assert_eq!(states.choice, 0);
    }

    #[test]
    fn test_components_radio() {
        // Make component
        let mut component = Radio::default()
            .foreground(Color::Red)
            .borders(Borders::default())
            .title("C'est oui ou bien c'est non?", Alignment::Center)
            .choices(&["Oui!", "Non", "Peut-être"])
            .rewind(false);

        assert_eq!(component.states.choice, 0);
        assert_eq!(component.states.choices.len(), 3);
        component.attr(
            Attribute::Value,
            AttrValue::Payload(PropPayload::One(PropValue::Usize(2))),
        );
        assert_eq!(component.state(), State::One(StateValue::Usize(2)));
        component.states.choice = 1;
        assert_eq!(component.state(), State::One(StateValue::Usize(1)));
        assert_eq!(
            component.perform(Cmd::Move(Direction::Left)),
            CmdResult::Changed(State::One(StateValue::Usize(0))),
        );
        assert_eq!(component.state(), State::One(StateValue::Usize(0)));
        assert_eq!(
            component.perform(Cmd::Move(Direction::Left)),
            CmdResult::Invalid(Cmd::Move(Direction::Left)),
        );
        assert_eq!(component.state(), State::One(StateValue::Usize(0)));
        assert_eq!(
            component.perform(Cmd::Move(Direction::Right)),
            CmdResult::Changed(State::One(StateValue::Usize(1))),
        );
        assert_eq!(component.state(), State::One(StateValue::Usize(1)));
        assert_eq!(
            component.perform(Cmd::Move(Direction::Right)),
            CmdResult::Changed(State::One(StateValue::Usize(2))),
        );
        assert_eq!(component.state(), State::One(StateValue::Usize(2)));
        assert_eq!(
            component.perform(Cmd::Move(Direction::Right)),
            CmdResult::Invalid(Cmd::Move(Direction::Right)),
        );
        assert_eq!(component.state(), State::One(StateValue::Usize(2)));
        assert_eq!(
            component.perform(Cmd::Submit),
            CmdResult::Submit(State::One(StateValue::Usize(2))),
        );
    }
}
