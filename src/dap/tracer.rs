use anyhow::Context;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Simple file-based tracer for adapter diagnostics.
#[derive(Clone)]
pub struct FileTracer {
    file: Arc<Mutex<std::fs::File>>,
}

impl FileTracer {
    pub fn new(path: &std::path::Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open log file {}", path.display()))?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub fn line(&self, text: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{text}");
        }
    }
}
