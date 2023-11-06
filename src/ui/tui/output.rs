use crate::ui::DebugeeOutReader;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::{io, thread};
use timeout_readwrite::TimeoutReader;

#[derive(Default, Clone)]
pub struct Handle {
    flag: Arc<AtomicBool>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst)
    }
}

#[derive(PartialEq, Eq, Clone, PartialOrd)]
pub enum OutputLine {
    Out(String),
    Err(String),
}

#[derive(Clone, Copy)]
pub enum StreamType {
    StdErr,
    StdOut,
}

pub struct OutputStreamProcessor {
    r#type: StreamType,
}

impl OutputStreamProcessor {
    pub fn new(r#type: StreamType) -> Self {
        Self { r#type }
    }

    pub fn run(
        self,
        stream: TimeoutReader<DebugeeOutReader>,
        output_buf: Arc<Mutex<Vec<OutputLine>>>,
    ) -> Handle {
        let handle = Handle::default();

        {
            let handle = handle.clone();
            thread::spawn(move || {
                let mut stream = BufReader::new(stream);
                loop {
                    if handle.flag.load(Ordering::SeqCst) {
                        return;
                    }

                    let mut line = String::new();
                    let size = match stream.read_line(&mut line) {
                        Ok(size) => size,
                        Err(e) => {
                            if e.kind() == io::ErrorKind::TimedOut {
                                continue;
                            }
                            0
                        }
                    };

                    if size == 0 {
                        return;
                    }
                    let line = match self.r#type {
                        StreamType::StdErr => OutputLine::Err(line),
                        StreamType::StdOut => OutputLine::Out(line),
                    };
                    output_buf.lock().unwrap().push(line);
                }
            });
        }

        handle
    }
}
