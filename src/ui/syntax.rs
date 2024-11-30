use crate::ui::config;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, ThemeSet};
use syntect::parsing::SyntaxSet;

pub struct RustCodeLineRenderer<'a> {
    syntax_set: &'a SyntaxSet,
    highlighter: Option<HighlightLines<'a>>,
}

/// Stylized line representation.
pub enum StylizedLine<'a> {
    /// No styling needed.
    NoneStyle(&'a str),
    /// `syntect` stylized line, list of stylized line segments.
    Stylized(Vec<(Style, &'a str)>),
}

impl RustCodeLineRenderer<'_> {
    /// Prettify rust code-line if needed.
    pub fn render_line<'s>(&mut self, line: &'s str) -> anyhow::Result<StylizedLine<'s>> {
        match &mut self.highlighter {
            None => Ok(StylizedLine::NoneStyle(line)),
            Some(h) => Ok(StylizedLine::Stylized(
                h.highlight_line(line, self.syntax_set)?,
            )),
        }
    }
}

pub struct RustCodeRenderer {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl Default for RustCodeRenderer {
    fn default() -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }
}

impl RustCodeRenderer {
    const RUST_EXT: &'static str = "rs";

    pub fn line_renderer(&self) -> RustCodeLineRenderer {
        let theme = match config::current().theme.to_syntect_name() {
            None => {
                return RustCodeLineRenderer {
                    syntax_set: &self.syntax_set,
                    highlighter: None,
                };
            }
            Some(theme_name) => &self.theme_set.themes[theme_name],
        };
        let syntax_ref = self
            .syntax_set
            .find_syntax_by_extension(Self::RUST_EXT)
            .expect("rust syntax should exist");
        let highlighter = HighlightLines::new(syntax_ref, theme);

        RustCodeLineRenderer {
            syntax_set: &self.syntax_set,
            highlighter: Some(highlighter),
        }
    }
}

static RENDERER: OnceLock<RustCodeRenderer> = OnceLock::new();

/// Return current source code renderer.
pub fn rust_syntax_renderer() -> &'static RustCodeRenderer {
    RENDERER.get_or_init(RustCodeRenderer::default)
}
