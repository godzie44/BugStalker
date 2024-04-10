use crate::debugger::PlaceDescriptor;
use crate::ui::syntax;
use crate::ui::syntax::StylizedLine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::{fs, io};
use syntect::util::as_24_bit_terminal_escaped;

#[derive(Default)]
pub struct FileView {
    cached_lines: RefCell<HashMap<PathBuf, Box<[String]>>>,
}

impl FileView {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::default()
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

        let syntax_renderer = syntax::rust_syntax_renderer();
        let mut line_renderer = syntax_renderer.line_renderer();

        let mut i = 0;
        let result = file_lines
            .iter()
            .skip(start as usize)
            .take(length as usize)
            .try_fold(String::default(), |acc, line| -> anyhow::Result<String> {
                let line_number = start + 1 + i;
                i += 1;

                match line_renderer.render_line(line)? {
                    StylizedLine::NoneStyle(line) => Ok(format!("{acc}{line_number:>4} {line}\n")),
                    StylizedLine::Stylized(segments) => {
                        let escaped = as_24_bit_terminal_escaped(&segments, false);
                        Ok(format!("{acc}{line_number:>4} {escaped}\x1b[0m\n"))
                    }
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
        file: &Path,
        from_line: u64,
        to_line: u64,
    ) -> anyhow::Result<String> {
        let start = if from_line == 0 { 0 } else { from_line - 1 };
        let bound = to_line - from_line + 1;

        self.render(file, start, bound)
    }
}
