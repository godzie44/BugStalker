use dap::types::{Variable, VariablePresentationHint};
use itertools::Itertools;

use crate::{
    debugger::{
        address::RelocatedAddress,
        variable::{
            render::{RenderValue, ValueLayout},
            value::Value,
        },
    },
    ui::dap::FrameInfo,
};

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
pub struct Key {
    inner: usize,
}

impl Key {
    pub fn from_var_ref(var_ref: i64) -> Self {
        Key {
            inner: var_ref as usize,
        }
    }

    pub fn as_var_ref(&self) -> i64 {
        self.inner as i64
    }

    fn new_unstable(idx: usize) -> Self {
        assert!(idx < u32::MAX as usize);
        Self {
            inner: (idx | (1usize << 0x21)),
        }
    }

    #[allow(unused)]
    fn new_stable(idx: usize) -> Self {
        assert!(idx < u32::MAX as usize);
        Self { inner: idx }
    }

    pub fn is_stable(&self) -> bool {
        (self.inner & (1usize << 0x21)) == 0
    }

    pub fn idx(&self) -> usize {
        (self.inner & (u32::MAX as usize)) as usize
    }
}

#[derive(Clone, PartialEq)]
pub enum Selector {
    Dqe(String),
    All,
}

#[derive(Clone)]
pub struct Var {
    pub scope: VarScope,
    pub frame: FrameInfo,
    pub selector: Selector,
}

#[derive(Default)]
pub struct VarRegistry {
    stable: Vec<Var>,
    unstable: Vec<Var>,
}

impl VarRegistry {
    pub fn insert_unstable(&mut self, var: Var) -> Key {
        self.unstable.push(var);
        Key::new_unstable(self.unstable.len() - 1)
    }

    #[allow(unused)]
    pub fn insert_stable(&mut self, var: Var) -> Key {
        self.stable.push(var);
        Key::new_stable(self.stable.len() - 1)
    }

    pub fn get_var(&self, key: Key) -> &Var {
        log::info!("got key: {}", key.inner);
        log::info!("is_stable: {}", key.is_stable());
        if key.is_stable() {
            &self.stable[key.idx()]
        } else {
            &self.unstable[key.idx()]
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VarScope {
    None = 0,
    Args = 1,
    Vars = 2,
}

impl TryFrom<u8> for VarScope {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(VarScope::None),
            1 => Ok(VarScope::Args),
            2 => Ok(VarScope::Vars),
            _ => Err(()),
        }
    }
}

pub fn expand_and_collect(
    registry: &mut VarRegistry,
    scope: VarScope,
    frame_info: FrameInfo,
    path: &str,
    value: &Value,
) -> Vec<Variable> {
    log::info!("collect_expand_variables: {path}");

    let Some(layout) = value.value_layout() else {
        return vec![];
    };

    match layout {
        ValueLayout::PreRendered(_) => {
            into_dap_var_repr(registry, scope, frame_info, path, "", value)
                .map(|v| vec![v])
                .unwrap_or_default()
        }
        ValueLayout::Referential(_) => {
            into_dap_var_repr(registry, scope, frame_info, path, "", value)
                .map(|v| vec![v])
                .unwrap_or_default()
        }
        ValueLayout::Wrapped(inner) => expand_and_collect(registry, scope, frame_info, path, inner),
        ValueLayout::Structure(members) => members
            .iter()
            .filter_map(|member| {
                let name = member.field_name.as_ref()?;
                let path = format!("({path}).{name}");

                into_dap_var_repr(registry, scope, frame_info, &path, name, &member.value)
            })
            .collect_vec(),
        ValueLayout::IndexedList(array_items) => array_items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let path = &format!("({path})[{i}]");
                into_dap_var_repr(
                    registry,
                    scope,
                    frame_info,
                    path,
                    &format!("{i}"),
                    &item.value,
                )
            })
            .collect_vec(),
        ValueLayout::NonIndexedList(values) => values
            .iter()
            .enumerate()
            .filter_map(|(i, value)| {
                let path = &format!("({path})[{i}]");
                into_dap_var_repr(registry, scope, frame_info, path, &format!("{i}"), value)
            })
            .collect_vec(),
        ValueLayout::Map(items) => items
            .iter()
            .filter_map(|(key, value)| {
                let name = format!("{key:?}");
                let path = &format!("({path})[{name}]");
                into_dap_var_repr(registry, scope, frame_info, path, &name, value)
            })
            .collect_vec(),
    }
}

pub fn into_dap_var_repr(
    registry: &mut VarRegistry,
    scope: VarScope,
    frame_info: FrameInfo,
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
            indexed_variables = Some(items.len() as i64);
            let key = registry.insert_unstable(Var {
                scope,
                frame: frame_info,
                selector: Selector::Dqe(path.to_string()),
            });
            variables_reference = key.as_var_ref();
            r_value = format!("{value:?}");
        }
        ValueLayout::NonIndexedList(items) => {
            indexed_variables = Some(items.len() as i64);
            let key = registry.insert_unstable(Var {
                scope,
                frame: frame_info,
                selector: Selector::Dqe(path.to_string()),
            });
            variables_reference = key.as_var_ref();
            r_value = format!("{value:?}");
        }
        ValueLayout::Map(items) => {
            named_variables = Some(items.len() as i64);
            let key = registry.insert_unstable(Var {
                scope,
                frame: frame_info,
                selector: Selector::Dqe(path.to_string()),
            });
            variables_reference = key.as_var_ref();
            r_value = format!("{value:?}");
        }
        ValueLayout::Structure(members, ..) => {
            named_variables = Some(members.len() as i64);
            r_value = format!("{value:?}");
            let key = registry.insert_unstable(Var {
                scope,
                frame: frame_info,
                selector: Selector::Dqe(path.to_string()),
            });
            variables_reference = key.as_var_ref();
        }
        ValueLayout::PreRendered(render) => {
            r_value = render.to_string();
        }
        ValueLayout::Referential(ptr) => {
            let deref_path = &format!("*({path})");
            r_value = format!("{}", RelocatedAddress::from(ptr as usize));
            let key = registry.insert_unstable(Var {
                scope,
                frame: frame_info,
                selector: Selector::Dqe(deref_path.to_string()),
            });
            variables_reference = key.as_var_ref();
        }
        ValueLayout::Wrapped(inner) => {
            let mut var = into_dap_var_repr(registry, scope, frame_info, path, name, inner);

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
