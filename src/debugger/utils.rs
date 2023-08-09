/// Transforms `Result` into `Option` and logs an error if it occurs.
#[macro_export]
macro_rules! weak_error {
    ($res: expr) => {
        match $res {
            Ok(value) => Some(value),
            Err(e) => {
                log::warn!(target: "debugger", "{:#}", e);
                None
            }
        }
    };
}

/// Macro for handle an error lists as warnings.
#[macro_export]
macro_rules! print_warns {
    ($errors:expr) => {
        $errors.iter().for_each(|e| {
            log::warn!(target: "debugger", "{:#}", e);
        })
    };
}

/// Types can implement this trait for include cache functionality.
pub trait TryGetOrInsert<T> {
    /// Returns inner value if exists, otherwise execute function `f`, then save returned value and return it.
    ///
    /// # Arguments
    ///
    /// * `f`: function executed if inner value not exists.
    fn try_get_or_insert_with<E, F>(&mut self, f: F) -> Result<&T, E>
    where
        F: FnOnce() -> Result<T, E>;
}

impl<T> TryGetOrInsert<T> for Option<T> {
    fn try_get_or_insert_with<E, F>(&mut self, f: F) -> Result<&T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        match self {
            Some(value) => Ok(value),
            None => Ok(self.insert(f()?)),
        }
    }
}
