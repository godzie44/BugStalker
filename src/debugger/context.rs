use crate::debugger::{call::CallCache, debugee::dwarf::r#type::TypeCache};
use std::cell::RefCell;

#[derive(Default)]
pub struct GlobalContext {
    /// Type declaration cache.
    type_cache: RefCell<TypeCache>,

    /// Cache for called functions.
    call_cache: RefCell<CallCache>,
}

impl GlobalContext {
    /// Execute function with type cache mut ref.
    pub fn with_type_cache<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut TypeCache) -> T,
    {
        let mut cache = self.type_cache.borrow_mut();
        f(&mut cache)
    }

    /// Execute function with call cache mut ref.
    pub fn with_call_cache<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut CallCache) -> T,
    {
        let mut cache = self.call_cache.borrow_mut();
        f(&mut cache)
    }
}
