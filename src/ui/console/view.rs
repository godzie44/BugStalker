use crate::debugger::PlaceDescriptor;
use anyhow::{anyhow, bail};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
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

    fn render(&self, file_path: &Path, start: u64, length: u64) -> anyhow::Result<String> {
        let mut cache = self.cached_lines.borrow_mut();
        let file_lines = match cache.get(file_path) {
            None => {
                let file = fs::File::open(file_path)?;
                let lines = io::BufReader::new(file)
                    .lines()
                    .map_while(Result::ok)
                    .collect::<Vec<_>>();
                cache.insert(file_path.to_path_buf(), lines.into_boxed_slice());
                cache.get(file_path).unwrap()
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

        let mut i = 0;
        let result = file_lines
            .iter()
            .skip(start as usize)
            .take(length as usize)
            .try_fold(String::default(), |acc, line| -> anyhow::Result<String> {
                let line_number = start + 1 + i;
                i += 1;

                #[cfg(feature = "int_test")]
                {
                    Ok(format!("{acc}{line_number:>4} {line}\n"))
                }
                #[cfg(not(feature = "int_test"))]
                {
                    use syntect::highlighting::Style;
                    use syntect::util::as_24_bit_terminal_escaped;

                    let ranges: Vec<(Style, &str)> = h.highlight_line(line, &self.ps)?;
                    let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                    Ok(format!("{acc}{line_number:>4} {escaped}\x1b[0m\n"))
                }
            })?;

        Ok(result)
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

        self.render(place.file, start, bounds * 2 + 1)
    }

    pub fn render_source_range(
        &self,
        from: &PlaceDescriptor,
        to: &PlaceDescriptor,
    ) -> anyhow::Result<String> {
        if from.file != to.file {
            bail!("invalid render range")
        }

        let start = if from.line_number == 0 {
            0
        } else {
            from.line_number - 1
        };
        let bound = to.line_number - from.line_number + 1;

        self.render(from.file, start, bound)
    }
}
