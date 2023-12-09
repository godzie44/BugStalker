use log::{LevelFilter, Log, Metadata, Record};
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};

struct NopLogger;

impl Log for NopLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        false
    }

    fn log(&self, _: &Record) {}

    fn flush(&self) {}
}

/// This logger proxy an underline logger and make available a logger switch possibility.
#[derive(Clone)]
pub struct ProxyLogger {
    logger: Arc<RwLock<Box<dyn Log>>>,
}

pub static LOGGER_SWITCHER: Lazy<ProxyLogger> = Lazy::new(|| {
    let logger = ProxyLogger {
        logger: Arc::new(RwLock::new(Box::new(NopLogger))),
    };

    log::set_boxed_logger(Box::new(logger.clone())).expect("infallible");
    log::set_max_level(log::LevelFilter::Debug);

    logger
});

impl ProxyLogger {
    /// Switch logger to new implementation and reset a global maximum log level.
    ///
    /// # Arguments
    ///
    /// * `logger`: a logger implementation.
    /// * `level_filter`: a new maximum log level.
    pub fn switch<L: Log + 'static>(&self, logger: L, level_filter: LevelFilter) {
        *self.logger.write().unwrap() = Box::new(logger);
        log::set_max_level(level_filter);
    }
}

impl Log for ProxyLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.logger.read().unwrap().enabled(metadata)
    }

    fn log(&self, record: &Record) {
        self.logger.read().unwrap().log(record)
    }

    fn flush(&self) {
        self.logger.read().unwrap().flush()
    }
}
