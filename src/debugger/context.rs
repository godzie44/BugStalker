use crate::debugger::{call::CallCache, debugee::dwarf::r#type::TypeCache};
use std::{
    cell::RefCell,
    sync::{LazyLock, Mutex},
};

/// Promise to use value only at one unique thread
#[derive(Default)]
struct SingleThreadPromise<T: Default>(T);

// SAFETY: cause promise never share this value between threads
unsafe impl<T: Default> Sync for SingleThreadPromise<T> {}
// SAFETY: cause promise never share this value between threads
unsafe impl<T: Default> Send for SingleThreadPromise<T> {}

#[derive(Default)]
pub struct GlobalContext {
    /// Type declaration cache.
    type_cache: SingleThreadPromise<RefCell<TypeCache>>,

    /// Cache for called functions.
    call_cache: SingleThreadPromise<RefCell<CallCache>>,

    /// String interner
    interner: Mutex<string_interner::StringInterner<string_interner::DefaultBackend>>,
}

// TODO: make this context part of the debugger structure
static GCX: LazyLock<GlobalContext> = LazyLock::new(GlobalContext::default);

pub fn gcx() -> &'static GlobalContext {
    &GCX
}

impl GlobalContext {
    /// Execute function with type cache mut ref.
    pub fn with_type_cache<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut TypeCache) -> T,
    {
        let mut cache = self.type_cache.0.borrow_mut();
        f(&mut cache)
    }

    /// Execute function with call cache mut ref.
    pub fn with_call_cache<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut CallCache) -> T,
    {
        let mut cache = self.call_cache.0.borrow_mut();
        f(&mut cache)
    }

    pub fn with_interner<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut string_interner::StringInterner<string_interner::DefaultBackend>) -> T,
    {
        let mut interner = self.interner.lock().unwrap();
        f(&mut interner)
    }
}
