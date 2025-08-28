use std::fmt::Write as _;
use std::io::Write;
use std::sync::{Arc, Mutex};

use dap::events::{Event, OutputEventBody};
use dap::server::ServerOutput;
use dap::types::OutputEventCategory;
use log::LevelFilter;

pub struct DapLogger<W: Write> {
    inner: env_logger::Logger,
    output: Arc<Mutex<ServerOutput<W>>>,
}

impl<W: Write> DapLogger<W> {
    pub fn new(output: Arc<Mutex<ServerOutput<W>>>) -> Self {
        Self {
            inner: env_logger::Logger::from_default_env(),
            output,
        }
    }

    pub fn filter(&self) -> LevelFilter {
        self.inner.filter()
    }
}

impl<W: Write + Send> log::Log for DapLogger<W> {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        let mut output = String::new();

        write!(output, "[{}] ", record.level()).unwrap();

        if let Some(module) = record.module_path() {
            write!(output, "{module} ").unwrap();
        }

        writeln!(output, "{}", record.args()).unwrap();

        self.output
            .lock()
            .unwrap()
            .send_event(Event::Output(OutputEventBody {
                category: Some(OutputEventCategory::Console),
                output,
                ..Default::default()
            }))
            .unwrap();
    }

    fn flush(&self) {}
}
