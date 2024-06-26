mod btree;
mod hashbrown;

use crate::debugger::debugee::dwarf::r#type::{EvaluationContext, TypeId, TypeIdentity};
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::select::ObjectBinaryRepr;
use crate::debugger::variable::specialization::btree::BTreeReflection;
use crate::debugger::variable::specialization::hashbrown::HashmapReflection;
use crate::debugger::variable::AssumeError::{
    TypeParameterNotFound, TypeParameterTypeNotFound, UnexpectedType,
};
use crate::debugger::variable::ParsingError::Assume;
use crate::debugger::variable::{
    ArrayItem, ArrayVariable, AssumeError, FieldOrIndex, Member, ParsingError, PointerVariable,
    ScalarVariable, StructVariable, SupportedScalar, VariableIR, VariableIdentity, VariableParser,
};
use crate::{debugger, version_switch, weak_error};
use anyhow::Context;
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use itertools::Itertools;
use log::warn;
use std::collections::HashMap;
use AssumeError::{FieldNotFound, IncompleteInterp, UnknownSize};

/// During program execution, the debugger may encounter uninitialized variables.
/// For example look at this code:
/// ```rust
///    let res: Result<(), String> = Ok(());
///     if let Err(e) = res {
///         unreachable!();
///     }
/// ```
///
/// if stop debugger at line 2 and and consider a variable `e` - capacity of this vector
/// may be over 9000, this is obviously not the size that user expect.
/// Therefore, artificial restrictions on size and capacity are introduced. This behavior may be
/// changed in the future.
const LEN_GUARD: i64 = 10_000;
const CAP_GUARD: i64 = 10_000;

fn guard_len(len: i64) -> i64 {
    if len > LEN_GUARD {
        LEN_GUARD
    } else {
        len
    }
}

fn guard_cap(cap: i64) -> i64 {
    if cap > CAP_GUARD {
        CAP_GUARD
    } else {
        cap
    }
}

#[derive(Clone, PartialEq)]
pub struct VecVariable {
    pub structure: StructVariable,
}

impl VecVariable {
    pub fn slice(&mut self, left: Option<usize>, right: Option<usize>) {
        debug_assert!(matches!(
            self.structure.members.get_mut(0).map(|m| &m.value),
            Some(VariableIR::Array(_))
        ));

        if let Some(Member {
            value: VariableIR::Array(array),
            ..
        }) = self.structure.members.get_mut(0)
        {
            array.slice(left, right);
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct StringVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

#[derive(Clone, PartialEq)]
pub struct HashMapVariable {
    pub identity: VariableIdentity,
    pub type_ident: TypeIdentity,
    pub kv_items: Vec<(VariableIR, VariableIR)>,
}

#[derive(Clone, PartialEq)]
pub struct HashSetVariable {
    pub identity: VariableIdentity,
    pub type_ident: TypeIdentity,
    pub items: Vec<VariableIR>,
}

#[derive(Clone, PartialEq)]
pub struct StrVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

#[derive(Clone, PartialEq)]
pub struct TlsVariable {
    pub identity: VariableIdentity,
    pub inner_value: Option<Box<VariableIR>>,
    pub inner_type: TypeIdentity,
}

#[derive(Clone, PartialEq)]
pub enum SpecializedVariableIR {
    Vector(VecVariable),
    VecDeque(VecVariable),
    HashMap(HashMapVariable),
    HashSet(HashSetVariable),
    BTreeMap(HashMapVariable),
    BTreeSet(HashSetVariable),
    String(StringVariable),
    Str(StrVariable),
    Tls(TlsVariable),
    Cell(Box<VariableIR>),
    RefCell(Box<VariableIR>),
    Rc(PointerVariable),
    Arc(PointerVariable),
    Uuid([u8; 16]),
    SystemTime((i64, u32)),
    Instant((i64, u32)),
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
        structure: &StructVariable,
    ) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_str_inner(eval_ctx, VariableIR::Struct(structure.clone()))
            .context("&str interpretation"))
        .map(SpecializedVariableIR::Str)
    }

    fn parse_str_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> Result<StrVariable, ParsingError> {
        let len = ir.assume_field_as_scalar_number("length")?;
        let len = guard_len(len);

        let data_ptr = ir.assume_field_as_pointer("data_ptr")?;

        let data = debugger::read_memory_by_pid(
            eval_ctx.expl_ctx.pid_on_focus(),
            data_ptr as usize,
            len as usize,
        )
        .map(Bytes::from)?;

        Ok(StrVariable {
            identity: ir.identity().clone(),
            value: String::from_utf8(data.to_vec()).map_err(AssumeError::from)?,
        })
    }

    pub fn parse_string(
        &self,
        eval_ctx: &EvaluationContext,
        structure: &StructVariable,
    ) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_string_inner(eval_ctx, VariableIR::Struct(structure.clone()))
            .context("String interpretation"))
        .map(SpecializedVariableIR::String)
    }

    fn parse_string_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> Result<StringVariable, ParsingError> {
        let len = ir.assume_field_as_scalar_number("len")?;
        let len = guard_len(len);

        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let data = debugger::read_memory_by_pid(
            eval_ctx.expl_ctx.pid_on_focus(),
            data_ptr as usize,
            len as usize,
        )?;

        Ok(StringVariable {
            identity: ir.identity().clone(),
            value: String::from_utf8(data).map_err(AssumeError::from)?,
        })
    }

    pub fn parse_vector(
        &self,
        eval_ctx: &EvaluationContext,
        structure: &StructVariable,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_vector_inner(eval_ctx, VariableIR::Struct(structure.clone()), type_params)
            .context("Vec<T> interpretation"))
        .map(SpecializedVariableIR::Vector)
    }

    fn parse_vector_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<VecVariable, ParsingError> {
        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let len = ir.assume_field_as_scalar_number("len")?;
        let len = guard_len(len);

        let cap = extract_capacity(eval_ctx, &ir)? as i64;
        let cap = guard_cap(cap);

        let data_ptr = ir.assume_field_as_pointer("pointer")? as usize;

        let el_type = self.parser.r#type;
        let el_type_size = el_type
            .type_size_in_bytes(eval_ctx, inner_type)
            .ok_or(UnknownSize(el_type.identity(inner_type)))? as usize;

        let raw_data = debugger::read_memory_by_pid(
            eval_ctx.expl_ctx.pid_on_focus(),
            data_ptr,
            len as usize * el_type_size,
        )
        .map(Bytes::from)?;

        let (mut bytes_chunks, mut empty_chunks);
        let raw_items_iter: &mut dyn Iterator<Item = (usize, &[u8])> = if el_type_size != 0 {
            bytes_chunks = raw_data.chunks(el_type_size).enumerate();
            &mut bytes_chunks
        } else {
            // if an item type is zst
            let v: Vec<&[u8]> = vec![&[]; len as usize];
            empty_chunks = v.into_iter().enumerate();
            &mut empty_chunks
        };

        let items = raw_items_iter
            .filter_map(|(i, chunk)| {
                let data = ObjectBinaryRepr {
                    raw_data: raw_data.slice_ref(chunk),
                    address: Some(data_ptr + (i * el_type_size)),
                    size: el_type_size,
                };
                ArrayItem {
                    index: i as i64,
                    value: self.parser.parse_inner(
                        eval_ctx,
                        VariableIdentity::default(),
                        Some(data),
                        inner_type,
                    ),
                }
            })
            .collect::<Vec<_>>();

        Ok(VecVariable {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_id: None,
                type_ident: ir.r#type().clone(),
                members: vec![
                    Member {
                        field_name: Some("buf".to_owned()),
                        value: VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::default(),
                            type_id: None,
                            type_ident: self.parser.r#type.identity(inner_type).as_array_type(),
                            items: Some(items),
                            // set to `None` because the address operator unavailable for spec vars
                            raw_address: None,
                        }),
                    },
                    Member {
                        field_name: Some("cap".to_owned()),
                        value: VariableIR::Scalar(ScalarVariable {
                            identity: VariableIdentity::default(),
                            type_id: None,
                            type_ident: TypeIdentity::no_namespace("usize"),
                            value: Some(SupportedScalar::Usize(cap as usize)),
                            // set to `None` because the address operator unavailable for spec vars
                            raw_address: None,
                        }),
                    },
                ],
                type_params: type_params.clone(),
                // set to `None` because the address operator unavailable for spec vars
                raw_address: None,
            },
        })
    }

    pub fn parse_tls_old(
        &self,
        structure: &StructVariable,
        type_params: &HashMap<String, Option<TypeId>>,
        is_const_initialized: bool,
    ) -> Option<SpecializedVariableIR> {
        let tls_var = if is_const_initialized {
            self.parse_const_init_tls_inner(VariableIR::Struct(structure.clone()), type_params)
        } else {
            self.parse_tls_inner_old(VariableIR::Struct(structure.clone()), type_params)
        };

        weak_error!(tls_var.context("TLS variable interpretation")).map(SpecializedVariableIR::Tls)
    }

    fn parse_const_init_tls_inner(
        &self,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<TlsVariable, ParsingError> {
        // we assume that tls variable name represents in dwarf
        // as namespace flowed before "__getit" namespace
        let namespace = &ir.identity().namespace;
        let name = namespace
            .iter()
            .find_position(|&ns| ns == "__getit")
            .map(|(pos, _)| namespace[pos - 1].clone());

        let value_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let value = ir.field("value");
        Ok(TlsVariable {
            identity: VariableIdentity::no_namespace(name),
            inner_value: value.map(Box::new),
            inner_type: self.parser.r#type.identity(value_type),
        })
    }

    fn parse_tls_inner_old(
        &self,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<TlsVariable, ParsingError> {
        // we assume that tls variable name represents in dwarf
        // as namespace flowed before "__getit" namespace
        let namespace = &ir.identity().namespace;
        let name = namespace
            .iter()
            .find_position(|&ns| ns == "__getit")
            .map(|(pos, _)| namespace[pos - 1].clone());

        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;

        let inner = ir
            .bfs_iterator()
            .find_map(|(field, child)| {
                (field == FieldOrIndex::Field(Some("inner"))).then_some(child)
            })
            .ok_or(FieldNotFound("inner"))?;
        let inner_option = inner.assume_field_as_rust_enum("value")?;
        let inner_value = inner_option.value.ok_or(IncompleteInterp("value"))?;

        // we assume that DWARF representation of tls variable contains ::Option
        if let VariableIR::Struct(ref opt_variant) = inner_value.value {
            let tls_value = if opt_variant.type_ident.name() == Some("None") {
                None
            } else {
                Some(Box::new(
                    inner_value
                        .value
                        .bfs_iterator()
                        .find_map(|(field, child)| {
                            (field == FieldOrIndex::Field(Some("__0"))).then_some(child)
                        })
                        .ok_or(FieldNotFound("__0"))?
                        .clone(),
                ))
            };

            return Ok(TlsVariable {
                identity: VariableIdentity::no_namespace(name),
                inner_value: tls_value,
                inner_type: self.parser.r#type.identity(inner_type),
            });
        }

        Err(ParsingError::Assume(IncompleteInterp(
            "expect TLS inner value as option",
        )))
    }

    pub fn parse_tls(
        &self,
        structure: StructVariable,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Option<SpecializedVariableIR> {
        let tls_var = self
            .parse_tls_inner(VariableIR::Struct(structure.clone()), type_params)
            .context("TLS variable interpretation");

        let var = match tls_var {
            Ok(Some(var)) => Some(var),
            Ok(None) => return None,
            Err(e) => {
                let e = e.context("TLS variable interpretation");
                warn!(target: "debugger", "{:#}", e);
                None
            }
        };

        Some(SpecializedVariableIR::Tls {
            tls_var: var,
            original: structure,
        })
    }

    fn parse_tls_inner(
        &self,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<Option<TlsVariable>, ParsingError> {
        // we assume that tls variable name represents in dwarf
        // as namespace flowed before "__getit" namespace
        let namespace = &ir.identity().namespace;
        let name = namespace
            .iter()
            .find_position(|&ns| ns.contains("{constant#"))
            .map(|(pos, _)| namespace[pos - 1].clone());

        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;

        let state = ir
            .bfs_iterator()
            .find(|child| child.name() == "state")
            .ok_or(FieldNotFound("state"))?;

        let state = state.assume_field_as_rust_enum("value")?;
        if let Some(VariableIR::Struct(val)) = state.value.as_deref() {
            let tls_val = if val.identity.name.as_deref() == Some("Alive") {
                Some(Box::new(val.members[0].clone()))
            } else {
                return Ok(None);
            };

            return Ok(Some(TlsVariable {
                identity: VariableIdentity::no_namespace(name),
                inner_value: tls_val,
                inner_type: self.parser.r#type.identity(inner_type),
            }));
        };

        Err(ParsingError::Assume(IncompleteInterp(
            "expect TLS inner value as option",
        )))
    }

    pub fn parse_hashmap(
        &self,
        eval_ctx: &EvaluationContext,
        structure: &StructVariable,
    ) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_hashmap_inner(eval_ctx, VariableIR::Struct(structure.clone()))
            .context("HashMap<K, V> interpretation"))
        .map(SpecializedVariableIR::HashMap)
    }

    fn parse_hashmap_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> Result<HashMapVariable, ParsingError> {
        let ctrl = ir.assume_field_as_pointer("pointer")?;
        let bucket_mask = ir.assume_field_as_scalar_number("bucket_mask")?;

        let table = ir.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;

        let r#type = self.parser.r#type;
        let kv_size = r#type
            .type_size_in_bytes(eval_ctx, kv_type)
            .ok_or(UnknownSize(r#type.identity(kv_type)))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.expl_ctx.pid_on_focus())?;
        let kv_items = iterator
            .map_err(ParsingError::from)
            .filter_map(|bucket| {
                let raw_data = bucket.read(eval_ctx.expl_ctx.pid_on_focus());
                let data = weak_error!(raw_data).map(|d| ObjectBinaryRepr {
                    raw_data: Bytes::from(d),
                    address: Some(bucket.location()),
                    size: bucket.size(),
                });

                let tuple = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("kv".to_string())),
                    data,
                    kv_type,
                );

                if let Some(VariableIR::Struct(mut tuple)) = tuple {
                    if tuple.members.len() == 2 {
                        let v = tuple.members.pop();
                        let k = tuple.members.pop();
                        return Ok(Some((k.unwrap().value, v.unwrap().value)));
                    }
                }

                Err(Assume(UnexpectedType("hashmap bucket")))
            })
            .collect()?;

        Ok(HashMapVariable {
            identity: ir.identity().clone(),
            type_ident: ir.r#type().to_owned(),
            kv_items,
        })
    }

    pub fn parse_hashset(
        &self,
        eval_ctx: &EvaluationContext,
        structure: &StructVariable,
    ) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_hashset_inner(eval_ctx, VariableIR::Struct(structure.clone()))
            .context("HashSet<T> interpretation"))
        .map(SpecializedVariableIR::HashSet)
    }

    fn parse_hashset_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> Result<HashSetVariable, ParsingError> {
        let ctrl = ir.assume_field_as_pointer("pointer")?;
        let bucket_mask = ir.assume_field_as_scalar_number("bucket_mask")?;

        let table = ir.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let r#type = self.parser.r#type;
        let kv_size = self
            .parser
            .r#type
            .type_size_in_bytes(eval_ctx, kv_type)
            .ok_or_else(|| UnknownSize(r#type.identity(kv_type)))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.expl_ctx.pid_on_focus())?;
        let items = iterator
            .map_err(ParsingError::from)
            .filter_map(|bucket| {
                let raw_data = bucket.read(eval_ctx.expl_ctx.pid_on_focus());
                let data = weak_error!(raw_data).map(|d| ObjectBinaryRepr {
                    raw_data: Bytes::from(d),
                    address: Some(bucket.location()),
                    size: bucket.size(),
                });

                let tuple = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("kv".to_string())),
                    data,
                    kv_type,
                );

                if let Some(VariableIR::Struct(mut tuple)) = tuple {
                    if tuple.members.len() == 2 {
                        let _ = tuple.members.pop();
                        let k = tuple.members.pop().unwrap();
                        return Ok(Some(k.value));
                    }
                }

                Err(Assume(UnexpectedType("hashset bucket")))
            })
            .collect()?;

        Ok(HashSetVariable {
            identity: ir.identity().clone(),
            type_ident: ir.r#type().to_owned(),
            items,
        })
    }

    pub fn parse_btree_map(
        &self,
        eval_ctx: &EvaluationContext,
        structure: &StructVariable,
        identity: TypeId,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_btree_map_inner(
                eval_ctx,
                VariableIR::Struct(structure.clone()),
                identity,
                type_params
            )
            .context("BTreeMap<K, V> interpretation"))
        .map(SpecializedVariableIR::BTreeMap)
    }

    fn parse_btree_map_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        identity: TypeId,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<HashMapVariable, ParsingError> {
        let height = ir.assume_field_as_scalar_number("height")?;
        let ptr = ir.assume_field_as_pointer("pointer")?;

        let k_type = type_params
            .get("K")
            .ok_or(TypeParameterNotFound("K"))?
            .ok_or(TypeParameterTypeNotFound("K"))?;
        let v_type = type_params
            .get("V")
            .ok_or(TypeParameterNotFound("V"))?
            .ok_or(TypeParameterTypeNotFound("V"))?;

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
            .map_err(ParsingError::from)
            .map(|(k, v)| {
                let key =
                    self.parser
                        .parse_inner(eval_ctx, VariableIdentity::default(), Some(k), k_type);

                let value =
                    self.parser
                        .parse_inner(eval_ctx, VariableIdentity::default(), Some(v), v_type);

                Ok(Some((key, value)))
            })
            .collect::<Vec<_>>()?;

        Ok(HashMapVariable {
            identity: ir.identity().clone(),
            type_ident: ir.r#type().to_owned(),
            kv_items,
        })
    }

    pub fn parse_btree_set(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_btree_set_inner(VariableIR::Struct(structure.clone()))
            .context("BTreeSet interpretation"))
        .map(SpecializedVariableIR::BTreeSet)
    }

    fn parse_btree_set_inner(&self, ir: VariableIR) -> Result<HashSetVariable, ParsingError> {
        let inner_map = ir
            .bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let VariableIR::Specialized {
                    value: Some(SpecializedVariableIR::BTreeMap(ref map)),
                    ..
                } = child
                {
                    if field_or_idx == FieldOrIndex::Field(Some("map")) {
                        return Some(map.clone());
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("BTreeMap"))?;

        Ok(HashSetVariable {
            identity: ir.identity().clone(),
            type_ident: ir.r#type().to_owned(),
            items: inner_map.kv_items.into_iter().map(|(k, _)| k).collect(),
        })
    }

    pub fn parse_vec_dequeue(
        &self,
        eval_ctx: &EvaluationContext,
        structure: &StructVariable,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_vec_dequeue_inner(eval_ctx, VariableIR::Struct(structure.clone()), type_params)
            .context("VeqDequeue<T> interpretation"))
        .map(SpecializedVariableIR::VecDeque)
    }

    fn parse_vec_dequeue_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<VecVariable, ParsingError> {
        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let len = ir.assume_field_as_scalar_number("len")? as usize;
        let len = guard_len(len as i64) as usize;

        let r#type = self.parser.r#type;
        let el_type_size = r#type
            .type_size_in_bytes(eval_ctx, inner_type)
            .ok_or_else(|| UnknownSize(r#type.identity(inner_type)))?
            as usize;
        let cap = if el_type_size == 0 {
            usize::MAX
        } else {
            extract_capacity(eval_ctx, &ir)?
        };
        let head = ir.assume_field_as_scalar_number("head")? as usize;

        let wrapped_start = if head >= cap { head - cap } else { head };
        let head_len = cap - wrapped_start;

        let slice_ranges = if head_len >= len {
            (wrapped_start..wrapped_start + len, 0..0)
        } else {
            let tail_len = len - head_len;
            (wrapped_start..cap, 0..tail_len)
        };

        let data_ptr = ir.assume_field_as_pointer("pointer")? as usize;

        let data = debugger::read_memory_by_pid(
            eval_ctx.expl_ctx.pid_on_focus(),
            data_ptr,
            cap * el_type_size,
        )
        .map(Bytes::from)?;

        let items = slice_ranges
            .0
            .chain(slice_ranges.1)
            .enumerate()
            .filter_map(|(i, real_idx)| {
                let offset = real_idx * el_type_size;
                let el_raw_data = &data[offset..(real_idx + 1) * el_type_size];
                let el_data = ObjectBinaryRepr {
                    raw_data: data.slice_ref(el_raw_data),
                    address: Some(data_ptr + offset),
                    size: el_type_size,
                };

                ArrayItem {
                    index: i as i64,
                    value: self.parser.parse_inner(
                        eval_ctx,
                        VariableIdentity::default(),
                        Some(el_data),
                        inner_type,
                    ),
                }
            })
            .collect::<Vec<_>>();

        Ok(VecVariable {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_id: None,
                type_ident: ir.r#type().to_owned(),
                members: vec![
                    Member {
                        field_name: Some("buf".to_owned()),
                        value: VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::default(),
                            type_id: None,
                            type_ident: self.parser.r#type.identity(inner_type).as_array_type(),
                            items: Some(items),
                            // set to `None` because the address operator unavailable for spec vars
                            raw_address: None,
                        }),
                    },
                    Member {
                        field_name: Some("cap".to_owned()),
                        value: VariableIR::Scalar(ScalarVariable {
                            identity: VariableIdentity::default(),
                            type_id: None,
                            type_ident: TypeIdentity::no_namespace("usize"),
                            value: Some(SupportedScalar::Usize(if el_type_size == 0 {
                                0
                            } else {
                                cap
                            })),
                            // set to `None` because the address operator unavailable for spec vars
                            raw_address: None,
                        }),
                    },
                ],
                type_params: type_params.clone(),
                // set to `None` because the address operator unavailable for spec vars
                raw_address: None,
            },
        })
    }

    pub fn parse_cell(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_cell_inner(VariableIR::Struct(structure.clone()))
            .context("Cell<T> interpretation"))
        .map(Box::new)
        .map(SpecializedVariableIR::Cell)
    }

    fn parse_cell_inner(&self, ir: VariableIR) -> Result<VariableIR, ParsingError> {
        let unsafe_cell = ir.assume_field_as_struct("value")?;
        let member = unsafe_cell
            .members
            .first()
            .ok_or(IncompleteInterp("UnsafeCell"))?;
        Ok(member.value.clone())
    }

    pub fn parse_refcell(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_refcell_inner(VariableIR::Struct(structure.clone()))
            .context("RefCell<T> interpretation"))
        .map(Box::new)
        .map(SpecializedVariableIR::RefCell)
    }

    fn parse_refcell_inner(&self, ir: VariableIR) -> Result<VariableIR, ParsingError> {
        let borrow = ir
            .bfs_iterator()
            .find_map(|(_, child)| {
                if let VariableIR::Specialized {
                    value: Some(SpecializedVariableIR::Cell(val)),
                    ..
                } = child
                {
                    return Some(val.clone());
                }
                None
            })
            .ok_or(IncompleteInterp("Cell"))?;
        let VariableIR::Scalar(var) = *borrow else {
            return Err(IncompleteInterp("Cell").into());
        };
        let borrow = VariableIR::Scalar(var);

        let unsafe_cell = ir.assume_field_as_struct("value")?;
        let value = unsafe_cell
            .members
            .first()
            .ok_or(IncompleteInterp("UnsafeCell"))?;

        Ok(VariableIR::Struct(StructVariable {
            identity: ir.identity().clone(),
            type_id: None,
            type_ident: ir.r#type().to_owned(),
            members: vec![
                Member {
                    field_name: Some("borrow".to_string()),
                    value: borrow,
                },
                value.clone(),
            ],
            type_params: Default::default(),
            // set to `None` because the address operator unavailable for spec vars
            raw_address: None,
        }))
    }

    pub fn parse_rc(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_rc_inner(VariableIR::Struct(structure.clone()))
            .context("Rc<T> interpretation"))
        .map(SpecializedVariableIR::Rc)
    }

    fn parse_rc_inner(&self, ir: VariableIR) -> Result<PointerVariable, ParsingError> {
        Ok(ir
            .bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let VariableIR::Pointer(pointer) = child {
                    if field_or_idx == FieldOrIndex::Field(Some("pointer")) {
                        let mut new_pointer = pointer.clone();
                        new_pointer.identity = ir.identity().clone();
                        return Some(new_pointer);
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("rc"))?)
    }

    pub fn parse_arc(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_arc_inner(VariableIR::Struct(structure.clone()))
            .context("Arc<T> interpretation"))
        .map(SpecializedVariableIR::Arc)
    }

    fn parse_arc_inner(&self, ir: VariableIR) -> Result<PointerVariable, ParsingError> {
        Ok(ir
            .bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let VariableIR::Pointer(pointer) = child {
                    if field_or_idx == FieldOrIndex::Field(Some("pointer")) {
                        let mut new_pointer = pointer.clone();
                        new_pointer.identity = ir.identity().clone();
                        return Some(new_pointer);
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("Arc"))?)
    }

    pub fn parse_uuid(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_uuid_inner(&structure)
            .context("Uuid interpretation"))
        .map(SpecializedVariableIR::Uuid)
    }

    fn parse_uuid_inner(&self, structure: &StructVariable) -> Result<[u8; 16], ParsingError> {
        let member0 = structure.members.first().ok_or(FieldNotFound("member 0"))?;
        let VariableIR::Array(ref arr) = member0.value else {
            return Err(UnexpectedType("uuid struct member must be an array").into());
        };
        let items = arr
            .items
            .as_ref()
            .ok_or(AssumeError::NoData("uuid items"))?;
        if items.len() != 16 {
            return Err(AssumeError::UnexpectedType("uuid struct member must be [u8; 16]").into());
        }

        let mut bytes_repr = [0; 16];
        for (i, item) in items.iter().enumerate() {
            let VariableIR::Scalar(ScalarVariable {
                value: Some(SupportedScalar::U8(byte)),
                ..
            }) = item.value
            else {
                return Err(UnexpectedType("uuid struct member must be [u8; 16]").into());
            };
            bytes_repr[i] = byte;
        }

        Ok(bytes_repr)
    }

    fn parse_timespec(&self, timespec: &StructVariable) -> Result<(i64, u32), ParsingError> {
        let &[Member {
            value: VariableIR::Scalar(secs),
            ..
        }, Member {
            value: VariableIR::Struct(n_secs),
            ..
        }] = &timespec.members.as_slice()
        else {
            let err = "`Timespec` should contains secs and n_secs fields";
            return Err(UnexpectedType(err).into());
        };

        let &[Member {
            value: VariableIR::Scalar(n_secs),
            ..
        }] = &n_secs.members.as_slice()
        else {
            let err = "`Nanoseconds` should contains u32 field";
            return Err(UnexpectedType(err).into());
        };

        let secs = secs
            .try_as_number()
            .ok_or(UnexpectedType("`Timespec::tv_sec` not an int"))?;
        let n_secs = n_secs
            .try_as_number()
            .ok_or(UnexpectedType("Timespec::tv_nsec` not an int"))? as u32;

        Ok((secs, n_secs))
    }

    pub fn parse_sys_time(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_sys_time_inner(&structure)
            .context("SystemTime interpretation"))
        .map(SpecializedVariableIR::SystemTime)
    }

    fn parse_sys_time_inner(&self, structure: &StructVariable) -> Result<(i64, u32), ParsingError> {
        let &[Member {
            value: VariableIR::Struct(time_instant),
            ..
        }] = &structure.members.as_slice()
        else {
            let err = "`std::time::SystemTime` should contains a `time::SystemTime` field";
            return Err(UnexpectedType(err).into());
        };

        let &[Member {
            value: VariableIR::Struct(timespec),
            ..
        }] = &time_instant.members.as_slice()
        else {
            let err = "`time::SystemTime` should contains a `Timespec` field";
            return Err(UnexpectedType(err).into());
        };

        self.parse_timespec(timespec)
    }

    pub fn parse_instant(&self, structure: &StructVariable) -> Option<SpecializedVariableIR> {
        weak_error!(self
            .parse_instant_inner(&structure)
            .context("Instant interpretation"))
        .map(SpecializedVariableIR::Instant)
    }

    fn parse_instant_inner(&self, structure: &StructVariable) -> Result<(i64, u32), ParsingError> {
        let &[Member {
            value: VariableIR::Struct(time_instant),
            ..
        }] = &structure.members.as_slice()
        else {
            let err = "`std::time::Instant` should contains a `time::Instant` field";
            return Err(UnexpectedType(err).into());
        };

        let &[Member {
            value: VariableIR::Struct(timespec),
            ..
        }] = &time_instant.members.as_slice()
        else {
            let err = "`time::Instant` should contains a `Timespec` field";
            return Err(UnexpectedType(err).into());
        };

        self.parse_timespec(timespec)
    }
}

fn extract_capacity(eval_ctx: &EvaluationContext, ir: &VariableIR) -> Result<usize, ParsingError> {
    let rust_version = eval_ctx
        .rustc_version()
        .ok_or(ParsingError::UnsupportedVersion)?;

    version_switch!(
                rust_version,
                (1, 0, 0) ..= (1, 75, u32::MAX) => ir.assume_field_as_scalar_number("cap")? as usize,
                (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => {
                        let cap_s = ir.assume_field_as_struct("cap")?;
                        let cap = &cap_s.members.first().ok_or(IncompleteInterp("Vec"))?.value;
                        if let VariableIR::Scalar(ScalarVariable {value: Some(SupportedScalar::Usize(cap)), ..}) = cap {
                            Ok(*cap)
                        } else {
                            Err(AssumeError::FieldNotANumber("cap"))
                        }?
                    },
                ).ok_or(ParsingError::UnsupportedVersion)
}
