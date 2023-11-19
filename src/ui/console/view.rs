use crate::debugger::PlaceDescriptor;
use anyhow::anyhow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::{fs, io};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

pub struct FileView {
    cached_lines: RefCell<HashMap<PathBuf, Box<[String]>>>,

    ps: SyntaxSet,
    ts: ThemeSet,
}

impl FileView {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            cached_lines: RefCell::default(),
            ps: SyntaxSet::load_defaults_newlines(),
            ts: ThemeSet::load_defaults(),
        }
    }

    pub fn render_source(&self, place: &PlaceDescriptor, bounds: u64) -> anyhow::Result<String> {
        let line_number = if place.line_number == 0 {
            1
        } else {
            place.line_number
        };
        let line_pos = line_number - 1;
        let start = if line_pos < bounds {
            0
        } else {
            line_pos - bounds
        };

        let mut cache = self.cached_lines.borrow_mut();
        let file_lines = match cache.get(place.file) {
            None => {
                let file = fs::File::open(place.file)?;
                let lines = io::BufReader::new(file)
                    .lines()
                    .map_while(Result::ok)
                    .collect::<Vec<_>>();
                cache.insert(place.file.to_path_buf(), lines.into_boxed_slice());
                cache.get(place.file).unwrap()
            }
            Some(lines) => lines,
        };

        #[allow(unused)]
        let syntax = self
            .ps
            .find_syntax_by_extension("rs")
            .ok_or(anyhow!("rust hl extension must exists"))?;
        #[allow(unused)]
        let mut h = HighlightLines::new(syntax, &self.ts.themes["Solarized (dark)"]);

        let result = file_lines
            .iter()
            .enumerate()
            .skip(start as usize)
            .take((bounds * 2 + 1) as usize)
            .try_fold(
                String::default(),
                |acc, (pos, line)| -> anyhow::Result<String> {
                    let line_number = place.line_number as i64 - (line_pos as i64 - pos as i64);

                    #[cfg(feature = "int_test")]
                    {
                        Ok(format!("{acc}{line_number} {line}\n"))
                    }
                    #[cfg(not(feature = "int_test"))]
                    {
                        use syntect::highlighting::Style;
                        use syntect::util::as_24_bit_terminal_escaped;

                        let ranges: Vec<(Style, &str)> = h.highlight_line(line, &self.ps)?;
                        let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                        Ok(format!("{acc}{line_number} {escaped}\x1b[0m\n"))
                    }
                },
            )?;

        Ok(result)
    }
}
