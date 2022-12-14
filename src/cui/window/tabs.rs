use crate::cui::window::{message, CuiComponent, RenderOpts};
use crate::cui::{context, AppState};
use crate::fire;
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::HashMap;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, BorderType, Borders};
use tui::Frame;

pub(super) struct TabVariant {
    title: String,
    active_state: Option<AppState>,
    on_select: message::ActionMessage,
    message_recipient: &'static str,
}

impl TabVariant {
    pub(super) fn new(
        title: impl Into<String>,
        on_select: message::ActionMessage,
        message_recipient: &'static str,
    ) -> Self {
        Self {
            title: title.into().to_uppercase(),
            on_select,
            active_state: None,
            message_recipient,
        }
    }

    pub(super) fn contextual(
        title: impl Into<String>,
        on_select: message::ActionMessage,
        state: AppState,
        message_recipient: &'static str,
    ) -> Self {
        Self {
            title: title.into().to_uppercase(),
            on_select,
            active_state: Some(state),
            message_recipient,
        }
    }
}

pub(super) struct Tabs {
    name: &'static str,
    title: &'static str,
    tabs: Vec<TabVariant>,
    active_tab: usize,
    hot_keys: HashMap<char, usize>,
}

impl Tabs {
    pub(super) fn new(name: &'static str, title: &'static str, tabs: Vec<TabVariant>) -> Self {
        Self {
            name,
            title,
            hot_keys: tabs
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    let first_char = t.title.chars().next()?;
                    Some((first_char.to_lowercase().next().unwrap_or(first_char), i))
                })
                .collect(),
            tabs,
            active_tab: 0,
        }
    }
}

impl CuiComponent for Tabs {
    fn render(&self, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect, _: RenderOpts) {
        let titles = self
            .tabs
            .iter()
            .map(|t| {
                let inactive_tab = t
                    .active_state
                    .map(|s| !context::Context::current().assert_state(s))
                    .unwrap_or(false);

                if inactive_tab {
                    Span::styled(
                        t.title.as_str().to_uppercase(),
                        Style::default().fg(Color::Gray),
                    )
                    .into()
                } else {
                    let (first, rest) = t.title.split_at(1);
                    Spans::from(vec![
                        Span::styled(
                            first.to_uppercase(),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                }
            })
            .collect();

        let tabs = tui::widgets::Tabs::new(titles)
            .select(self.active_tab)
            .block(
                Block::default()
                    .title(self.title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().fg(Color::Yellow))
            .divider(Span::raw("|"));

        frame.render_widget(tabs, rect);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        if let KeyCode::Char(char_key) = e.code {
            if let Some(tab_idx) = self.hot_keys.get(&char_key) {
                let tab = &self.tabs[*tab_idx];

                if tab
                    .active_state
                    .map(|expected_state| context::Context::current().assert_state(expected_state))
                    .unwrap_or(true)
                {
                    self.active_tab = *tab_idx;
                    fire!(tab.on_select.clone() => tab.message_recipient);
                }
            }
        }
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
