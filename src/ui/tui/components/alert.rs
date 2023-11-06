use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::Msg;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::prelude::Style;
use ratatui::style::Color;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use tuirealm::command::{Cmd, CmdResult};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::{AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State};

#[derive(Default)]
pub struct Alert {
    props: Props,
}

impl MockComponent for Alert {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Alert!")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded);

        let alert_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Percentage(35),
                    Constraint::Percentage(30),
                    Constraint::Percentage(35),
                ]
                .as_ref(),
            )
            .split(area);

        let alert_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(
                [
                    Constraint::Percentage(20),
                    Constraint::Percentage(60),
                    Constraint::Percentage(20),
                ]
                .as_ref(),
            )
            .split(alert_layout[1])[1];

        let text = self
            .props
            .get_or(
                Attribute::Text,
                AttrValue::String("some error happens".to_string()),
            )
            .unwrap_string();

        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(Color::Black))
            .block(block)
            .alignment(Alignment::Center);

        //this clears out the background
        frame.render_widget(Clear, alert_layout);
        frame.render_widget(paragraph, alert_layout);
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value)
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        CmdResult::None
    }
}

impl Component<Msg, UserEvent> for Alert {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            })
            | Event::Keyboard(KeyEvent { code: Key::Esc, .. }) => Some(Msg::CloseAlert),
            _ => None,
        }
    }
}
