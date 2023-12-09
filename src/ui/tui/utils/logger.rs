use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::sync;
use std::sync::Arc;
use tuirealm::props::{Color, TextSpan};

#[derive(PartialEq, PartialOrd, Clone, Debug, Eq)]
pub struct TuiLogLine {
    level: Level,
    time: String,
    target: String,
    body: String,
}

impl TuiLogLine {
    pub fn to_text_spans(&self) -> Vec<TextSpan> {
        fn fg_for_level(lvl: Level) -> Color {
            match lvl {
                Level::Error => Color::Red,
                Level::Warn => Color::Yellow,
                Level::Info => Color::Green,
                Level::Debug => Color::Magenta,
                Level::Trace => Color::LightBlue,
            }
        }

        vec![
            TextSpan::new(format!("[{} ", self.time)),
            TextSpan::new(self.level.to_string()).fg(fg_for_level(self.level)),
            TextSpan::new(format!(" {}] {}", self.target, self.body)),
        ]
    }
}

pub struct TuiLogger {
    inner: env_logger::Logger,
    buffer: Arc<sync::Mutex<Vec<TuiLogLine>>>,
}

impl Log for TuiLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        let ts = Local::now();
        let log = TuiLogLine {
            level: record.level(),
            time: ts.to_rfc3339(),
            target: record.target().to_string(),
            body: format!("{}", record.args()),
        };

        self.buffer.lock().unwrap().push(log);
    }

    fn flush(&self) {}
}

impl TuiLogger {
    pub fn new(buffer: Arc<sync::Mutex<Vec<TuiLogLine>>>) -> Self {
        Self {
            inner: env_logger::Logger::from_env(env_logger::Env::default()),
            buffer,
        }
    }

    pub fn filter(&self) -> LevelFilter {
        self.inner.filter()
    }
}
