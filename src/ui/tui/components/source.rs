use crate::ui::short::Abbreviator;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::utils::mstextarea::MultiSpanTextarea;
use crate::ui::tui::utils::syntect::into_text_span;
use crate::ui::tui::{Id, Msg};
use log::warn;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Borders, Style, TextSpan};
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::prelude::Color;
use tuirealm::tui::widgets::BorderType;
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

#[derive(Default)]
struct FileLinesCache {
    files: HashMap<PathBuf, Vec<Vec<TextSpan>>>,
    empty_file: Vec<Vec<TextSpan>>,
}

impl FileLinesCache {
    fn lines(&mut self, file: &Path) -> &Vec<Vec<TextSpan>> {
        match self.files.entry(file.to_path_buf()) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                let mut file = match fs::File::open(file) {
                    Ok(f) => f,
                    Err(e) => {
                        warn!("error while open {file:?}: {e}");
                        return &self.empty_file;
                    }
                };

                let mut source_code = String::new();
                file.read_to_string(&mut source_code).unwrap();

                let ps = SyntaxSet::load_defaults_newlines();
                let ts = ThemeSet::load_defaults();
                let syntax = ps.find_syntax_by_extension("rs").unwrap();
                let mut h = HighlightLines::new(syntax, &ts.themes["Solarized (dark)"]);
                let mut lines = vec![];
                for (i, line) in LinesWithEndings::from(&source_code).enumerate() {
                    let mut line_spans = vec![TextSpan::new(format!("{:>4} ", i + 1))];

                    h.highlight_line(line, &ps)
                        .unwrap()
                        .into_iter()
                        .for_each(|segment| {
                            if let Ok(span) = into_text_span(segment) {
                                line_spans.push(span)
                            }
                        });

                    lines.push(line_spans);
                }

                v.insert(lines)
            }
        }
    }
}

#[derive(MockComponent)]
pub struct Source {
    component: MultiSpanTextarea,
    file_cache: FileLinesCache,
}

impl Source {
    fn get_title(mb_file: Option<&Path>) -> String {
        if let Some(file) = mb_file {
            let abbreviator = Abbreviator::new("/", "/..", 70);

            format!(
                "Program source code ({:?})",
                abbreviator.apply(file.to_string_lossy().as_ref())
            )
        } else {
            "Program source code".into()
        }
    }

    pub fn new(exchanger: Arc<ClientExchanger>) -> anyhow::Result<Self> {
        let mb_threads = exchanger
            .request_sync(|dbg| dbg.thread_state())
            .expect("messaging enabled")
            .ok();
        let mb_place_in_focus = mb_threads.and_then(|threads| {
            threads
                .into_iter()
                .find_map(|snap| if snap.in_focus { snap.place } else { None })
        });

        let cache = FileLinesCache::default();
        let component = MultiSpanTextarea::default()
            .borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(Color::LightYellow),
            )
            .title("Program source code", Alignment::Center)
            .step(4)
            .inactive(Style::default().fg(Color::Gray))
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
            .cloned()
            .enumerate()
            .map(|(i, mut line)| {
                if Some((i + 1) as u64) == mb_line_num {
                    line.iter_mut().for_each(|text| text.fg = Color::LightRed)
                }
                line
            })
            .collect();
        self.component.text_rows(lines);

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
            Sub::new(SubEventClause::User(UserEvent::Exit(0)), SubClause::Always),
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
            Event::User(UserEvent::Exit { .. }) => {
                self.component.text_rows(vec![]);
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
