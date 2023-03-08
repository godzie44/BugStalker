mod btree;
mod hashbrown;

use crate::debugger::debugee::dwarf::r#type::{EvaluationContext, TypeIdentity};
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::specialization::btree::BTreeReflection;
use crate::debugger::variable::specialization::hashbrown::HashmapReflection;
use crate::debugger::variable::{
    ArrayVariable, AssumeError, ScalarVariable, StructVariable, SupportedScalar, VariableIR,
    VariableIdentity, VariableParser,
};
use crate::{debugger, weak_error};
use anyhow::Context;
use anyhow::{anyhow, bail};
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use itertools::Itertools;
use std::collections::HashMap;

#[derive(Clone)]
pub struct VecVariable {
    pub structure: StructVariable,
}

#[derive(Clone)]
pub struct StringVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

#[derive(Clone)]
pub struct HashMapVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub kv_items: Vec<(VariableIR, VariableIR)>,
}

#[derive(Clone)]
pub struct HashSetVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub items: Vec<VariableIR>,
}

#[derive(Clone)]
pub struct StrVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

#[derive(Clone)]
pub struct TlsVariable {
    pub identity: VariableIdentity,
    pub inner_value: Option<Box<VariableIR>>,
    pub inner_type: Option<String>,
}

#[derive(Clone)]
pub enum SpecializedVariableIR {
    Vector {
        vec: Option<VecVariable>,
        original: StructVariable,
    },
    VecDeque {
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
    BTreeMap {
        map: Option<HashMapVariable>,
        original: StructVariable,
    },
    BTreeSet {
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

pub struct VariableParserExtension<'a> {
    parser: &'a VariableParser<'a>,
}

impl<'a> VariableParserExtension<'a> {
    pub fn new(parser: &'a VariableParser) -> Self {
        Self { parser }
    }

    pub fn parse_str(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Str {
            string: weak_error!(self
                .parse_str_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("&str interpretation")),
            original: structure,
        }
    }

    fn parse_str_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<StrVariable> {
        let len = ir.assume_field_as_scalar_number("length")?;
        let data_ptr = ir.assume_field_as_pointer("data_ptr")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)
            .map(Bytes::from)?;

        Ok(StrVariable {
            identity: ir.identity().clone(),
            value: String::from_utf8(data.to_vec())?,
        })
    }

    pub fn parse_string(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::String {
            string: weak_error!(self
                .parse_string_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("string interpretation")),
            original: structure,
        }
    }

    fn parse_string_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<StringVariable> {
        let len = ir.assume_field_as_scalar_number("len")?;
        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)?;

        Ok(StringVariable {
            identity: ir.identity().clone(),
            value: String::from_utf8(data)?,
        })
    }

    pub fn parse_vector(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Vector {
            vec: weak_error!(self
                .parse_vector_inner(eval_ctx, VariableIR::Struct(structure.clone()), type_params)
                .context("vec interpretation")),
            original: structure,
        }
    }

    fn parse_vector_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> anyhow::Result<VecVariable> {
        let inner_type = type_params
            .get("T")
            .ok_or_else(|| anyhow!("template parameter `T`"))?
            .ok_or_else(|| anyhow!("unreachable: template param die without type"))?;
        let len = ir.assume_field_as_scalar_number("len")?;
        let cap = ir.assume_field_as_scalar_number("cap")?;

        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let el_type_size = self
            .parser
            .r#type
            .type_size_in_bytes(eval_ctx, inner_type)
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
                self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                    Some(data.slice_ref(chunk)),
                    inner_type,
                )
            })
            .collect::<Vec<_>>();

        Ok(VecVariable {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_name: Some(ir.r#type().to_owned()),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        identity: VariableIdentity::no_namespace(Some("buf".to_owned())),
                        type_name: self
                            .parser
                            .r#type
                            .type_name(inner_type)
                            .map(|tp| format!("[{tp}]")),
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

    pub fn parse_tls(
        &self,
        structure: StructVariable,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Tls {
            tls_var: weak_error!(self
                .parse_tls_inner(VariableIR::Struct(structure.clone()), type_params)
                .context("tls interpretation")),
            original: structure,
        }
    }

    fn parse_tls_inner(
        &self,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> anyhow::Result<TlsVariable> {
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

            return Ok(TlsVariable {
                identity: VariableIdentity::no_namespace(name),
                inner_value: tls_value,
                inner_type: self.parser.r#type.type_name(inner_type),
            });
        }

        bail!(AssumeError::IncompleteInterp(
            "expect tls inner value is option"
        ))
    }

    pub fn parse_hashmap(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::HashMap {
            map: weak_error!(self
                .parse_hashmap_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("hashmap interpretation")),
            original: structure,
        }
    }

    pub fn parse_hashmap_inner(
        &self,
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
            .ok_or_else(|| anyhow!("unknown hashmap bucket type"))?;
        let kv_size = self
            .parser
            .r#type
            .type_size_in_bytes(eval_ctx, kv_type)
            .ok_or_else(|| anyhow!("unknown hashmap bucket size"))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.pid)?;
        let kv_items = iterator
            .map_err(anyhow::Error::from)
            .filter_map(|bucket| {
                let data = bucket.read(eval_ctx.pid);
                let tuple = self.parser.parse_inner(
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

    pub fn parse_hashset(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::HashSet {
            set: weak_error!(self
                .parse_hashset_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("hashset interpretation")),
            original: structure,
        }
    }

    pub fn parse_hashset_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<HashSetVariable> {
        let ctrl = ir.assume_field_as_pointer("pointer")?;
        let bucket_mask = ir.assume_field_as_scalar_number("bucket_mask")?;

        let table = ir.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or_else(|| anyhow!("hashset bucket type not found"))?
            .ok_or_else(|| anyhow!("unknown hashset bucket type"))?;
        let kv_size = self
            .parser
            .r#type
            .type_size_in_bytes(eval_ctx, kv_type)
            .ok_or_else(|| anyhow!("unknown hashset bucket size"))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.pid)?;
        let items = iterator
            .map_err(anyhow::Error::from)
            .filter_map(|bucket| {
                let data = bucket.read(eval_ctx.pid);

                let tuple = self.parser.parse_inner(
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

    pub fn parse_btree_map(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
        identity: TypeIdentity,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::BTreeMap {
            map: weak_error!(self
                .parse_btree_map_inner(
                    eval_ctx,
                    VariableIR::Struct(structure.clone()),
                    identity,
                    type_params
                )
                .context("BTreeMap interpretation")),
            original: structure,
        }
    }

    pub fn parse_btree_map_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        identity: TypeIdentity,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> anyhow::Result<HashMapVariable> {
        let height = ir.assume_field_as_scalar_number("height")?;
        let ptr = ir.assume_field_as_pointer("pointer")?;

        let k_type = type_params
            .get("K")
            .ok_or_else(|| anyhow!("btree map bucket type not found"))?
            .ok_or_else(|| anyhow!("unknown BTreeMap bucket type"))?;
        let v_type = type_params
            .get("V")
            .ok_or_else(|| anyhow!("btree map bucket type not found"))?
            .ok_or_else(|| anyhow!("unknown BTreeMap bucket type"))?;

        let reflection = BTreeReflection::new(
            self.parser.r#type,
            ptr,
            height as usize,
            identity,
            k_type,
            v_type,
        )?;
        let iterator = reflection.iter(eval_ctx)?;
        let kv_items = iterator
            .map_err(anyhow::Error::from)
            .map(|(k, v)| {
                let key = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("k".to_string())),
                    Some(Bytes::from(k)),
                    k_type,
                );

                let value = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("v".to_string())),
                    Some(Bytes::from(v)),
                    v_type,
                );

                Ok((key, value))
            })
            .collect::<Vec<_>>()?;

        Ok(HashMapVariable {
            identity: ir.identity().clone(),
            type_name: Some(ir.r#type().to_owned()),
            kv_items,
        })
    }

    pub fn parse_btree_set(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::BTreeSet {
            set: weak_error!(self
                .parse_btree_set_inner(VariableIR::Struct(structure.clone()))
                .context("BTreeSet interpretation")),
            original: structure,
        }
    }

    pub fn parse_btree_set_inner(&self, ir: VariableIR) -> anyhow::Result<HashSetVariable> {
        let inner_map = ir
            .bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Specialized(SpecializedVariableIR::BTreeMap {
                    map: Some(ref map),
                    ..
                }) = child
                {
                    if map.identity.name.as_deref() == Some("map") {
                        return Some(map.clone());
                    }
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("BTreeMap"))?;

        Ok(HashSetVariable {
            identity: ir.identity().clone(),
            type_name: Some(ir.r#type().to_owned()),
            items: inner_map.kv_items.into_iter().map(|(k, _)| k).collect(),
        })
    }

    pub fn parse_vec_dequeue(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::VecDeque {
            vec: weak_error!(self
                .parse_vec_dequeue_inner(
                    eval_ctx,
                    VariableIR::Struct(structure.clone()),
                    type_params
                )
                .context("VeqDequeue interpretation")),
            original: structure,
        }
    }

    pub fn parse_vec_dequeue_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeIdentity>>,
    ) -> anyhow::Result<VecVariable> {
        let inner_type = type_params
            .get("T")
            .ok_or_else(|| anyhow!("template parameter `T`"))?
            .ok_or_else(|| anyhow!("unreachable: template param die without type"))?;
        let len = ir.assume_field_as_scalar_number("len")?;
        let cap = ir.assume_field_as_scalar_number("cap")?;
        let head = ir.assume_field_as_scalar_number("head")?;

        let wrapped_start = if head >= cap { head - cap } else { head };
        let head_len = cap - wrapped_start;

        let slice_ranges = if head_len >= len {
            // we know that `len + wrapped_start <= self.capacity <= usize::MAX`, so this addition can't overflow
            (wrapped_start..wrapped_start + len, 0..0)
        } else {
            // can't overflow because of the if condition
            let tail_len = len - head_len;
            (wrapped_start..cap, 0..tail_len)
        };

        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let el_type_size = self
            .parser
            .r#type
            .type_size_in_bytes(eval_ctx, inner_type)
            .ok_or_else(|| anyhow!("unknown element size"))? as usize;

        let data = debugger::read_memory_by_pid(
            eval_ctx.pid,
            data_ptr as usize,
            cap as usize * el_type_size,
        )
        .map(Bytes::from)?;

        let items = slice_ranges
            .0
            .into_iter()
            .chain(slice_ranges.1.into_iter())
            .enumerate()
            .map(|(i, real_idx)| {
                let real_idx = real_idx as usize;
                let el_data = &data[real_idx * el_type_size..(real_idx + 1) * el_type_size];
                self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                    Some(data.slice_ref(el_data)),
                    inner_type,
                )
            })
            .collect::<Vec<_>>();

        Ok(VecVariable {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_name: Some(ir.r#type().to_owned()),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        identity: VariableIdentity::no_namespace(Some("buf".to_owned())),
                        type_name: self
                            .parser
                            .r#type
                            .type_name(inner_type)
                            .map(|tp| format!("[{tp}]")),
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
