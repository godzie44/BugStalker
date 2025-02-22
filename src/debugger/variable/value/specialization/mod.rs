use crate::debugger::debugee::dwarf::r#type::{TypeId, TypeIdentity};
use crate::debugger::variable::ObjectBinaryRepr;
use crate::debugger::variable::render::RenderValue;
use crate::debugger::variable::value::AssumeError::{
    TypeParameterNotFound, TypeParameterTypeNotFound, UnexpectedType,
};
use crate::debugger::variable::value::ParsingError::Assume;
use crate::debugger::variable::value::parser::{ParseContext, ValueParser};
use crate::debugger::variable::value::specialization::btree::BTreeReflection;
use crate::debugger::variable::value::specialization::hashbrown::HashmapReflection;
use crate::debugger::variable::value::{
    ArrayItem, ArrayValue, AssumeError, FieldOrIndex, Member, ParsingError, ScalarValue,
    SupportedScalar,
};
use crate::debugger::variable::value::{PointerValue, StructValue, Value};
use crate::{debugger, version_switch, weak_error};
use AssumeError::{FieldNotFound, IncompleteInterp, UnknownSize};
use anyhow::Context;
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use std::collections::HashMap;

mod btree;
mod hashbrown;

/// During program execution, the debugger may encounter uninitialized variables.
/// For example, look at this code:
/// ```rust
///    let res: Result<(), String> = Ok(());
///     if let Err(e) = res {
///         unreachable!();
///     }
/// ```
///
/// if stop debugger at line 2 and consider a variable `e` - capacity of this vector
/// may be over 9000, this is obviously not the size that user expects.
/// Therefore, artificial restrictions on size and capacity are introduced. This behavior may be
/// changed in the future.
const LEN_GUARD: i64 = 10_000;
const CAP_GUARD: i64 = 10_000;

fn guard_len(len: i64) -> i64 {
    if len > LEN_GUARD { LEN_GUARD } else { len }
}

fn guard_cap(cap: i64) -> i64 {
    if cap > CAP_GUARD { CAP_GUARD } else { cap }
}

#[derive(Clone, PartialEq)]
pub struct VecValue {
    pub structure: StructValue,
}

impl VecValue {
    pub fn slice(&mut self, left: Option<usize>, right: Option<usize>) {
        debug_assert!(matches!(
            self.structure.members.get_mut(0).map(|m| &m.value),
            Some(Value::Array(_))
        ));

        if let Some(Member {
            value: Value::Array(array),
            ..
        }) = self.structure.members.get_mut(0)
        {
            array.slice(left, right);
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct StringVariable {
    pub value: String,
}

#[derive(Clone, PartialEq)]
pub struct HashMapVariable {
    pub type_ident: TypeIdentity,
    pub kv_items: Vec<(Value, Value)>,
}

#[derive(Clone, PartialEq)]
pub struct HashSetVariable {
    pub type_ident: TypeIdentity,
    pub items: Vec<Value>,
}

#[derive(Clone, PartialEq)]
pub struct StrVariable {
    pub value: String,
}

#[derive(Clone, PartialEq)]
pub struct TlsVariable {
    pub inner_value: Option<Box<Value>>,
    pub inner_type: TypeIdentity,
}

#[derive(Clone, PartialEq)]
pub enum SpecializedValue {
    Vector(VecValue),
    VecDeque(VecValue),
    HashMap(HashMapVariable),
    HashSet(HashSetVariable),
    BTreeMap(HashMapVariable),
    BTreeSet(HashSetVariable),
    String(StringVariable),
    Str(StrVariable),
    Tls(TlsVariable),
    Cell(Box<Value>),
    RefCell(Box<Value>),
    Rc(PointerValue),
    Arc(PointerValue),
    Uuid([u8; 16]),
    SystemTime((i64, u32)),
    Instant((i64, u32)),
}

pub struct VariableParserExtension<'a> {
    parser: &'a ValueParser,
}

impl<'a> VariableParserExtension<'a> {
    pub fn new(parser: &'a ValueParser) -> Self {
        Self { parser }
    }

    pub fn parse_str(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
    ) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_str_inner(ctx, Value::Struct(structure.clone()))
                .context("&str interpretation")
        )
        .map(SpecializedValue::Str)
    }

    fn parse_str_inner(&self, ctx: &ParseContext, val: Value) -> Result<StrVariable, ParsingError> {
        let len = val.assume_field_as_scalar_number("length")?;
        let len = guard_len(len);

        let data_ptr = val.assume_field_as_pointer("data_ptr")?;

        let data = debugger::read_memory_by_pid(
            ctx.evaluation_context.expl_ctx.pid_on_focus(),
            data_ptr as usize,
            len as usize,
        )
        .map(Bytes::from)?;

        Ok(StrVariable {
            value: String::from_utf8(data.to_vec()).map_err(AssumeError::from)?,
        })
    }

    pub fn parse_string(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
    ) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_string_inner(ctx, Value::Struct(structure.clone()))
                .context("String interpretation")
        )
        .map(SpecializedValue::String)
    }

    fn parse_string_inner(
        &self,
        ctx: &ParseContext,
        val: Value,
    ) -> Result<StringVariable, ParsingError> {
        let len = val.assume_field_as_scalar_number("len")?;
        let len = guard_len(len);

        let data_ptr = val.assume_field_as_pointer("pointer")?;

        let data = debugger::read_memory_by_pid(
            ctx.evaluation_context.expl_ctx.pid_on_focus(),
            data_ptr as usize,
            len as usize,
        )?;

        Ok(StringVariable {
            value: String::from_utf8(data).map_err(AssumeError::from)?,
        })
    }

    pub fn parse_vector(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_vector_inner(ctx, Value::Struct(structure.clone()), type_params)
                .context("Vec<T> interpretation")
        )
        .map(SpecializedValue::Vector)
    }

    fn parse_vector_inner(
        &self,
        ctx: &ParseContext,
        val: Value,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<VecValue, ParsingError> {
        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let len = val.assume_field_as_scalar_number("len")?;
        let len = guard_len(len);

        let cap = extract_capacity(ctx, &val)? as i64;
        let cap = guard_cap(cap);

        let data_ptr = val.assume_field_as_pointer("pointer")? as usize;

        let el_type = ctx.type_graph;
        let el_type_size = el_type
            .type_size_in_bytes(ctx.evaluation_context, inner_type)
            .ok_or(UnknownSize(el_type.identity(inner_type)))? as usize;

        let raw_data = debugger::read_memory_by_pid(
            ctx.evaluation_context.expl_ctx.pid_on_focus(),
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
                Some(ArrayItem {
                    index: i as i64,
                    value: self.parser.parse_inner(ctx, Some(data), inner_type)?,
                })
            })
            .collect::<Vec<_>>();

        Ok(VecValue {
            structure: StructValue {
                type_id: None,
                type_ident: val.r#type().clone(),
                members: vec![
                    Member {
                        field_name: Some("buf".to_owned()),
                        value: Value::Array(ArrayValue {
                            type_id: None,
                            type_ident: ctx.type_graph.identity(inner_type).as_array_type(),
                            items: Some(items),
                            // set to `None` because the address operator unavailable for spec vars
                            raw_address: None,
                        }),
                    },
                    Member {
                        field_name: Some("cap".to_owned()),
                        value: Value::Scalar(ScalarValue {
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
        ctx: &ParseContext,
        structure: &StructValue,
        type_params: &HashMap<String, Option<TypeId>>,
        is_const_initialized: bool,
    ) -> Option<SpecializedValue> {
        let tls_var = if is_const_initialized {
            self.parse_const_init_tls_inner(ctx, Value::Struct(structure.clone()), type_params)
        } else {
            self.parse_tls_inner_old(ctx, Value::Struct(structure.clone()), type_params)
        };

        weak_error!(tls_var.context("TLS variable interpretation")).map(SpecializedValue::Tls)
    }

    fn parse_const_init_tls_inner(
        &self,
        ctx: &ParseContext,
        inner: Value,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<TlsVariable, ParsingError> {
        let value_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let value = inner.field("value");
        Ok(TlsVariable {
            inner_value: value.map(Box::new),
            inner_type: ctx.type_graph.identity(value_type),
        })
    }

    fn parse_tls_inner_old(
        &self,
        ctx: &ParseContext,
        inner_val: Value,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<TlsVariable, ParsingError> {
        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;

        let inner = inner_val
            .bfs_iterator()
            .find_map(|(field, child)| {
                (field == FieldOrIndex::Field(Some("inner"))).then_some(child)
            })
            .ok_or(FieldNotFound("inner"))?;
        let inner_option = inner.assume_field_as_rust_enum("value")?;
        let inner_value = inner_option.value.ok_or(IncompleteInterp("value"))?;

        // we assume that DWARF representation of tls variable contains ::Option
        if let Value::Struct(ref opt_variant) = inner_value.value {
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
                inner_value: tls_value,
                inner_type: ctx.type_graph.identity(inner_type),
            });
        }

        Err(ParsingError::Assume(IncompleteInterp(
            "expect TLS inner value as option",
        )))
    }

    pub fn parse_tls(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<Option<TlsVariable>, ParsingError> {
        if structure.type_ident.namespace().contains(&["eager"]) {
            // constant tls
            self.parse_const_tls_inner(ctx, Value::Struct(structure.clone()), type_params)
        } else {
            self.parse_tls_inner(ctx, Value::Struct(structure.clone()), type_params)
        }
    }

    fn parse_tls_inner(
        &self,
        ctx: &ParseContext,
        inner_val: Value,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<Option<TlsVariable>, ParsingError> {
        if type_params.is_empty() {
            return Ok(None);
        }

        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;

        let state = inner_val
            .bfs_iterator()
            .find_map(|(field, child)| {
                (field == FieldOrIndex::Field(Some("state"))).then_some(child)
            })
            .ok_or(FieldNotFound("state"))?;

        let state = state.assume_field_as_rust_enum("value")?;
        if let Some(member) = state.value {
            let tls_val = if member.field_name.as_deref() == Some("Alive") {
                member.value.field("__0").map(Box::new)
            } else {
                return Ok(None);
            };

            return Ok(Some(TlsVariable {
                inner_value: tls_val,
                inner_type: ctx.type_graph.identity(inner_type),
            }));
        };
        Ok(None)
    }

    fn parse_const_tls_inner(
        &self,
        ctx: &ParseContext,
        inner_val: Value,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<Option<TlsVariable>, ParsingError> {
        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;

        if let Some(val) = inner_val.field("val") {
            return Ok(Some(TlsVariable {
                inner_value: val.field("value").map(Box::new),
                inner_type: ctx.type_graph.identity(inner_type),
            }));
        }

        Err(ParsingError::Assume(IncompleteInterp(
            "expect TLS inner value as `val` field",
        )))
    }

    pub fn parse_hashmap(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
    ) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_hashmap_inner(ctx, Value::Struct(structure.clone()))
                .context("HashMap<K, V> interpretation")
        )
        .map(SpecializedValue::HashMap)
    }

    fn parse_hashmap_inner(
        &self,
        ctx: &ParseContext,
        val: Value,
    ) -> Result<HashMapVariable, ParsingError> {
        let ctrl = val.assume_field_as_pointer("pointer")?;
        let bucket_mask = val.assume_field_as_scalar_number("bucket_mask")?;

        let table = val.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;

        let r#type = ctx.type_graph;
        let kv_size = r#type
            .type_size_in_bytes(ctx.evaluation_context, kv_type)
            .ok_or(UnknownSize(r#type.identity(kv_type)))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(ctx.evaluation_context.expl_ctx.pid_on_focus())?;
        let kv_items = iterator
            .map_err(ParsingError::from)
            .filter_map(|bucket| {
                let raw_data = bucket.read(ctx.evaluation_context.expl_ctx.pid_on_focus());
                let data = weak_error!(raw_data).map(|d| ObjectBinaryRepr {
                    raw_data: Bytes::from(d),
                    address: Some(bucket.location()),
                    size: bucket.size(),
                });

                let tuple = self.parser.parse_inner(ctx, data, kv_type);

                if let Some(Value::Struct(mut tuple)) = tuple {
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
            type_ident: val.r#type().to_owned(),
            kv_items,
        })
    }

    pub fn parse_hashset(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
    ) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_hashset_inner(ctx, Value::Struct(structure.clone()))
                .context("HashSet<T> interpretation")
        )
        .map(SpecializedValue::HashSet)
    }

    fn parse_hashset_inner(
        &self,
        ctx: &ParseContext,
        val: Value,
    ) -> Result<HashSetVariable, ParsingError> {
        let ctrl = val.assume_field_as_pointer("pointer")?;
        let bucket_mask = val.assume_field_as_scalar_number("bucket_mask")?;

        let table = val.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let r#type = ctx.type_graph;
        let kv_size = r#type
            .type_size_in_bytes(ctx.evaluation_context, kv_type)
            .ok_or_else(|| UnknownSize(r#type.identity(kv_type)))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(ctx.evaluation_context.expl_ctx.pid_on_focus())?;
        let items = iterator
            .map_err(ParsingError::from)
            .filter_map(|bucket| {
                let raw_data = bucket.read(ctx.evaluation_context.expl_ctx.pid_on_focus());
                let data = weak_error!(raw_data).map(|d| ObjectBinaryRepr {
                    raw_data: Bytes::from(d),
                    address: Some(bucket.location()),
                    size: bucket.size(),
                });

                let tuple = self.parser.parse_inner(ctx, data, kv_type);

                if let Some(Value::Struct(mut tuple)) = tuple {
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
            type_ident: val.r#type().to_owned(),
            items,
        })
    }

    pub fn parse_btree_map(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
        identity: TypeId,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_btree_map_inner(
                ctx,
                Value::Struct(structure.clone()),
                identity,
                type_params
            )
            .context("BTreeMap<K, V> interpretation")
        )
        .map(SpecializedValue::BTreeMap)
    }

    fn parse_btree_map_inner(
        &self,
        ctx: &ParseContext,
        val: Value,
        identity: TypeId,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<HashMapVariable, ParsingError> {
        let height = val.assume_field_as_scalar_number("height")?;
        let ptr = val.assume_field_as_pointer("pointer")?;

        let k_type = type_params
            .get("K")
            .ok_or(TypeParameterNotFound("K"))?
            .ok_or(TypeParameterTypeNotFound("K"))?;
        let v_type = type_params
            .get("V")
            .ok_or(TypeParameterNotFound("V"))?
            .ok_or(TypeParameterTypeNotFound("V"))?;

        let reflection = BTreeReflection::new(
            ctx.type_graph,
            ptr,
            height as usize,
            identity,
            k_type,
            v_type,
        )?;
        let iterator = reflection.iter(ctx.evaluation_context)?;
        let kv_items = iterator
            .map_err(ParsingError::from)
            .filter_map(|(k, v)| {
                let Some(key) = self.parser.parse_inner(ctx, Some(k), k_type) else {
                    return Ok(None);
                };

                let Some(value) = self.parser.parse_inner(ctx, Some(v), v_type) else {
                    return Ok(None);
                };

                Ok(Some((key, value)))
            })
            .collect::<Vec<_>>()?;

        Ok(HashMapVariable {
            type_ident: val.r#type().to_owned(),
            kv_items,
        })
    }

    pub fn parse_btree_set(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_btree_set_inner(Value::Struct(structure.clone()))
                .context("BTreeSet interpretation")
        )
        .map(SpecializedValue::BTreeSet)
    }

    fn parse_btree_set_inner(&self, val: Value) -> Result<HashSetVariable, ParsingError> {
        let inner_map = val
            .bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let Value::Specialized {
                    value: Some(SpecializedValue::BTreeMap(map)),
                    ..
                } = child
                {
                    if field_or_idx == FieldOrIndex::Field(Some("map")) {
                        return Some(map.clone());
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("BTreeSet"))?;

        Ok(HashSetVariable {
            type_ident: val.r#type().to_owned(),
            items: inner_map.kv_items.into_iter().map(|(k, _)| k).collect(),
        })
    }

    pub fn parse_vec_dequeue(
        &self,
        ctx: &ParseContext,
        structure: &StructValue,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_vec_dequeue_inner(ctx, Value::Struct(structure.clone()), type_params)
                .context("VeqDequeue<T> interpretation")
        )
        .map(SpecializedValue::VecDeque)
    }

    fn parse_vec_dequeue_inner(
        &self,
        ctx: &ParseContext,
        val: Value,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> Result<VecValue, ParsingError> {
        let inner_type = type_params
            .get("T")
            .ok_or(TypeParameterNotFound("T"))?
            .ok_or(TypeParameterTypeNotFound("T"))?;
        let len = val.assume_field_as_scalar_number("len")? as usize;
        let len = guard_len(len as i64) as usize;

        let r#type = ctx.type_graph;
        let el_type_size = r#type
            .type_size_in_bytes(ctx.evaluation_context, inner_type)
            .ok_or_else(|| UnknownSize(r#type.identity(inner_type)))?
            as usize;
        let cap = if el_type_size == 0 {
            usize::MAX
        } else {
            extract_capacity(ctx, &val)?
        };
        let head = val.assume_field_as_scalar_number("head")? as usize;

        let wrapped_start = if head >= cap { head - cap } else { head };
        let head_len = cap - wrapped_start;

        let slice_ranges = if head_len >= len {
            (wrapped_start..wrapped_start + len, 0..0)
        } else {
            let tail_len = len - head_len;
            (wrapped_start..cap, 0..tail_len)
        };

        let data_ptr = val.assume_field_as_pointer("pointer")? as usize;

        let data = debugger::read_memory_by_pid(
            ctx.evaluation_context.expl_ctx.pid_on_focus(),
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

                Some(ArrayItem {
                    index: i as i64,
                    value: self.parser.parse_inner(ctx, Some(el_data), inner_type)?,
                })
            })
            .collect::<Vec<_>>();

        Ok(VecValue {
            structure: StructValue {
                type_id: None,
                type_ident: val.r#type().to_owned(),
                members: vec![
                    Member {
                        field_name: Some("buf".to_owned()),
                        value: Value::Array(ArrayValue {
                            type_id: None,
                            type_ident: ctx.type_graph.identity(inner_type).as_array_type(),
                            items: Some(items),
                            // set to `None` because the address operator unavailable for spec vars
                            raw_address: None,
                        }),
                    },
                    Member {
                        field_name: Some("cap".to_owned()),
                        value: Value::Scalar(ScalarValue {
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

    pub fn parse_cell(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_cell_inner(Value::Struct(structure.clone()))
                .context("Cell<T> interpretation")
        )
        .map(Box::new)
        .map(SpecializedValue::Cell)
    }

    fn parse_cell_inner(&self, val: Value) -> Result<Value, ParsingError> {
        let unsafe_cell = val.assume_field_as_struct("value")?;
        let member = unsafe_cell
            .members
            .first()
            .ok_or(IncompleteInterp("UnsafeCell"))?;
        Ok(member.value.clone())
    }

    pub fn parse_refcell(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_refcell_inner(Value::Struct(structure.clone()))
                .context("RefCell<T> interpretation")
        )
        .map(Box::new)
        .map(SpecializedValue::RefCell)
    }

    fn parse_refcell_inner(&self, val: Value) -> Result<Value, ParsingError> {
        let borrow = val
            .bfs_iterator()
            .find_map(|(_, child)| {
                if let Value::Specialized {
                    value: Some(SpecializedValue::Cell(val)),
                    ..
                } = child
                {
                    return Some(val.clone());
                }
                None
            })
            .ok_or(IncompleteInterp("Cell"))?;
        let Value::Scalar(var) = *borrow else {
            return Err(IncompleteInterp("Cell").into());
        };
        let borrow = Value::Scalar(var);

        let unsafe_cell = val.assume_field_as_struct("value")?;
        let value = unsafe_cell
            .members
            .first()
            .ok_or(IncompleteInterp("UnsafeCell"))?;

        Ok(Value::Struct(StructValue {
            type_id: None,
            type_ident: val.r#type().to_owned(),
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

    pub fn parse_rc(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_rc_inner(Value::Struct(structure.clone()))
                .context("Rc<T> interpretation")
        )
        .map(SpecializedValue::Rc)
    }

    fn parse_rc_inner(&self, val: Value) -> Result<PointerValue, ParsingError> {
        Ok(val
            .bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let Value::Pointer(pointer) = child {
                    if field_or_idx == FieldOrIndex::Field(Some("pointer")) {
                        let new_pointer = pointer.clone();
                        return Some(new_pointer);
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("rc"))?)
    }

    pub fn parse_arc(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_arc_inner(Value::Struct(structure.clone()))
                .context("Arc<T> interpretation")
        )
        .map(SpecializedValue::Arc)
    }

    fn parse_arc_inner(&self, val: Value) -> Result<PointerValue, ParsingError> {
        Ok(val
            .bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let Value::Pointer(pointer) = child {
                    if field_or_idx == FieldOrIndex::Field(Some("pointer")) {
                        let new_pointer = pointer.clone();
                        return Some(new_pointer);
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("Arc"))?)
    }

    pub fn parse_uuid(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_uuid_inner(structure)
                .context("Uuid interpretation")
        )
        .map(SpecializedValue::Uuid)
    }

    fn parse_uuid_inner(&self, structure: &StructValue) -> Result<[u8; 16], ParsingError> {
        let member0 = structure.members.first().ok_or(FieldNotFound("member 0"))?;
        let Value::Array(ref arr) = member0.value else {
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
            let Value::Scalar(ScalarValue {
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

    fn parse_timespec(&self, timespec: &StructValue) -> Result<(i64, u32), ParsingError> {
        let &[
            Member {
                value: Value::Scalar(secs),
                ..
            },
            Member {
                value: Value::Struct(n_secs),
                ..
            },
        ] = &timespec.members.as_slice()
        else {
            let err = "`Timespec` should contains secs and n_secs fields";
            return Err(UnexpectedType(err).into());
        };

        let &[
            Member {
                value: Value::Scalar(n_secs),
                ..
            },
        ] = &n_secs.members.as_slice()
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

    pub fn parse_sys_time(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_sys_time_inner(structure)
                .context("SystemTime interpretation")
        )
        .map(SpecializedValue::SystemTime)
    }

    fn parse_sys_time_inner(&self, structure: &StructValue) -> Result<(i64, u32), ParsingError> {
        let &[
            Member {
                value: Value::Struct(time_instant),
                ..
            },
        ] = &structure.members.as_slice()
        else {
            let err = "`std::time::SystemTime` should contains a `time::SystemTime` field";
            return Err(UnexpectedType(err).into());
        };

        let &[
            Member {
                value: Value::Struct(timespec),
                ..
            },
        ] = &time_instant.members.as_slice()
        else {
            let err = "`time::SystemTime` should contains a `Timespec` field";
            return Err(UnexpectedType(err).into());
        };

        self.parse_timespec(timespec)
    }

    pub fn parse_instant(&self, structure: &StructValue) -> Option<SpecializedValue> {
        weak_error!(
            self.parse_instant_inner(structure)
                .context("Instant interpretation")
        )
        .map(SpecializedValue::Instant)
    }

    fn parse_instant_inner(&self, structure: &StructValue) -> Result<(i64, u32), ParsingError> {
        let &[
            Member {
                value: Value::Struct(time_instant),
                ..
            },
        ] = &structure.members.as_slice()
        else {
            let err = "`std::time::Instant` should contains a `time::Instant` field";
            return Err(UnexpectedType(err).into());
        };

        let &[
            Member {
                value: Value::Struct(timespec),
                ..
            },
        ] = &time_instant.members.as_slice()
        else {
            let err = "`time::Instant` should contains a `Timespec` field";
            return Err(UnexpectedType(err).into());
        };

        self.parse_timespec(timespec)
    }
}

fn extract_capacity(ctx: &ParseContext, val: &Value) -> Result<usize, ParsingError> {
    let rust_version = ctx
        .evaluation_context
        .rustc_version()
        .ok_or(ParsingError::UnsupportedVersion)?;

    version_switch!(
    rust_version,
    .. (1 . 76) => val.assume_field_as_scalar_number("cap")? as usize,
    (1 . 76) .. => {
            let cap_s = val.assume_field_as_struct("cap")?;
            let cap = &cap_s.members.first().ok_or(IncompleteInterp("Vec"))?.value;
            if let Value::Scalar(ScalarValue {value: Some(SupportedScalar::Usize(cap)), ..}) = cap {
                Ok(*cap)
            } else {
                Err(AssumeError::FieldNotANumber("cap"))
            }?
        },
    )
    .ok_or(ParsingError::UnsupportedVersion)
}
