use crate::debugger;
use crate::debugger::debugee::dwarf::r#type::EvaluationContext;
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::{
    ArrayVariable, ScalarVariable, StructVariable, SupportedScalar, VariableIR,
};
use crate::debugger::TypeDeclaration;
use anyhow::anyhow;
use bytes::Bytes;
use std::collections::HashMap;

#[derive(Clone)]
pub struct VecVariable {
    pub(super) structure: StructVariable,
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
                    Some(format!("{}", i as i64)),
                    Some(data.slice_ref(chunk)),
                    Some(inner_type),
                )
            })
            .collect::<Vec<_>>();

        Ok(Self {
            structure: StructVariable {
                name: Some(ir.name().to_owned()),
                type_name: Some(ir.r#type().to_owned()),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        name: Some("buf".to_owned()),
                        type_name: Some("".to_owned()),
                        items: Some(items),
                    }),
                    VariableIR::Scalar(ScalarVariable {
                        name: Some("cap".to_owned()),
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
    pub(super) name: Option<String>,
    pub(super) value: String,
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
    pub(super) name: Option<String>,
    pub(super) value: String,
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
}
