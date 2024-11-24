/// Translate syntect styled text to [`tuirealm::props::TextSpan`]
use anyhow::anyhow;
use tuirealm::props::TextSpan;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::style::Modifier;

pub fn into_text_span(
    (style, content): (syntect::highlighting::Style, &str),
) -> anyhow::Result<TextSpan> {
    Ok(TextSpan {
        content: String::from(content),
        fg: translate_color(style.foreground).unwrap_or_default(),
        bg: Color::default(),
        modifiers: translate_font_style(style.font_style)?,
    })
}

pub fn translate_color(syntect_color: syntect::highlighting::Color) -> Option<Color> {
    match syntect_color {
        syntect::highlighting::Color { r, g, b, a } if a > 0 => Some(Color::Rgb(r, g, b)),
        _ => None,
    }
}

pub fn translate_font_style(
    syntect_font_style: syntect::highlighting::FontStyle,
) -> anyhow::Result<Modifier> {
    use syntect::highlighting::FontStyle;
    match syntect_font_style {
        x if x == FontStyle::empty() => Ok(Modifier::empty()),
        x if x == FontStyle::BOLD => Ok(Modifier::BOLD),
        x if x == FontStyle::ITALIC => Ok(Modifier::ITALIC),
        x if x == FontStyle::UNDERLINE => Ok(Modifier::UNDERLINED),
        x if x == FontStyle::BOLD | FontStyle::ITALIC => Ok(Modifier::BOLD | Modifier::ITALIC),
        x if x == FontStyle::BOLD | FontStyle::UNDERLINE => {
            Ok(Modifier::BOLD | Modifier::UNDERLINED)
        }
        x if x == FontStyle::ITALIC | FontStyle::UNDERLINE => {
            Ok(Modifier::ITALIC | Modifier::UNDERLINED)
        }
        x if x == FontStyle::BOLD | FontStyle::ITALIC | FontStyle::UNDERLINE => {
            Ok(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED)
        }
        unknown => Err(anyhow!("unknown font style: {:?}", unknown.bits())),
    }
}
