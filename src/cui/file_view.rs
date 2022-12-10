use crate::debugger::Place;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::{fs, io};

pub struct FileView {
    cached_lines: RefCell<HashMap<String, Box<[String]>>>,
}

impl FileView {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            cached_lines: RefCell::default(),
        }
    }

    pub fn render_source(&self, place: &Place) -> anyhow::Result<(String, u64)> {
        let line_number = if place.line_number == 0 {
            1
        } else {
            place.line_number
        };
        let line_pos = line_number - 1;

        let mut cache = self.cached_lines.borrow_mut();
        let file_lines = match cache.get(place.file) {
            None => {
                let file = fs::File::open(place.file)?;
                let lines = io::BufReader::new(file)
                    .lines()
                    .enumerate()
                    .filter_map(|(num, line)| {
                        let line = line.map(|l| format!("{ln} {l}", ln = num + 1));
                        line.ok()
                    })
                    .collect::<Vec<_>>();
                cache.insert(place.file.to_string(), lines.into_boxed_slice());
                cache.get(place.file).unwrap()
            }
            Some(lines) => lines,
        };

        let result = file_lines
            .iter()
            .enumerate()
            .fold("".to_string(), |acc, (pos, line)| {
                if pos as u64 == line_pos {
                    acc + "\n" + ">" + line
                } else {
                    acc + "\n" + line
                }
            });

        Ok((result, line_pos))
    }
}
