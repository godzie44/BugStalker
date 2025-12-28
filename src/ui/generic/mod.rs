//! Generic UI components

/// Render async primitives
pub mod r#async;
/// Handle commands from [`crate::ui::command`]
pub mod command_handler;
/// Render files
pub mod file;
/// Help information
pub mod help;
/// Print rendered information
pub mod print;
/// User defined triggers
pub mod trigger;
/// Render variables and arguments
pub mod variable;
