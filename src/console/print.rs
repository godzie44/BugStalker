use rustyline::history::History;
use rustyline::{Editor, ExternalPrinter as RLExternalPrinter, Helper};
use std::cell::RefCell;

/// [`ExternalPrinter`] safe print messages to stdout
///
/// There is a problem with [`ExternalPrinter`] and integration tests, see [this issue](https://github.com/kkawakam/rustyline/issues/703).
/// That's why in test environment external printer disabled.
pub struct ExternalPrinter {
    printer: Option<RefCell<Box<dyn RLExternalPrinter>>>,
}

unsafe impl Send for ExternalPrinter {}

impl ExternalPrinter {
    #[cfg(not(feature = "int_test"))]
    pub fn new<H: Helper, I: History>(editor: &mut Editor<H, I>) -> rustyline::Result<Self> {
        let external_p = editor.create_external_printer()?;
        Ok(Self {
            printer: Some(RefCell::new(Box::new(external_p))),
        })
    }

    #[cfg(feature = "int_test")]
    pub fn new<H: Helper, I: History>(_editor: &mut Editor<H, I>) -> rustyline::Result<Self> {
        Ok(Self { printer: None })
    }

    pub fn print(&self, msg: impl Into<String>) {
        let msg = msg.into();
        match &self.printer {
            None => {
                println!("{msg}")
            }
            Some(printer) => {
                printer
                    .borrow_mut()
                    .print(msg)
                    .expect("external printer error");
            }
        }
    }
}
