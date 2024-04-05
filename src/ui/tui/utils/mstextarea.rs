use std::collections::LinkedList;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::props::{Borders, PropPayload, PropValue, Style, TextModifiers, TextSpan};
use tuirealm::tui::layout::{Alignment, Corner, Rect};
use tuirealm::tui::prelude::Color;
use tuirealm::tui::widgets::{List, ListItem, ListState};
use tuirealm::{AttrValue, Attribute, Frame, MockComponent, Props, State};
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub struct MSTextareaStates {
    pub list_index: usize, // Index of selected item in textarea
    pub list_len: usize,   // Lines in text area
}

impl MSTextareaStates {
    /// ### set_list_len
    ///
    /// Set list length and fix list index
    pub fn set_list_len(&mut self, len: usize) {
        self.list_len = len;
        self.fix_list_index();
    }

    /// ### incr_list_index
    ///
    /// Incremenet list index
    pub fn incr_list_index(&mut self) {
        // Check if index is at last element
        if self.list_index + 1 < self.list_len {
            self.list_index += 1;
        }
    }

    /// ### decr_list_index
    ///
    /// Decrement list index
    pub fn decr_list_index(&mut self) {
        // Check if index is bigger than 0
        if self.list_index > 0 {
            self.list_index -= 1;
        }
    }

    /// ### fix_list_index
    ///
    /// Keep index if possible, otherwise set to lenght - 1
    pub fn fix_list_index(&mut self) {
        if self.list_index >= self.list_len && self.list_len > 0 {
            self.list_index = self.list_len - 1;
        } else if self.list_len == 0 {
            self.list_index = 0;
        }
    }

    /// ### list_index_at_first
    ///
    /// Set list index to the first item in the list
    pub fn list_index_at_first(&mut self) {
        self.list_index = 0;
    }

    /// ### list_index_at_last
    ///
    /// Set list index at the last item of the list
    pub fn list_index_at_last(&mut self) {
        if self.list_len > 0 {
            self.list_index = self.list_len - 1;
        } else {
            self.list_index = 0;
        }
    }

    /// ### calc_max_step_ahead
    ///
    /// Calculate the max step ahead to scroll list
    fn calc_max_step_ahead(&self, max: usize) -> usize {
        let remaining: usize = match self.list_len {
            0 => 0,
            len => len - 1 - self.list_index,
        };
        if remaining > max {
            max
        } else {
            remaining
        }
    }

    /// ### calc_max_step_ahead
    ///
    /// Calculate the max step ahead to scroll list
    fn calc_max_step_behind(&self, max: usize) -> usize {
        if self.list_index > max {
            max
        } else {
            self.list_index
        }
    }
}

/// ## Multi span textarea
///
/// Like a original `textarea` component but support multiple `TextSpan` in single line.
#[derive(Default)]
pub struct MultiSpanTextarea {
    props: Props,
    pub states: MSTextareaStates,
    hg_str: Option<String>,
}

impl MultiSpanTextarea {
    pub fn foreground(mut self, fg: Color) -> Self {
        self.attr(Attribute::Foreground, AttrValue::Color(fg));
        self
    }

    pub fn background(mut self, bg: Color) -> Self {
        self.attr(Attribute::Background, AttrValue::Color(bg));
        self
    }

    pub fn inactive(mut self, s: Style) -> Self {
        self.attr(Attribute::FocusStyle, AttrValue::Style(s));
        self
    }

    pub fn modifiers(mut self, m: TextModifiers) -> Self {
        self.attr(Attribute::TextProps, AttrValue::TextModifiers(m));
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

    pub fn step(mut self, step: usize) -> Self {
        self.attr(Attribute::ScrollStep, AttrValue::Length(step));
        self
    }

    pub fn highlighted_str<S: AsRef<str>>(mut self, s: S) -> Self {
        self.attr(
            Attribute::HighlightedStr,
            AttrValue::String(s.as_ref().to_string()),
        );
        self
    }

    pub fn text_rows(&mut self, rows: Vec<Vec<TextSpan>>) {
        let mut ll = LinkedList::new();

        for line in rows {
            let line: Vec<_> = line.into_iter().map(PropValue::TextSpan).collect();
            ll.push_back(PropPayload::Vec(line));
        }

        self.states.set_list_len(ll.len());
        self.attr(Attribute::Text, AttrValue::Payload(PropPayload::Linked(ll)));
    }
}

impl MockComponent for MultiSpanTextarea {
    fn view(&mut self, render: &mut Frame, area: Rect) {
        // Make a Span
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            // Make text items
            // Highlighted symbol
            self.hg_str = self
                .props
                .get(Attribute::HighlightedStr)
                .map(|x| x.unwrap_string());
            // NOTE: wrap width is width of area minus 2 (block) minus width of highlighting string
            let wrap_width =
                (area.width as usize) - self.hg_str.as_ref().map(|x| x.width()).unwrap_or(0) - 2;
            let lines: Vec<ListItem> =
                match self.props.get(Attribute::Text).map(|x| x.unwrap_payload()) {
                    Some(PropPayload::Linked(lines)) => lines
                        .into_iter()
                        .map(|x| x.unwrap_vec())
                        .map(|line| {
                            let line: Vec<_> = line
                                .into_iter()
                                .map(|span| span.unwrap_text_span())
                                .collect();
                            tui_realm_stdlib::utils::wrap_spans(
                                line.as_slice(),
                                wrap_width,
                                &self.props,
                            )
                        })
                        .map(ListItem::new)
                        .collect(),
                    _ => Vec::new(),
                };
            let foreground = self
                .props
                .get_or(Attribute::Foreground, AttrValue::Color(Color::Reset))
                .unwrap_color();
            let background = self
                .props
                .get_or(Attribute::Background, AttrValue::Color(Color::Reset))
                .unwrap_color();
            let modifiers = self
                .props
                .get_or(
                    Attribute::TextProps,
                    AttrValue::TextModifiers(TextModifiers::empty()),
                )
                .unwrap_text_modifiers();
            let title = self
                .props
                .get_or(
                    Attribute::Title,
                    AttrValue::Title((String::default(), Alignment::Center)),
                )
                .unwrap_title();
            let borders = self
                .props
                .get_or(Attribute::Borders, AttrValue::Borders(Borders::default()))
                .unwrap_borders();
            let focus = self
                .props
                .get_or(Attribute::Focus, AttrValue::Flag(false))
                .unwrap_flag();
            let inactive_style = self
                .props
                .get(Attribute::FocusStyle)
                .map(|x| x.unwrap_style());

            // Make component
            let block =
                tui_realm_stdlib::utils::get_block(borders, Some(title), focus, inactive_style);

            let render_area_h = block.inner(area).height as usize;
            let num_lines_to_show_at_top = render_area_h / 2;
            let offset_max = lines.len().saturating_sub(render_area_h);
            let offset = self
                .states
                .list_index
                .saturating_sub(num_lines_to_show_at_top)
                .min(offset_max);

            let mut state: ListState = ListState::default()
                .with_offset(offset)
                .with_selected(Some(self.states.list_index));

            let mut list = List::new(lines)
                .block(block)
                .start_corner(Corner::TopLeft)
                .style(
                    Style::default()
                        .fg(foreground)
                        .bg(background)
                        .add_modifier(modifiers),
                );

            if let Some(hg_str) = &self.hg_str {
                list = list.highlight_symbol(hg_str);
            }
            render.render_stateful_widget(list, area, &mut state);
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
        // Update list len and fix index
        self.states.set_list_len(
            match self.props.get(Attribute::Text).map(|x| x.unwrap_payload()) {
                Some(PropPayload::Vec(spans)) => spans.len(),
                Some(PropPayload::Linked(lines)) => lines.len(),
                _ => 0,
            },
        );
        self.states.fix_list_index();
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Move(Direction::Down) => {
                self.states.incr_list_index();
            }
            Cmd::Move(Direction::Up) => {
                self.states.decr_list_index();
            }
            Cmd::Scroll(Direction::Down) => {
                let step = self
                    .props
                    .get_or(Attribute::ScrollStep, AttrValue::Length(8))
                    .unwrap_length();
                let step = self.states.calc_max_step_ahead(step);
                (0..step).for_each(|_| self.states.incr_list_index());
            }
            Cmd::Scroll(Direction::Up) => {
                let step = self
                    .props
                    .get_or(Attribute::ScrollStep, AttrValue::Length(8))
                    .unwrap_length();
                let step = self.states.calc_max_step_behind(step);
                (0..step).for_each(|_| self.states.decr_list_index());
            }
            Cmd::GoTo(Position::Begin) => {
                self.states.list_index_at_first();
            }
            Cmd::GoTo(Position::End) => {
                self.states.list_index_at_last();
            }
            _ => {}
        }
        CmdResult::None
    }
}
