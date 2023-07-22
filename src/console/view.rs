use crate::debugger::Place;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::{fs, io};

pub struct FileView {
    cached_lines: RefCell<HashMap<PathBuf, Box<[String]>>>,
}

impl FileView {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            cached_lines: RefCell::default(),
        }
    }

    pub fn render_source(&self, place: &Place, bounds: u64) -> anyhow::Result<String> {
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

        let result = file_lines
            .iter()
            .enumerate()
            .skip(start as usize)
            .take((bounds * 2 + 1) as usize)
            .fold(String::default(), |acc, (pos, line)| {
                let line_number = place.line_number as i64 - (line_pos as i64 - pos as i64);
                format!("{acc}{line_number} {line}\n")
            });

        Ok(result)
    }
}
