mod hashbrown;

use crate::debugger::debugee::dwarf::r#type::EvaluationContext;
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::specialization::hashbrown::HashmapReflection;
use crate::debugger::variable::{
    ArrayVariable, AssumeError, ScalarVariable, StructVariable, SupportedScalar, VariableIR,
    VariableIdentity,
};
use crate::debugger::TypeDeclaration;
use crate::{debugger, weak_error};
use anyhow::{anyhow, bail};
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
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
                type_params: type_params.clone(),
            },
        })
    }
}

#[derive(Clone)]
pub struct StringVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

impl StringVariable {
    pub fn from_struct_ir(eval_ctx: &EvaluationContext, ir: VariableIR) -> anyhow::Result<Self> {
        let len = ir.assume_field_as_scalar_number("len")?;
        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)?;

        Ok(Self {
            identity: ir.identity().clone(),
            value: String::from_utf8(data)?,
        })
    }
}

#[derive(Clone)]
pub struct HashMapVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub kv_items: Vec<(VariableIR, VariableIR)>,
}

impl HashMapVariable {
    pub fn from_struct_ir(
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<HashMapVariable> {
        let ctrl = ir.assume_field_as_pointer("pointer")?;
        let bucket_mask = ir.assume_field_as_scalar_number("bucket_mask")?;

        let table = ir.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or_else(|| anyhow!("hashmap bucket type not found"))?
            .as_ref();
        let kv_size = kv_type
            .map(|t| t.size_in_bytes(eval_ctx))
            .unwrap_or_default()
            .ok_or_else(|| anyhow!("unknown hashmap bucket size"))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.pid)?;
        let kv_items = iterator
            .map_err(anyhow::Error::from)
            .filter_map(|bucket| {
                let data = bucket.read(eval_ctx.pid);

                let tuple = VariableIR::new(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("kv".to_string())),
                    weak_error!(data).map(Bytes::from),
                    kv_type,
                );

                if let VariableIR::Struct(mut tuple) = tuple {
                    if tuple.members.len() == 2 {
                        let v = tuple.members.pop();
                        let k = tuple.members.pop();
                        return Ok(Some((k.unwrap(), v.unwrap())));
                    }
                }

                Err(anyhow!("unexpected bucket type"))
            })
            .collect()?;

        Ok(HashMapVariable {
            identity: ir.identity().clone(),
            type_name: Some(ir.r#type().to_owned()),
            kv_items,
        })
    }
}

#[derive(Clone)]
pub struct HashSetVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub items: Vec<VariableIR>,
}

impl HashSetVariable {
    pub fn from_struct_ir(
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<HashSetVariable> {
        let ctrl = ir.assume_field_as_pointer("pointer")?;
        let bucket_mask = ir.assume_field_as_scalar_number("bucket_mask")?;

        let table = ir.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or_else(|| anyhow!("hashmap bucket type not found"))?
            .as_ref();
        let kv_size = kv_type
            .map(|t| t.size_in_bytes(eval_ctx))
            .unwrap_or_default()
            .ok_or_else(|| anyhow!("unknown hashmap bucket size"))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.pid)?;
        let items = iterator
            .map_err(anyhow::Error::from)
            .filter_map(|bucket| {
                let data = bucket.read(eval_ctx.pid);

                let tuple = VariableIR::new(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("kv".to_string())),
                    weak_error!(data).map(Bytes::from),
                    kv_type,
                );

                if let VariableIR::Struct(mut tuple) = tuple {
                    if tuple.members.len() == 2 {
                        let _ = tuple.members.pop();
                        let k = tuple.members.pop().unwrap();
                        return Ok(Some(k));
                    }
                }

                Err(anyhow!("unexpected bucket type"))
            })
            .collect()?;

        Ok(HashSetVariable {
            identity: ir.identity().clone(),
            type_name: Some(ir.r#type().to_owned()),
            items,
        })
    }
}

#[derive(Clone)]
pub struct StrVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

impl StrVariable {
    pub fn from_struct_ir(eval_ctx: &EvaluationContext, ir: VariableIR) -> anyhow::Result<Self> {
        let len = ir.assume_field_as_scalar_number("length")?;
        let data_ptr = ir.assume_field_as_pointer("data_ptr")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)
            .map(Bytes::from)?;

        Ok(Self {
            identity: ir.identity().clone(),
            value: String::from_utf8(data.to_vec())?,
        })
    }
}

#[derive(Clone)]
pub struct TlsVariable {
    pub identity: VariableIdentity,
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
                identity: VariableIdentity::no_namespace(name),
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
    HashMap {
        map: Option<HashMapVariable>,
        original: StructVariable,
    },
    HashSet {
        set: Option<HashSetVariable>,
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
