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
