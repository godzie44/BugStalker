use crate::ui;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::CommonAction;
use crate::ui::tui::{ConfirmedAction, Msg};
use std::str::FromStr;
use tui_realm_stdlib::Radio;
use tuirealm::command::{Cmd, CmdResult};
use tuirealm::props::{BorderSides, Borders, PropPayload, PropValue};
use tuirealm::ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tuirealm::ratatui::prelude::Style;
use tuirealm::ratatui::style::{Color, Stylize};
use tuirealm::ratatui::widgets;
use tuirealm::ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, command,
};

#[derive(PartialEq)]
enum OpMode {
    Ok,
    YesNo,
}

pub struct Popup {
    props: Props,
    buttons: Radio,
    mode: OpMode,
}

impl Default for Popup {
    fn default() -> Self {
        let buttons = Radio::default()
            .borders(Borders::default().sides(BorderSides::NONE))
            .foreground(Color::LightGreen)
            .background(Color::Black)
            .rewind(false);

        Self {
            props: Props::default(),
            buttons,
            mode: OpMode::Ok,
        }
    }
}

pub struct YesNoLabels<T: ToString> {
    yes: T,
    no: T,
}

impl<T: ToString> YesNoLabels<T> {
    pub fn new(yes: T, no: T) -> Self {
        Self { yes, no }
    }
}

impl Default for YesNoLabels<String> {
    fn default() -> Self {
        Self {
            yes: "yes".to_string(),
            no: "no".to_string(),
        }
    }
}

impl Popup {
    pub fn ok_attrs() -> (Attribute, AttrValue) {
        (
            Attribute::Content,
            AttrValue::Payload(PropPayload::Vec(vec![PropValue::Str("OK".to_string())])),
        )
    }

    pub fn yes_no_attrs<T: ToString>(labels: YesNoLabels<T>) -> (Attribute, AttrValue) {
        (
            Attribute::Content,
            AttrValue::Payload(PropPayload::Vec(vec![
                PropValue::Str(labels.yes.to_string()),
                PropValue::Str(labels.no.to_string()),
            ])),
        )
    }
}

impl MockComponent for Popup {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let mut block = Block::default()
            .borders(widgets::Borders::TOP | widgets::Borders::RIGHT | widgets::Borders::LEFT)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::LightGreen));
        if let Some(title) = self.query(Attribute::Title) {
            if self.mode == OpMode::Ok {
                block = block.title(title.unwrap_string());
            }
        }

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
            .split(
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(
                        [
                            Constraint::Percentage(35),
                            Constraint::Percentage(30),
                            Constraint::Percentage(35),
                        ]
                        .as_ref(),
                    )
                    .split(area)[1],
            )[1];

        let alert_layout_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
            .split(alert_layout);

        let text_layout = alert_layout_split[0];
        let buttons_layout = alert_layout_split[1];

        let text = self
            .props
            .get_or(Attribute::Text, AttrValue::String("???".to_string()))
            .unwrap_string();

        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(Color::Black))
            .block(block)
            .alignment(Alignment::Center);

        let buttons_block = Block::default()
            .borders(widgets::Borders::BOTTOM | widgets::Borders::RIGHT | widgets::Borders::LEFT)
            .bg(Color::Black)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::LightGreen));

        let rb_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(90), Constraint::Percentage(10)].as_ref())
            .split(buttons_layout)[1];

        //this clears out the background
        frame.render_widget(Clear, alert_layout);
        frame.render_widget(paragraph, text_layout);
        frame.render_widget(buttons_block, buttons_layout);
        self.buttons.view(frame, rb_layout);
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if attr == Attribute::Focus {
            self.buttons.attr(attr, value.clone());
        }

        if attr == Attribute::Content {
            if value == Self::ok_attrs().1 {
                self.mode = OpMode::Ok;
            } else {
                self.mode = OpMode::YesNo;
            }

            return self.buttons.attr(attr, value);
        }

        self.props.set(attr, value)
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        CmdResult::None
    }
}

impl Component<Msg, UserEvent> for Popup {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(key_event) => {
                let keymap = &ui::config::current().tui_keymap;
                if let Some(action) = keymap.get_common(&key_event) {
                    match action {
                        CommonAction::Left => {
                            self.buttons.perform(Cmd::Move(command::Direction::Left));
                            Some(Msg::None)
                        }
                        CommonAction::Right => {
                            self.buttons.perform(Cmd::Move(command::Direction::Right));
                            Some(Msg::None)
                        }
                        CommonAction::Submit => {
                            let res = self.buttons.perform(Cmd::Submit);

                            match res {
                                CmdResult::Submit(state) => match self.mode {
                                    OpMode::Ok => Some(Msg::PopupOk),
                                    OpMode::YesNo => {
                                        let action = self
                                            .query(Attribute::Custom("action"))
                                            .expect("infallible")
                                            .unwrap_string();
                                        let action =
                                            ConfirmedAction::from_str(&action).expect("infallible");
                                        let state = state.unwrap_one().unwrap_usize();
                                        if state == 0 {
                                            Some(Msg::PopupYes(action))
                                        } else {
                                            Some(Msg::PopupNo(action))
                                        }
                                    }
                                },
                                _ => Some(Msg::PopupOk),
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
