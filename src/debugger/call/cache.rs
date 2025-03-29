use std::{
    collections::{HashMap, hash_map::Entry},
    rc::Rc,
};

use crate::{
    debugger::{
        Debugger,
        address::RelocatedAddress,
        debugee::dwarf::{AsAllocatedData, r#type::ComplexType},
    },
    type_from_cache,
};

#[derive(Eq, PartialEq, Hash)]
pub struct Key {
    linkage_name: String,
    name: Option<String>,
}

impl Key {
    pub fn new(linkage_name: impl Into<String>) -> Self {
        Self {
            linkage_name: linkage_name.into(),
            name: None,
        }
    }

    pub fn new_with_name(linkage_name: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            linkage_name: linkage_name.into(),
            name: Some(name.into()),
        }
    }
}

#[derive(Clone)]
pub struct Value {
    fn_addr: RelocatedAddress,
    fn_param_types: Rc<Box<[Rc<ComplexType>]>>,
}

impl Value {
    pub fn new(fn_addr: RelocatedAddress, fn_param_types: Box<[Rc<ComplexType>]>) -> Self {
        Self {
            fn_addr,
            fn_param_types: Rc::new(fn_param_types),
        }
    }

    pub fn fn_addr(&self) -> RelocatedAddress {
        self.fn_addr
    }

    pub fn fn_param_types(&self) -> &[Rc<ComplexType>] {
        &self.fn_param_types
    }
}

/// Cache for already called functions.
#[derive(Default)]
pub struct CallCache(HashMap<Key, Value>);

impl CallCache {
    pub fn get_or_insert(
        &mut self,
        dbg: &Debugger,
        linkage_name: &str,
        name: Option<&str>,
    ) -> Result<Value, crate::debugger::Error> {
        let key = if let Some(name) = name {
            Key::new_with_name(linkage_name, name)
        } else {
            Key::new(linkage_name)
        };
        let entry = self.0.entry(key);
        let fn_info = match entry {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let (dwarf, func) = dbg.search_fn_to_call(linkage_name, name)?;
                let fn_addr = func
                    .prolog_start_place()?
                    .address
                    .relocate_to_segment(&dbg.debugee, dwarf)?;

                let params = {
                    let mut type_cache = dbg.type_cache.borrow_mut();
                    func.parameters()
                        .into_iter()
                        .map(|die| type_from_cache!(die, type_cache))
                        .collect::<Result<Box<[_]>, _>>()?
                };
                e.insert(Value::new(fn_addr, params))
            }
        };
        Ok(fn_info.clone())
    }
}
