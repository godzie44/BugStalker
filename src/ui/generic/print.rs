use crate::ui::console::editor::BSEditor;
use rustyline::ExternalPrinter as RLExternalPrinter;
use std::cell::RefCell;
use std::fmt::Display;
use std::rc::Rc;

/// [ExternalPrinter] safely prints messages to stdout or another destination.
///
/// There is a problem with [`ExternalPrinter`] and integration tests, see [this issue](https://github.com/kkawakam/rustyline/issues/703).
/// That's why in test environment external printer disabled.
pub struct ExternalPrinter {
    printer: Option<RefCell<Box<dyn RLExternalPrinter>>>,
}

unsafe impl Send for ExternalPrinter {}

impl ExternalPrinter {
    #[cfg(not(feature = "int_test"))]
    pub fn new_for_editor(editor: &mut BSEditor) -> rustyline::Result<Self> {
        let external_p = editor.create_external_printer()?;

        Ok(Self::new(Box::new(external_p)))
    }

    #[cfg(feature = "int_test")]
    pub fn new_for_editor(_editor: &mut BSEditor) -> rustyline::Result<Self> {
        Ok(Self { printer: None })
    }

    pub fn new(p: Box<dyn RLExternalPrinter>) -> Self {
        Self {
            printer: Some(RefCell::new(p)),
        }
    }

    pub fn take(self) -> Option<RefCell<Box<dyn RLExternalPrinter>>> {
        self.printer
    }

    pub fn print(&self, msg: impl Display) {
        let msg = msg.to_string();
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

    pub fn println(&self, msg: impl Display) {
        let msg = format!("{msg}\n");
        self.print(msg)
    }
}

pub mod style {
    use crossterm::style::{Color, Stylize};
    use std::fmt::{Display, Formatter};

    pub const UNKNOWN_PLACEHOLDER: &str = "???";

    struct View<T: Display> {
        inner: Option<T>,
        color: Color,
    }

    impl<T: Display> Display for View<T> {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            let addr = self
                .inner
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| UNKNOWN_PLACEHOLDER.to_string());

            if cfg!(feature = "int_test") {
                f.write_str(&addr)
            } else {
                f.write_fmt(format_args!("{}", addr.with(self.color)))
            }
        }
    }

    /// Construct structure declaration to display data of the same type (file paths, addresses, etc.).
    /// A display style will reset if program compile with `int_test` feature.
    macro_rules! view_struct {
        ($name: ident, $color: expr) => {
            pub struct $name<T: Display>(View<T>);

            impl<T: Display> From<T> for $name<T> {
                fn from(value: T) -> Self {
                    Self(View {
                        inner: Some(value),
                        color: $color,
                    })
                }
            }

            impl<T: Display> From<Option<T>> for $name<T> {
                fn from(value: Option<T>) -> Self {
                    Self(View {
                        inner: value,
                        color: $color,
                    })
                }
            }

            impl<T: Display> Display for $name<T> {
                fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                    self.0.fmt(f)
                }
            }
        };
    }

    view_struct!(AddressView, Color::Blue);
    view_struct!(FilePathView, Color::Green);
    view_struct!(FunctionNameView, Color::Yellow);
    view_struct!(KeywordView, Color::Magenta);
    view_struct!(AsmInstructionView, Color::DarkRed);
    view_struct!(AsmOperandsView, Color::DarkGreen);
    view_struct!(ErrorView, Color::DarkRed);
    view_struct!(ImportantView, Color::Magenta);

    view_struct!(AsyncTaskView, Color::Green);
    view_struct!(FutureFunctionView, Color::Yellow);
    view_struct!(FutureTypeView, Color::Magenta);
}

#[derive(Default)]
pub struct InStringPrinter {
    data: Rc<RefCell<String>>,
}

impl RLExternalPrinter for InStringPrinter {
    fn print(&mut self, msg: String) -> rustyline::Result<()> {
        *self.data.borrow_mut() += &msg;
        Ok(())
    }
}

impl InStringPrinter {
    pub fn new(data: Rc<RefCell<String>>) -> Self {
        Self { data: data }
    }
}
