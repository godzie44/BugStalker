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
    ArrayVariable, AssumeError, ParsingError, PointerVariable, ScalarVariable, StructVariable,
    SupportedScalar, VariableIR, VariableIdentity, VariableParser,
};
use crate::{debugger, version_switch, weak_error};
use anyhow::Context;
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use itertools::Itertools;
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
            self.structure.members.get_mut(0),
            Some(VariableIR::Array(_))
        ));

        if let Some(VariableIR::Array(array)) = self.structure.members.get_mut(0) {
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
    Cell {
        value: Option<Box<VariableIR>>,
        original: StructVariable,
    },
    RefCell {
        value: Option<Box<VariableIR>>,
        original: StructVariable,
    },
    Rc {
        value: Option<PointerVariable>,
        original: StructVariable,
    },
    Arc {
        value: Option<PointerVariable>,
        original: StructVariable,
    },
    Uuid {
        value: Option<[u8; 16]>,
        original: StructVariable,
    },
    SystemTime {
        value: Option<(i64, u32)>,
        original: StructVariable,
    },
    Instant {
        value: Option<(i64, u32)>,
        original: StructVariable,
    },
}

impl SpecializedVariableIR {
    pub(super) fn in_memory_location(&self) -> Option<usize> {
        match self {
            SpecializedVariableIR::Vector { original, .. } => original.raw_address,
            SpecializedVariableIR::VecDeque { original, .. } => original.raw_address,
            SpecializedVariableIR::HashMap { original, .. } => original.raw_address,
            SpecializedVariableIR::HashSet { original, .. } => original.raw_address,
            SpecializedVariableIR::BTreeMap { original, .. } => original.raw_address,
            SpecializedVariableIR::BTreeSet { original, .. } => original.raw_address,
            SpecializedVariableIR::String { original, .. } => original.raw_address,
            SpecializedVariableIR::Str { original, .. } => original.raw_address,
            SpecializedVariableIR::Tls { original, .. } => original.raw_address,
            SpecializedVariableIR::Cell { original, .. } => original.raw_address,
            SpecializedVariableIR::RefCell { original, .. } => original.raw_address,
            SpecializedVariableIR::Rc { original, .. } => original.raw_address,
            SpecializedVariableIR::Arc { original, .. } => original.raw_address,
            SpecializedVariableIR::Uuid { original, .. } => original.raw_address,
            SpecializedVariableIR::SystemTime { original, .. } => original.raw_address,
            SpecializedVariableIR::Instant { original, .. } => original.raw_address,
        }
    }

    pub(super) fn type_id(&self) -> Option<TypeId> {
        match self {
            SpecializedVariableIR::Vector { original, .. } => original.type_id,
            SpecializedVariableIR::VecDeque { original, .. } => original.type_id,
            SpecializedVariableIR::HashMap { original, .. } => original.type_id,
            SpecializedVariableIR::HashSet { original, .. } => original.type_id,
            SpecializedVariableIR::BTreeMap { original, .. } => original.type_id,
            SpecializedVariableIR::BTreeSet { original, .. } => original.type_id,
            SpecializedVariableIR::String { original, .. } => original.type_id,
            SpecializedVariableIR::Str { original, .. } => original.type_id,
            SpecializedVariableIR::Tls { original, .. } => original.type_id,
            SpecializedVariableIR::Cell { original, .. } => original.type_id,
            SpecializedVariableIR::RefCell { original, .. } => original.type_id,
            SpecializedVariableIR::Rc { original, .. } => original.type_id,
            SpecializedVariableIR::Arc { original, .. } => original.type_id,
            SpecializedVariableIR::Uuid { original, .. } => original.type_id,
            SpecializedVariableIR::SystemTime { original, .. } => original.type_id,
            SpecializedVariableIR::Instant { original, .. } => original.type_id,
        }
    }
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
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::String {
            string: weak_error!(self
                .parse_string_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("String interpretation")),
            original: structure,
        }
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
        structure: StructVariable,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Vector {
            vec: weak_error!(self
                .parse_vector_inner(eval_ctx, VariableIR::Struct(structure.clone()), type_params)
                .context("Vec<T> interpretation")),
            original: structure,
        }
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
            .map(|(i, chunk)| {
                let data = ObjectBinaryRepr {
                    raw_data: raw_data.slice_ref(chunk),
                    address: Some(data_ptr + (i * el_type_size)),
                    size: el_type_size,
                };
                self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                    Some(data),
                    inner_type,
                )
            })
            .collect::<Vec<_>>();

        Ok(VecVariable {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_id: None,
                type_ident: ir.r#type().clone(),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        identity: VariableIdentity::no_namespace(Some("buf".to_owned())),
                        type_id: None,
                        type_ident: self.parser.r#type.identity(inner_type).as_array_type(),
                        items: Some(items),
                        // set to `None` because the address operator unavailable for spec vars
                        raw_address: None,
                    }),
                    VariableIR::Scalar(ScalarVariable {
                        identity: VariableIdentity::no_namespace(Some("cap".to_owned())),
                        type_id: None,
                        type_ident: TypeIdentity::no_namespace("usize"),
                        value: Some(SupportedScalar::Usize(cap as usize)),
                        // set to `None` because the address operator unavailable for spec vars
                        raw_address: None,
                    }),
                ],
                type_params: type_params.clone(),
                // set to `None` because the address operator unavailable for spec vars
                raw_address: None,
            },
        })
    }

    pub fn parse_tls(
        &self,
        structure: StructVariable,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Tls {
            tls_var: weak_error!(self
                .parse_tls_inner(VariableIR::Struct(structure.clone()), type_params)
                .context("TLS variable interpretation")),
            original: structure,
        }
    }

    fn parse_tls_inner(
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
            .find(|child| child.name() == "inner")
            .ok_or(FieldNotFound("inner"))?;
        let inner_option = inner.assume_field_as_rust_enum("value")?;
        let inner_value = inner_option.value.ok_or(IncompleteInterp("value"))?;

        // we assume that DWARF representation of tls variable contains ::Option
        if let VariableIR::Struct(opt_variant) = inner_value.as_ref() {
            let tls_value = if opt_variant.type_ident.name() == Some("None") {
                None
            } else {
                Some(Box::new(
                    inner_value
                        .bfs_iterator()
                        .find(|child| child.name() == "0")
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

    pub fn parse_hashmap(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::HashMap {
            map: weak_error!(self
                .parse_hashmap_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("HashMap<K, V> interpretation")),
            original: structure,
        }
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

                if let VariableIR::Struct(mut tuple) = tuple {
                    if tuple.members.len() == 2 {
                        let v = tuple.members.pop();
                        let k = tuple.members.pop();
                        return Ok(Some((k.unwrap(), v.unwrap())));
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
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::HashSet {
            set: weak_error!(self
                .parse_hashset_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("HashSet<T> interpretation")),
            original: structure,
        }
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

                if let VariableIR::Struct(mut tuple) = tuple {
                    if tuple.members.len() == 2 {
                        let _ = tuple.members.pop();
                        let k = tuple.members.pop().unwrap();
                        return Ok(Some(k));
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
        structure: StructVariable,
        identity: TypeId,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::BTreeMap {
            map: weak_error!(self
                .parse_btree_map_inner(
                    eval_ctx,
                    VariableIR::Struct(structure.clone()),
                    identity,
                    type_params
                )
                .context("BTreeMap<K, V> interpretation")),
            original: structure,
        }
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
                let key = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("k".to_string())),
                    Some(k),
                    k_type,
                );

                let value = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("v".to_string())),
                    Some(v),
                    v_type,
                );

                Ok((key, value))
            })
            .collect::<Vec<_>>()?;

        Ok(HashMapVariable {
            identity: ir.identity().clone(),
            type_ident: ir.r#type().to_owned(),
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

    fn parse_btree_set_inner(&self, ir: VariableIR) -> Result<HashSetVariable, ParsingError> {
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
        structure: StructVariable,
        type_params: &HashMap<String, Option<TypeId>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::VecDeque {
            vec: weak_error!(self
                .parse_vec_dequeue_inner(
                    eval_ctx,
                    VariableIR::Struct(structure.clone()),
                    type_params
                )
                .context("VeqDequeue<T> interpretation")),
            original: structure,
        }
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
            .map(|(i, real_idx)| {
                let offset = real_idx * el_type_size;
                let el_raw_data = &data[offset..(real_idx + 1) * el_type_size];
                let el_data = ObjectBinaryRepr {
                    raw_data: data.slice_ref(el_raw_data),
                    address: Some(data_ptr + offset),
                    size: el_type_size,
                };
                self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                    Some(el_data),
                    inner_type,
                )
            })
            .collect::<Vec<_>>();

        Ok(VecVariable {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_id: None,
                type_ident: ir.r#type().to_owned(),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        identity: VariableIdentity::no_namespace(Some("buf".to_owned())),
                        type_id: None,
                        type_ident: self.parser.r#type.identity(inner_type).as_array_type(),
                        items: Some(items),
                        // set to `None` because the address operator unavailable for spec vars
                        raw_address: None,
                    }),
                    VariableIR::Scalar(ScalarVariable {
                        identity: VariableIdentity::no_namespace(Some("cap".to_owned())),
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
                ],
                type_params: type_params.clone(),
                // set to `None` because the address operator unavailable for spec vars
                raw_address: None,
            },
        })
    }

    pub fn parse_cell(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::Cell {
            value: weak_error!(self
                .parse_cell_inner(VariableIR::Struct(structure.clone()))
                .context("Cell<T> interpretation"))
            .map(Box::new),
            original: structure,
        }
    }

    fn parse_cell_inner(&self, ir: VariableIR) -> Result<VariableIR, ParsingError> {
        let unsafe_cell = ir.assume_field_as_struct("value")?;
        let value = unsafe_cell
            .members
            .first()
            .ok_or(IncompleteInterp("UnsafeCell"))?;
        Ok(value.clone())
    }

    pub fn parse_refcell(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::RefCell {
            value: weak_error!(self
                .parse_refcell_inner(VariableIR::Struct(structure.clone()))
                .context("RefCell<T> interpretation"))
            .map(Box::new),
            original: structure,
        }
    }

    fn parse_refcell_inner(&self, ir: VariableIR) -> Result<VariableIR, ParsingError> {
        let borrow = ir
            .bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Specialized(SpecializedVariableIR::Cell {
                    value: Some(val),
                    ..
                }) = child
                {
                    return Some(val.clone());
                }
                None
            })
            .ok_or(IncompleteInterp("Cell"))?;
        let VariableIR::Scalar(mut var) = *borrow else {
            return Err(IncompleteInterp("Cell").into());
        };
        var.identity = VariableIdentity::no_namespace(Some("borrow".to_string()));
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
            members: vec![borrow, value.clone()],
            type_params: Default::default(),
            // set to `None` because the address operator unavailable for spec vars
            raw_address: None,
        }))
    }

    pub fn parse_rc(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::Rc {
            value: weak_error!(self
                .parse_rc_inner(VariableIR::Struct(structure.clone()))
                .context("Rc<T> interpretation")),
            original: structure,
        }
    }

    fn parse_rc_inner(&self, ir: VariableIR) -> Result<PointerVariable, ParsingError> {
        Ok(ir
            .bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Pointer(pointer) = child {
                    if pointer.identity.name.as_deref()? == "pointer" {
                        let mut new_pointer = pointer.clone();
                        new_pointer.identity = ir.identity().clone();
                        return Some(new_pointer);
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("rc"))?)
    }

    pub fn parse_arc(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::Arc {
            value: weak_error!(self
                .parse_arc_inner(VariableIR::Struct(structure.clone()))
                .context("Arc<T> interpretation")),
            original: structure,
        }
    }

    fn parse_arc_inner(&self, ir: VariableIR) -> Result<PointerVariable, ParsingError> {
        Ok(ir
            .bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Pointer(pointer) = child {
                    if pointer.identity.name.as_deref()? == "pointer" {
                        let mut new_pointer = pointer.clone();
                        new_pointer.identity = ir.identity().clone();
                        return Some(new_pointer);
                    }
                }
                None
            })
            .ok_or(IncompleteInterp("Arc"))?)
    }

    pub fn parse_uuid(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::Uuid {
            value: weak_error!(self
                .parse_uuid_inner(&structure)
                .context("Uuid interpretation")),
            original: structure,
        }
    }

    fn parse_uuid_inner(&self, structure: &StructVariable) -> Result<[u8; 16], ParsingError> {
        let member0 = structure.members.first().ok_or(FieldNotFound("member 0"))?;
        let VariableIR::Array(arr) = member0 else {
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
            }) = item
            else {
                return Err(UnexpectedType("uuid struct member must be [u8; 16]").into());
            };
            bytes_repr[i] = *byte;
        }

        Ok(bytes_repr)
    }

    fn parse_timespec(&self, timespec: &StructVariable) -> Result<(i64, u32), ParsingError> {
        let &[VariableIR::Scalar(secs), VariableIR::Struct(n_secs)] = &timespec.members.as_slice()
        else {
            let err = "`Timespec` should contains secs and n_secs fields";
            return Err(UnexpectedType(err).into());
        };

        let &[VariableIR::Scalar(n_secs)] = &n_secs.members.as_slice() else {
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

    pub fn parse_sys_time(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::SystemTime {
            value: weak_error!(self
                .parse_sys_time_inner(&structure)
                .context("SystemTime interpretation")),
            original: structure,
        }
    }

    fn parse_sys_time_inner(&self, structure: &StructVariable) -> Result<(i64, u32), ParsingError> {
        let &[VariableIR::Struct(time_instant)] = &structure.members.as_slice() else {
            let err = "`std::time::SystemTime` should contains a `time::SystemTime` field";
            return Err(UnexpectedType(err).into());
        };

        let &[VariableIR::Struct(timespec)] = &time_instant.members.as_slice() else {
            let err = "`time::SystemTime` should contains a `Timespec` field";
            return Err(UnexpectedType(err).into());
        };

        self.parse_timespec(timespec)
    }

    pub fn parse_instant(&self, structure: StructVariable) -> SpecializedVariableIR {
        SpecializedVariableIR::Instant {
            value: weak_error!(self
                .parse_instant_inner(&structure)
                .context("Instant interpretation")),
            original: structure,
        }
    }

    fn parse_instant_inner(&self, structure: &StructVariable) -> Result<(i64, u32), ParsingError> {
        let &[VariableIR::Struct(time_instant)] = &structure.members.as_slice() else {
            let err = "`std::time::Instant` should contains a `time::Instant` field";
            return Err(UnexpectedType(err).into());
        };

        let &[VariableIR::Struct(timespec)] = &time_instant.members.as_slice() else {
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
                        let cap = cap_s.members.first().ok_or(IncompleteInterp("Vec"))?;
                        if let VariableIR::Scalar(ScalarVariable {value: Some(SupportedScalar::Usize(cap)), ..}) = cap {
                            Ok(*cap)
                        } else {
                            Err(AssumeError::FieldNotANumber("cap"))
                        }?
                    },
                ).ok_or(ParsingError::UnsupportedVersion)
}
