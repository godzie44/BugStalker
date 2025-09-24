use crate::debugger::address::GlobalAddress;
use object::{Object, ObjectSymbol, ObjectSymbolTable, SymbolKind};
use regex::Regex;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Symbol<'a> {
    pub name: &'a str,
    pub kind: SymbolKind,
    pub addr: GlobalAddress,
}

#[derive(Debug, Clone)]
struct SymbolVal {
    pub kind: SymbolKind,
    pub addr: GlobalAddress,
}

type Name = String;

#[derive(Debug, Clone)]
pub(super) struct SymbolTab(HashMap<Name, SymbolVal>);

impl SymbolTab {
    pub(super) fn new<'data, 'file, OBJ>(object_file: &'data OBJ) -> Option<Self>
    where
        'data: 'file,
        OBJ: Object<'data, 'file>,
    {
        object_file.symbol_table().as_ref().map(|sym_table| {
            SymbolTab(
                sym_table
                    .symbols()
                    .map(|symbol| {
                        let name = symbol.name().unwrap_or_default();
                        let name = rustc_demangle::demangle(name).to_string();
                        (
                            name,
                            SymbolVal {
                                kind: symbol.kind(),
                                addr: symbol.address().into(),
                            },
                        )
                    })
                    .collect::<HashMap<_, _>>(),
            )
        })
    }

    pub fn find(&'_ self, regex: &Regex) -> Vec<Symbol<'_>> {
        let keys = self
            .0
            .keys()
            .filter(|key| regex.find(key.as_str()).is_some());
        keys.map(|k| {
            let s = &self.0[k];
            Symbol {
                name: k.as_str(),
                kind: s.kind,
                addr: s.addr,
            }
        })
        .collect()
    }
}
