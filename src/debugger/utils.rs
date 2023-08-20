#[macro_export]
macro_rules! _error {
    ($log_fn: path, $res: expr) => {
        match $res {
            Ok(value) => Some(value),
            Err(e) => {
                $log_fn!(target: "debugger", "{:#}", e);
                None
            }
        }
    };
    ($log_fn: path, $res: expr, $msg: tt) => {
        match $res {
            Ok(value) => Some(value),
            Err(e) => {
                $log_fn!(target: "debugger", concat!($msg, " {:#}"), e);
                None
            }
        }
    };
}

/// Transforms `Result` into `Option` and logs an error if it occurs.
#[macro_export]
macro_rules! weak_error {
    ($res: expr) => {
        $crate::_error!(log::warn, $res)
    };
    ($res: expr, $msg: tt) => {
        $crate::_error!(log::warn, $res, $msg)
    };
}

/// Transforms `Result` into `Option` and put error into debug logs if it occurs.
#[macro_export]
macro_rules! muted_error {
    ($res: expr) => {
        $crate::_error!(log::debug, $res)
    };
    ($res: expr, $msg: tt) => {
        $crate::_error!(log::debug, $res, $msg)
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
