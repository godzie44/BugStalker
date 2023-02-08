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

pub trait TryGetOrInsert<T> {
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
