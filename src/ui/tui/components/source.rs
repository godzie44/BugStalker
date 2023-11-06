use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::{Id, Msg};
use ratatui::layout::Alignment;
use ratatui::prelude::Color;
use ratatui::widgets::BorderType;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};
use tui_realm_stdlib::Textarea;
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Borders, PropPayload, PropValue, TextSpan};
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

#[derive(Default)]
struct FileLinesCache {
    files: HashMap<PathBuf, Vec<String>>,
}

impl FileLinesCache {
    fn lines(&mut self, file: &Path) -> &Vec<String> {
        match self.files.entry(file.to_path_buf()) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                let lines = fs::File::open(file)
                    .map(|file| {
                        io::BufReader::new(file)
                            .lines()
                            .enumerate()
                            .filter_map(|(num, line)| {
                                line.map(|l| format!("{ln} {l}", ln = num + 1)).ok()
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|_| vec!["Failed to open file".to_string()]);

                v.insert(lines)
            }
        }
    }
}

#[derive(MockComponent)]
pub struct Source {
    component: Textarea,
    file_cache: FileLinesCache,
}

impl Source {
    fn get_title(mb_file: Option<&Path>) -> String {
        if let Some(file) = mb_file {
            format!("Program source code ({:?})", file)
        } else {
            "Program source code".into()
        }
    }

    pub fn new(exchanger: Arc<ClientExchanger>) -> anyhow::Result<Self> {
        let mb_threads = exchanger.request_sync(|dbg| dbg.thread_state()).ok();
        let mb_place_in_focus = mb_threads.and_then(|threads| {
            threads
                .into_iter()
                .find_map(|snap| if snap.in_focus { snap.place } else { None })
        });

        let cache = FileLinesCache::default();
        let component = Textarea::default()
            .borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(Color::LightBlue),
            )
            .foreground(Color::LightBlue)
            .title("Program source code", Alignment::Center)
            .step(4)
            .highlighted_str("▶");

        let mut this = Self {
            file_cache: cache,
            component,
        };

        if let Some(place) = mb_place_in_focus {
            this.update_source_view(place.file.as_path(), Some(place.line_number));
        }

        Ok(this)
    }

    fn update_source_view(&mut self, file: &Path, mb_line_num: Option<u64>) {
        self.component.attr(
            Attribute::Title,
            AttrValue::Title((Self::get_title(Some(file)), Alignment::Center)),
        );

        let lines = self
            .file_cache
            .lines(file)
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if Some((i + 1) as u64) == mb_line_num {
                    TextSpan::new(line).bg(Color::LightRed)
                } else {
                    TextSpan::new(line)
                }
            })
            .collect::<Vec<_>>();

        self.component.attr(
            Attribute::Text,
            AttrValue::Payload(PropPayload::Vec(
                lines.into_iter().map(PropValue::TextSpan).collect(),
            )),
        );

        if let Some(line) = mb_line_num {
            self.component.states.list_index = (line as usize).saturating_sub(1);
        }
    }

    pub fn subscriptions() -> Vec<Sub<Id, UserEvent>> {
        vec![
            Sub::new(
                SubEventClause::User(UserEvent::Breakpoint {
                    pc: Default::default(),
                    num: 0,
                    file: None,
                    line: None,
                    function: None,
                }),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::User(UserEvent::Step {
                    pc: Default::default(),
                    file: None,
                    line: None,
                    function: None,
                }),
                SubClause::Always,
            ),
        ]
    }
}

impl Component<Msg, UserEvent> for Source {
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
            Event::User(UserEvent::Breakpoint { file, line, .. })
            | Event::User(UserEvent::Step { file, line, .. }) => {
                if let Some(file) = file {
                    self.update_source_view(PathBuf::from(file).as_path(), line);
                }
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
