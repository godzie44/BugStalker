use crate::debugger::debugee::dwarf::{AsAllocatedData, ContextualDieRef, NamespaceHierarchy};
use bytes::Bytes;
use std::fmt::{Display, Formatter};

pub mod dqe;
pub mod execute;
pub mod render;
pub mod value;
mod r#virtual;

/// Identifier of a query result.
/// Consists name and namespace of the variable or argument.
#[derive(Clone, Default, PartialEq)]
pub struct Identity {
    namespace: NamespaceHierarchy,
    pub name: Option<String>,
}

impl Identity {
    pub fn new(namespace: NamespaceHierarchy, name: Option<String>) -> Self {
        Self { namespace, name }
    }

    pub fn from_die(var: &ContextualDieRef<impl AsAllocatedData>) -> Self {
        Self::new(var.namespaces(), var.die.name().map(String::from))
    }

    pub fn no_namespace(name: Option<String>) -> Self {
        Self {
            namespace: NamespaceHierarchy::default(),
            name,
        }
    }
}

impl Display for Identity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let namespaces = if self.namespace.is_empty() {
            String::default()
        } else {
            self.namespace.join("::") + "::"
        };

        match self.name.as_deref() {
            None => Ok(()),
            Some(name) => f.write_fmt(format_args!("{namespaces}{name}")),
        }
    }
}

/// Object binary representation in debugee memory.
pub struct ObjectBinaryRepr {
    /// Binary representation.
    pub raw_data: Bytes,
    /// Possible address of object data in debugee memory.
    /// It may not exist if there is no debug information, or if an object is allocated in registers.
    pub address: Option<usize>,
    /// Binary size.
    pub size: usize,
}
