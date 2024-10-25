use crate::debugger::debugee::dwarf::unit::Unit;
use crate::debugger::r#async::AsyncError;
use crate::debugger::variable::value::Value;
use crate::version_switch;

/// Helpers for typed values.

/// Representation of a `tokio::task::id::Id` type.
pub struct TaskIdValue {
    id: u64,
}

impl TaskIdValue {
    /// Return representation of a `tokio::task::id::Id` type with respect of
    /// current rustc version.
    pub fn from_value(unit: &Unit, value: Value) -> Result<Self, AsyncError> {
        let rustc_version = unit.rustc_version().unwrap_or_default();
        let task_id = value
            .field("__0")
            .and_then(|v| {
                version_switch!(
                    rustc_version,
                    .. (1 . 79) => {
                        v.field("__0")?
                    },
                    (1 . 79) .. => {
                        v.field("__0")?.field("__0")?
                    },
                )
            })
            .ok_or(AsyncError::IncorrectAssumption(
                "task_id field not found in task structure",
            ))?;
        let Value::Scalar(task_id) = task_id else {
            return Err(AsyncError::IncorrectAssumption(
                "unexpected task_id field format in task structure",
            ));
        };
        let task_id = task_id.try_as_number().expect("should be a number") as u64;

        Ok(Self { id: task_id })
    }
}

impl From<TaskIdValue> for u64 {
    fn from(value: TaskIdValue) -> Self {
        value.id
    }
}
