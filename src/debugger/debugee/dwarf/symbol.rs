use object::{Object, ObjectSymbol, ObjectSymbolTable, SymbolKind};
use std::collections::HashMap;
use std::ops::Deref;

#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub addr: u64,
}

#[derive(Debug, Clone)]
pub(super) struct SymbolTab(HashMap<String, Symbol>);

impl Deref for SymbolTab {
    type Target = HashMap<String, Symbol>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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
                        let name: String = symbol.name().unwrap_or_default().into();
                        (
                            name,
                            Symbol {
                                kind: symbol.kind(),
                                addr: symbol.address(),
                            },
                        )
                    })
                    .collect::<HashMap<_, _>>(),
            )
        })
    }
}
