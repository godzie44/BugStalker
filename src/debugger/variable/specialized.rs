use crate::debugger;
use crate::debugger::debugee::dwarf::r#type::EvaluationContext;
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::{
    ArrayVariable, AssumeError, ScalarVariable, StructVariable, SupportedScalar, VariableIR,
    VariableIdentity,
};
use crate::debugger::TypeDeclaration;
use anyhow::{anyhow, bail};
use bytes::Bytes;
use itertools::Itertools;
use std::collections::HashMap;

#[derive(Clone)]
pub struct VecVariable {
    pub structure: StructVariable,
}

impl VecVariable {
    pub fn from_struct_ir(
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeDeclaration>>,
    ) -> anyhow::Result<Self> {
        let inner_type = type_params
            .get("T")
            .ok_or_else(|| anyhow!("template parameter `T`"))?
            .as_ref()
            .ok_or_else(|| anyhow!("unreachable: template param die without type"))?;
        let len = ir.assume_field_as_scalar_number("len")?;
        let cap = ir.assume_field_as_scalar_number("cap")?;

        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let el_type_size = inner_type
            .size_in_bytes(eval_ctx)
            .ok_or_else(|| anyhow!("unknown element size"))?;

        let data = debugger::read_memory_by_pid(
            eval_ctx.pid,
            data_ptr as usize,
            len as usize * el_type_size as usize,
        )
        .map(Bytes::from)?;

        let items = data
            .chunks(el_type_size as usize)
            .enumerate()
            .map(|(i, chunk)| {
                VariableIR::new(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                    Some(data.slice_ref(chunk)),
                    Some(inner_type),
                )
            })
            .collect::<Vec<_>>();

        Ok(Self {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_name: Some(ir.r#type().to_owned()),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        identity: VariableIdentity::no_namespace(Some("buf".to_owned())),
                        type_name: inner_type.name().map(|tp| format!("[{tp}]")),
                        items: Some(items),
                    }),
                    VariableIR::Scalar(ScalarVariable {
                        identity: VariableIdentity::no_namespace(Some("cap".to_owned())),
                        type_name: Some("usize".to_owned()),
                        value: Some(SupportedScalar::Usize(cap as usize)),
                    }),
                ],
            },
        })
    }
}

#[derive(Clone)]
pub struct StringVariable {
    pub name: Option<String>,
    pub value: String,
}

impl StringVariable {
    pub fn from_struct_ir(eval_ctx: &EvaluationContext, ir: VariableIR) -> anyhow::Result<Self> {
        let len = ir.assume_field_as_scalar_number("len")?;
        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)
            .map(Bytes::from)?;

        Ok(Self {
            name: Some(ir.name().to_owned()),
            value: String::from_utf8(data.to_vec())?,
        })
    }
}

#[allow(unused)]
#[derive(Clone)]
pub struct HashMapVariable {
    pub(super) name: Option<String>,
    pub(super) type_name: Option<String>,
    pub(super) kv_items: Vec<(VariableIR, VariableIR)>,
}

impl HashMapVariable {
    // todo
}

#[derive(Clone)]
pub struct StrVariable {
    pub name: Option<String>,
    pub value: String,
}

impl StrVariable {
    pub fn from_struct_ir(eval_ctx: &EvaluationContext, ir: VariableIR) -> anyhow::Result<Self> {
        let len = ir.assume_field_as_scalar_number("length")?;
        let data_ptr = ir.assume_field_as_pointer("data_ptr")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)
            .map(Bytes::from)?;

        Ok(Self {
            name: Some(ir.name().to_owned()),
            value: String::from_utf8(data.to_vec())?,
        })
    }
}

#[derive(Clone)]
pub struct TlsVariable {
    pub name: Option<String>,
    pub inner_value: Option<Box<VariableIR>>,
    pub inner_type: Option<String>,
}

impl TlsVariable {
    pub fn from_struct_ir(
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeDeclaration>>,
    ) -> anyhow::Result<Self> {
        // we assume that tls variable name represent in dwarf
        // as namespace flowed before "__getit" namespace
        let namespace = &ir.identity().namespace;
        let name = namespace
            .iter()
            .find_position(|&ns| ns == "__getit")
            .map(|(pos, _)| namespace[pos - 1].clone());

        let inner_type = type_params
            .get("T")
            .ok_or_else(|| anyhow!("template parameter `T`"))?
            .as_ref()
            .ok_or_else(|| anyhow!("unreachable: template param die without type"))?;

        let inner = ir
            .bfs_iterator()
            .find(|child| child.name() == "inner")
            .ok_or(AssumeError::FieldNotFound("inner"))?;
        let inner_option = inner.assume_field_as_rust_enum("value")?;
        let inner_value = inner_option
            .value
            .ok_or(AssumeError::IncompleteInterp(""))?;

        // we assume that dwarf representation of tls variable contains ::Option
        if let VariableIR::Struct(opt_variant) = inner_value.as_ref() {
            let tls_value = if opt_variant.type_name == Some("None".to_string()) {
                None
            } else {
                Some(Box::new(
                    inner_value
                        .bfs_iterator()
                        .find(|child| child.name() == "0")
                        .ok_or(AssumeError::FieldNotFound("0"))?
                        .clone(),
                ))
            };

            return Ok(Self {
                name,
                inner_value: tls_value,
                inner_type: inner_type.name(),
            });
        }

        bail!(AssumeError::IncompleteInterp(
            "expect tls inner value is option"
        ))
    }
}

#[derive(Clone)]
pub enum SpecializedVariableIR {
    Vector {
        vec: Option<VecVariable>,
        original: StructVariable,
    },
    String {
        string: Option<StringVariable>,
        original: StructVariable,
    },
    Str {
        string: Option<StrVariable>,
        original: StructVariable,
    },
    Tls {
        tls_var: Option<TlsVariable>,
        original: StructVariable,
    },
}
