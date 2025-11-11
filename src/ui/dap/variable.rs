use std::{collections::HashMap, sync::atomic::AtomicU16};

use dap::types::{Variable, VariablePresentationHint};
use itertools::Itertools;

use crate::debugger::{
    address::RelocatedAddress,
    variable::{
        render::{RenderValue, ValueLayout},
        value::Value,
    },
};

#[derive(Default)]
pub struct ReferenceRegistry(HashMap<u16, String>);

impl ReferenceRegistry {
    pub fn insert(&mut self, path: &str) -> u16 {
        static NONCE: AtomicU16 = AtomicU16::new(100);

        let next = NONCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0.insert(next, path.to_string());
        next
    }

    pub fn get_path(&self, id: u16) -> &str {
        &self.0[&id]
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VarScope {
    None = 0,
    Args = 1,
    Locals = 2,
}

impl TryFrom<u8> for VarScope {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(VarScope::None),
            1 => Ok(VarScope::Args),
            2 => Ok(VarScope::Locals),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VarRef {
    pub scope: VarScope,
    pub frame_info: u32,
    pub var_id: u16,
}

impl VarRef {
    pub fn decode(&self) -> i64 {
        (self.scope as u64 | ((self.frame_info as u64) << 8) | ((self.var_id as u64) << 40)) as i64
    }

    pub fn encode(raw: u64) -> Self {
        Self {
            scope: ((raw & 0xFF) as u8).try_into().unwrap(),
            frame_info: ((raw >> 8) & 0xFFFFFFFF) as u32,
            var_id: ((raw >> 40) & 0xFFFF) as u16,
        }
    }

    fn extend(other: VarRef, new_id: u16) -> Self {
        Self {
            scope: other.scope,
            frame_info: other.frame_info,
            var_id: new_id,
        }
    }

    #[allow(unused)]
    fn unexpanded() -> Self {
        VarRef {
            scope: VarScope::None,
            frame_info: 0,
            var_id: 0,
        }
    }
}

pub fn expand_and_collect(
    registry: &mut ReferenceRegistry,
    init_ref: VarRef,
    path: &str,
    value: &Value,
) -> Vec<Variable> {
    log::info!("collect_expand_variables: {path}");

    let Some(layout) = value.value_layout() else {
        return vec![];
    };

    match layout {
        ValueLayout::PreRendered(_) => into_dap_var_repr(registry, init_ref, path, "", value)
            .map(|v| vec![v])
            .unwrap_or_default(),
        ValueLayout::Referential(_) => into_dap_var_repr(registry, init_ref, path, "", value)
            .map(|v| vec![v])
            .unwrap_or_default(),
        ValueLayout::Wrapped(inner) => expand_and_collect(registry, init_ref, path, inner),
        ValueLayout::Structure(members) => members
            .iter()
            .filter_map(|member| {
                let name = member.field_name.as_ref()?;
                let path = format!("({path}).{name}");

                into_dap_var_repr(registry, init_ref, &path, name, &member.value)
            })
            .collect_vec(),
        ValueLayout::IndexedList(array_items) => array_items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let path = &format!("({path})[{i}]");
                into_dap_var_repr(registry, init_ref, path, &format!("{i}"), &item.value)
            })
            .collect_vec(),
        ValueLayout::NonIndexedList(values) => values
            .iter()
            .enumerate()
            .filter_map(|(i, value)| {
                let path = &format!("({path})[{i}]");
                into_dap_var_repr(registry, init_ref, path, &format!("{i}"), value)
            })
            .collect_vec(),
        ValueLayout::Map(items) => items
            .iter()
            .filter_map(|(key, value)| {
                let name = format!("{key:?}");
                let path = &format!("({path})[{name}]");
                into_dap_var_repr(registry, init_ref, path, &name, value)
            })
            .collect_vec(),
    }
}

pub fn into_dap_var_repr(
    registry: &mut ReferenceRegistry,
    init_ref: VarRef,
    path: &str,
    name: &str,
    value: &Value,
) -> Option<Variable> {
    let layout = value.value_layout()?;

    let default_data_hint = VariablePresentationHint {
        kind: Some(dap::types::VariablePresentationHintKind::Data),
        ..Default::default()
    };

    let mut named_variables = None;
    let mut indexed_variables = None;
    let mut variables_reference = 0;
    let r_value;

    match layout {
        ValueLayout::IndexedList(items) => {
            variables_reference = VarRef::extend(init_ref, registry.insert(path)).decode();
            indexed_variables = Some(items.len() as i64);
            r_value = format!("{value:?}");
        }
        ValueLayout::NonIndexedList(items) => {
            variables_reference = VarRef::extend(init_ref, registry.insert(path)).decode();
            indexed_variables = Some(items.len() as i64);
            r_value = format!("{value:?}");
        }
        ValueLayout::Map(items) => {
            variables_reference = VarRef::extend(init_ref, registry.insert(path)).decode();
            named_variables = Some(items.len() as i64);
            r_value = format!("{value:?}");
        }
        ValueLayout::Structure(members, ..) => {
            variables_reference = VarRef::extend(init_ref, registry.insert(path)).decode();
            named_variables = Some(members.len() as i64);
            r_value = format!("{value:?}");
        }
        ValueLayout::PreRendered(render) => {
            r_value = render.to_string();
        }
        ValueLayout::Referential(ptr) => {
            let deref_path = &format!("*({path})");
            variables_reference = VarRef::extend(init_ref, registry.insert(deref_path)).decode();
            r_value = format!("{}", RelocatedAddress::from(ptr as usize));
        }
        ValueLayout::Wrapped(inner) => {
            let mut var = into_dap_var_repr(registry, init_ref, path, name, inner);

            if let Some(v) = var.as_mut() {
                v.value = format!("{}::{}", RenderValue::r#type(inner).name_fmt(), v.value);
            }

            return var;
        }
    };

    let var = Variable {
        name: name.to_string(),
        value: r_value,
        type_field: Some(value.r#type().name_fmt().to_string()),
        presentation_hint: Some(default_data_hint.clone()),
        indexed_variables,
        named_variables,
        variables_reference,
        ..Default::default()
    };

    Some(var)
}
