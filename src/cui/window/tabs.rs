use crate::cui::window::{Action, CuiComponent, RenderContext};
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::HashMap;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders};
use tui::Frame;

#[macro_export]
macro_rules! tab_switch_action {
    ($from: expr, $to: expr) => {
        vec![
            $crate::cui::window::Action::DeActivateComponent($from),
            $crate::cui::window::Action::HideComponent($from),
            $crate::cui::window::Action::ActivateComponent($to),
            $crate::cui::window::Action::ShowComponent($to),
        ]
    };
}

pub(super) struct TabVariant {
    title: &'static str,
    on_select: Box<[Action]>,
}

impl TabVariant {
    pub(super) fn new(title: &'static str, on_select: impl Into<Box<[Action]>>) -> Self {
        Self {
            title,
            on_select: on_select.into(),
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
    fn render(
        &self,
        _: RenderContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
    ) {
        let titles = self
            .tabs
            .iter()
            .map(|t| {
                let (first, rest) = t.title.split_at(1);
                Spans::from(vec![
                    Span::styled(
                        first,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::UNDERLINED),
                    ),
                    Span::styled(rest, Style::default().fg(Color::White)),
                ])
            })
            .collect();

        let tabs = tui::widgets::Tabs::new(titles)
            .select(self.active_tab)
            .block(Block::default().title(self.title).borders(Borders::ALL))
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().fg(Color::Yellow))
            .divider(Span::raw("|"));

        frame.render_widget(tabs, rect);
    }

    fn handle_user_event(&mut self, e: KeyEvent) -> Vec<Action> {
        if let KeyCode::Char(char_key) = e.code {
            if let Some(tab_idx) = self.hot_keys.get(&char_key) {
                self.active_tab = *tab_idx;
                return self.tabs[self.active_tab].on_select.clone().into();
            }
        }
        vec![]
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
