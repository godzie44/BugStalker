use crate::debugger::TypeDeclaration;
use crate::debugger::debugee::dwarf::eval::EvaluationContext;
use crate::debugger::debugee::dwarf::r#type::{
    ArrayType, ComplexType, ScalarType, StructureMember, TypeId,
};
use crate::debugger::variable::value::specialization::VariableParserExtension;
use crate::debugger::variable::value::{
    ArrayItem, ArrayValue, CEnumValue, CModifiedValue, Member, PointerValue, RustEnumValue,
    ScalarValue, SpecializedValue, StructValue, SubroutineValue, SupportedScalar, Value,
};
use crate::debugger::variable::{Identity, ObjectBinaryRepr};
use crate::version::Version;
use crate::version_switch;
use bytes::Bytes;
use gimli::{
    DW_ATE_ASCII, DW_ATE_UTF, DW_ATE_address, DW_ATE_boolean, DW_ATE_float, DW_ATE_signed,
    DW_ATE_signed_char, DW_ATE_unsigned, DW_ATE_unsigned_char,
};
use indexmap::IndexMap;
use log::warn;
use std::collections::HashMap;
use std::fmt::Display;

/// Additional information about value.
//
// FIXME: this modifier currently using only for TLS variables and should be deleted
// after minimal supported rust version will be greater than 1.80.0
#[derive(Default)]
pub struct ValueModifiers {
    tls: bool,
    tls_const: bool,
    const_tls_duplicate: bool,
}

impl ValueModifiers {
    pub fn from_identity(p_ctx: &ParseContext, ident: Identity) -> ValueModifiers {
        let mut this = ValueModifiers::default();

        let ver = p_ctx.evaluation_context.rustc_version().unwrap_or_default();
        if ver >= Version((1, 80, 0)) {
            // not sure that value is tls, but some additional checks will be occurred on
            // a value type at parsing stage
            this.tls = ident.name.as_deref() == Some("VAL");

            // This condition protects against duplication of the constant tls variables
            if ident.namespace.contains(&["thread_local_const_init"])
                && !ident.namespace.contains(&["{closure#0}"])
            {
                this.const_tls_duplicate = true;
            }
        } else {
            let var_name_is_tls = ident.namespace.contains(&["__getit"])
                && (ident.name.as_deref() == Some("VAL") || ident.name.as_deref() == Some("__KEY"));
            if var_name_is_tls {
                this.tls = true;
                if ident.name.as_deref() == Some("VAL") {
                    this.tls_const = true
                }
            }
        }

        this
    }
}

pub struct ParseContext<'a> {
    pub evaluation_context: &'a EvaluationContext<'a>,
    pub type_graph: &'a ComplexType,
}

/// Value parser object.
#[derive(Default)]
pub struct ValueParser;

impl ValueParser {
    /// Create parser for a type graph.
    ///
    /// # Arguments
    ///
    /// * `type_graph`: types that value parser can create
    pub fn new() -> Self {
        ValueParser
    }

    fn parse_scalar(
        &self,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
        r#type: &ScalarType,
    ) -> ScalarValue {
        fn render_scalar<S: Copy + Display>(data: Option<ObjectBinaryRepr>) -> Option<S> {
            data.as_ref().map(|v| scalar_from_bytes::<S>(&v.raw_data))
        }
        let in_debugee_loc = data.as_ref().and_then(|d| d.address);
        #[allow(non_upper_case_globals)]
        let value_view = r#type.encoding.and_then(|encoding| match encoding {
            DW_ATE_address => render_scalar::<usize>(data).map(SupportedScalar::Usize),
            DW_ATE_signed_char => render_scalar::<i8>(data).map(SupportedScalar::I8),
            DW_ATE_unsigned_char => render_scalar::<u8>(data).map(SupportedScalar::U8),
            DW_ATE_signed => match r#type.byte_size.unwrap_or(0) {
                0 => Some(SupportedScalar::Empty()),
                1 => render_scalar::<i8>(data).map(SupportedScalar::I8),
                2 => render_scalar::<i16>(data).map(SupportedScalar::I16),
                4 => render_scalar::<i32>(data).map(SupportedScalar::I32),
                8 => {
                    if r#type.name.as_deref() == Some("isize") {
                        render_scalar::<isize>(data).map(SupportedScalar::Isize)
                    } else {
                        render_scalar::<i64>(data).map(SupportedScalar::I64)
                    }
                }
                16 => render_scalar::<i128>(data).map(SupportedScalar::I128),
                _ => {
                    warn!(
                        "parse scalar: unexpected signed size: {size:?}",
                        size = r#type.byte_size
                    );
                    None
                }
            },
            DW_ATE_unsigned => match r#type.byte_size.unwrap_or(0) {
                0 => Some(SupportedScalar::Empty()),
                1 => render_scalar::<u8>(data).map(SupportedScalar::U8),
                2 => render_scalar::<u16>(data).map(SupportedScalar::U16),
                4 => render_scalar::<u32>(data).map(SupportedScalar::U32),
                8 => {
                    if r#type.name.as_deref() == Some("usize") {
                        render_scalar::<usize>(data).map(SupportedScalar::Usize)
                    } else {
                        render_scalar::<u64>(data).map(SupportedScalar::U64)
                    }
                }
                16 => render_scalar::<u128>(data).map(SupportedScalar::U128),
                _ => {
                    warn!(
                        "parse scalar: unexpected unsigned size: {size:?}",
                        size = r#type.byte_size
                    );
                    None
                }
            },
            DW_ATE_float => match r#type.byte_size.unwrap_or(0) {
                4 => render_scalar::<f32>(data).map(SupportedScalar::F32),
                8 => render_scalar::<f64>(data).map(SupportedScalar::F64),
                _ => {
                    warn!(
                        "parse scalar: unexpected float size: {size:?}",
                        size = r#type.byte_size
                    );
                    None
                }
            },
            DW_ATE_boolean => render_scalar::<bool>(data).map(SupportedScalar::Bool),
            DW_ATE_UTF => render_scalar::<char>(data).map(|char| {
                // WAITFORFIX: https://github.com/rust-lang/rust/issues/113819
                // this check is meaningfully here cause in case above there is a random bytes here,
                // and it may lead to panic in other places
                // (specially when someone tries to render this char)
                if String::from_utf8(char.to_string().into_bytes()).is_err() {
                    SupportedScalar::Char('?')
                } else {
                    SupportedScalar::Char(char)
                }
            }),
            DW_ATE_ASCII => render_scalar::<char>(data).map(SupportedScalar::Char),
            _ => {
                warn!("parse scalar: unexpected base type encoding: {encoding}");
                None
            }
        });

        ScalarValue {
            type_ident: r#type.identity(),
            type_id: Some(type_id),
            value: value_view,
            raw_address: in_debugee_loc,
        }
    }

    fn parse_struct_variable(
        &self,
        ctx: &ParseContext,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
        type_params: IndexMap<String, Option<TypeId>>,
        members: &[StructureMember],
    ) -> StructValue {
        let children = members
            .iter()
            .filter_map(|member| self.parse_struct_member(ctx, member, data.as_ref()))
            .collect();

        StructValue {
            type_id: Some(type_id),
            type_ident: ctx.type_graph.identity(type_id),
            members: children,
            type_params,
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_struct_member(
        &self,
        ctx: &ParseContext,
        member: &StructureMember,
        parent_data: Option<&ObjectBinaryRepr>,
    ) -> Option<Member> {
        let name = member.name.clone();
        let Some(type_ref) = member.type_ref else {
            warn!(
                "parse structure: unknown type for member {}",
                name.as_deref().unwrap_or_default()
            );
            return None;
        };
        let member_val =
            parent_data.and_then(|data| member.value(ctx.evaluation_context, ctx.type_graph, data));
        let value = self.parse_inner(ctx, member_val, type_ref)?;
        Some(Member {
            field_name: member.name.clone(),
            value,
        })
    }

    fn parse_array(
        &self,
        ctx: &ParseContext,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
        array_decl: &ArrayType,
    ) -> ArrayValue {
        let items = array_decl
            .bounds(ctx.evaluation_context)
            .and_then(|bounds| {
                let len = bounds.1 - bounds.0;
                let data = data.as_ref()?;
                let el_size = (array_decl.size_in_bytes(ctx.evaluation_context, ctx.type_graph)?
                    / len as u64) as usize;
                let bytes = &data.raw_data;
                let el_type_id = array_decl.element_type()?;

                let (mut bytes_chunks, mut empty_chunks);
                let raw_items_iter: &mut dyn Iterator<Item = (usize, &[u8])> = if el_size != 0 {
                    bytes_chunks = bytes.chunks(el_size).enumerate();
                    &mut bytes_chunks
                } else {
                    // if an item type is zst
                    let v: Vec<&[u8]> = vec![&[]; len as usize];
                    empty_chunks = v.into_iter().enumerate();
                    &mut empty_chunks
                };

                Some(
                    raw_items_iter
                        .filter_map(|(i, chunk)| {
                            let offset = i * el_size;
                            let data = ObjectBinaryRepr {
                                raw_data: bytes.slice_ref(chunk),
                                address: data.address.map(|addr| addr + offset),
                                size: el_size,
                            };

                            let value = self.parse_inner(ctx, Some(data), el_type_id)?;
                            Some(ArrayItem {
                                index: bounds.0 + i as i64,
                                value,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            });

        ArrayValue {
            items,
            type_id: Some(type_id),
            type_ident: ctx.type_graph.identity(type_id),
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_c_enum(
        &self,
        ctx: &ParseContext,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
        discr_type: Option<TypeId>,
        enumerators: &HashMap<i64, String>,
    ) -> CEnumValue {
        let in_debugee_loc = data.as_ref().and_then(|d| d.address);
        let mb_discr = discr_type.and_then(|type_id| self.parse_inner(ctx, data, type_id));

        let value = mb_discr.and_then(|discr| {
            if let Value::Scalar(scalar) = discr {
                scalar.try_as_number()
            } else {
                None
            }
        });

        CEnumValue {
            type_ident: ctx.type_graph.identity(type_id),
            type_id: Some(type_id),
            value: value.and_then(|val| enumerators.get(&val).cloned()),
            raw_address: in_debugee_loc,
        }
    }

    fn parse_rust_enum(
        &self,
        ctx: &ParseContext,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
        discr_member: Option<&StructureMember>,
        enumerators: &HashMap<Option<i64>, StructureMember>,
    ) -> RustEnumValue {
        let discr_value = discr_member.and_then(|member| {
            let discr = self.parse_struct_member(ctx, member, data.as_ref())?.value;
            if let Value::Scalar(scalar) = discr {
                return scalar.try_as_number();
            }
            None
        });

        let enumerator =
            discr_value.and_then(|v| enumerators.get(&Some(v)).or_else(|| enumerators.get(&None)));

        let enumerator = enumerator.and_then(|member| {
            Some(Box::new(self.parse_struct_member(
                ctx,
                member,
                data.as_ref(),
            )?))
        });

        RustEnumValue {
            type_id: Some(type_id),
            type_ident: ctx.type_graph.identity(type_id),
            value: enumerator,
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_pointer(
        &self,
        ctx: &ParseContext,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
        target_type: Option<TypeId>,
    ) -> PointerValue {
        let mb_ptr = data
            .as_ref()
            .map(|v| scalar_from_bytes::<*const ()>(&v.raw_data));

        let mut type_ident = ctx.type_graph.identity(type_id);
        if type_ident.is_unknown() {
            if let Some(target_type) = target_type {
                type_ident = ctx.type_graph.identity(target_type).as_deref_type();
            }
        }

        PointerValue {
            type_id: Some(type_id),
            type_ident,
            value: mb_ptr,
            target_type,
            target_type_size: None,
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_inner_with_modifiers(
        &self,
        ctx: &ParseContext,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
        modifiers: &ValueModifiers,
    ) -> Option<Value> {
        let type_graph = ctx.type_graph;
        match &type_graph.types[&type_id] {
            TypeDeclaration::Scalar(scalar_type) => {
                Some(Value::Scalar(self.parse_scalar(data, type_id, scalar_type)))
            }
            TypeDeclaration::Structure {
                namespaces: type_ns_h,
                members,
                type_params,
                name: struct_name,
                ..
            } => {
                let struct_var =
                    self.parse_struct_variable(ctx, data, type_id, type_params.clone(), members);

                let parser_ext = VariableParserExtension::new(self);
                // Reinterpret structure if underline data type is:
                // - Vector
                // - String
                // - &str
                // - tls variable
                // - hashmaps
                // - hashset
                // - btree map
                // - btree set
                // - vecdeque
                // - cell/refcell
                // - rc/arc
                // - uuid
                // - SystemTime/Instant
                if struct_name.as_deref() == Some("&str") {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_str(ctx, &struct_var),
                        original: struct_var,
                    });
                };

                if struct_name.as_deref() == Some("String") {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_string(ctx, &struct_var),
                        original: struct_var,
                    });
                };

                if struct_name.as_ref().map(|name| name.starts_with("Vec")) == Some(true)
                    && type_ns_h.contains(&["vec"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_vector(ctx, &struct_var, type_params),
                        original: struct_var,
                    });
                };

                let rust_version = ctx.evaluation_context.rustc_version().unwrap_or_default();
                let type_is_tls = version_switch!(
                    rust_version,
                    .. (1 . 77) => type_ns_h.contains(&["std", "sys", "common", "thread_local", "fast_local"]),
                    (1 . 77) .. (1 . 78) => type_ns_h.contains(&["std", "sys", "pal", "common", "thread_local", "fast_local"]),
                    (1 . 78) .. => type_ns_h.contains(&["std", "sys", "thread_local", "fast_local"]),
                ).unwrap_or_default();

                if type_is_tls || modifiers.tls {
                    return if rust_version >= Version((1, 80, 0)) {
                        match parser_ext.parse_tls(ctx, &struct_var, type_params) {
                            Ok(Some(value)) => Some(Value::Specialized {
                                value: Some(SpecializedValue::Tls(value)),
                                original: struct_var,
                            }),
                            Ok(None) => None,
                            Err(e) => {
                                warn!(target: "debugger", "{:#}", e);
                                Some(Value::Specialized {
                                    value: None,
                                    original: struct_var,
                                })
                            }
                        }
                    } else {
                        Some(Value::Specialized {
                            value: parser_ext.parse_tls_old(
                                ctx,
                                &struct_var,
                                type_params,
                                modifiers.tls_const,
                            ),
                            original: struct_var,
                        })
                    };
                }

                if struct_name.as_ref().map(|name| name.starts_with("HashMap")) == Some(true)
                    && type_ns_h.contains(&["collections", "hash", "map"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_hashmap(ctx, &struct_var),
                        original: struct_var,
                    });
                };

                if struct_name.as_ref().map(|name| name.starts_with("HashSet")) == Some(true)
                    && type_ns_h.contains(&["collections", "hash", "set"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_hashset(ctx, &struct_var),
                        original: struct_var,
                    });
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("BTreeMap"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "btree", "map"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_btree_map(ctx, &struct_var, type_id, type_params),
                        original: struct_var,
                    });
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("BTreeSet"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "btree", "set"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_btree_set(&struct_var),
                        original: struct_var,
                    });
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("VecDeque"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "vec_deque"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_vec_dequeue(ctx, &struct_var, type_params),
                        original: struct_var,
                    });
                };

                if struct_name.as_ref().map(|name| name.starts_with("Cell")) == Some(true)
                    && type_ns_h.contains(&["cell"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_cell(&struct_var),
                        original: struct_var,
                    });
                };

                if struct_name.as_ref().map(|name| name.starts_with("RefCell")) == Some(true)
                    && type_ns_h.contains(&["cell"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_refcell(&struct_var),
                        original: struct_var,
                    });
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("Rc<") | name.starts_with("Weak<"))
                    == Some(true)
                    && type_ns_h.contains(&["rc"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_rc(&struct_var),
                        original: struct_var,
                    });
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("Arc<") | name.starts_with("Weak<"))
                    == Some(true)
                    && type_ns_h.contains(&["sync"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_arc(&struct_var),
                        original: struct_var,
                    });
                };

                if struct_name.as_ref().map(|name| name == "Uuid") == Some(true)
                    && type_ns_h.contains(&["uuid"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_uuid(&struct_var),
                        original: struct_var,
                    });
                };

                if struct_name.as_ref().map(|name| name == "Instant") == Some(true)
                    && type_ns_h.contains(&["std", "time"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_instant(&struct_var),
                        original: struct_var,
                    });
                };

                if struct_name.as_ref().map(|name| name == "SystemTime") == Some(true)
                    && type_ns_h.contains(&["std", "time"])
                {
                    return Some(Value::Specialized {
                        value: parser_ext.parse_sys_time(&struct_var),
                        original: struct_var,
                    });
                };

                Some(Value::Struct(struct_var))
            }
            TypeDeclaration::Array(decl) => {
                Some(Value::Array(self.parse_array(ctx, data, type_id, decl)))
            }
            TypeDeclaration::CStyleEnum {
                discr_type,
                enumerators,
                ..
            } => Some(Value::CEnum(self.parse_c_enum(
                ctx,
                data,
                type_id,
                *discr_type,
                enumerators,
            ))),
            TypeDeclaration::RustEnum {
                discr_type,
                enumerators,
                ..
            } => Some(Value::RustEnum(self.parse_rust_enum(
                ctx,
                data,
                type_id,
                discr_type.as_ref().map(|t| t.as_ref()),
                enumerators,
            ))),
            TypeDeclaration::Pointer { target_type, .. } => Some(Value::Pointer(
                self.parse_pointer(ctx, data, type_id, *target_type),
            )),
            TypeDeclaration::Union { members, .. } => {
                let struct_var =
                    self.parse_struct_variable(ctx, data, type_id, IndexMap::new(), members);
                Some(Value::Struct(struct_var))
            }
            TypeDeclaration::Subroutine { return_type, .. } => {
                let ret_type = return_type.map(|t_id| ctx.type_graph.identity(t_id));
                let fn_var = SubroutineValue {
                    type_id: Some(type_id),
                    return_type_ident: ret_type,
                    address: data.and_then(|d| d.address),
                };
                Some(Value::Subroutine(fn_var))
            }
            TypeDeclaration::ModifiedType {
                inner, modifier, ..
            } => {
                let in_debugee_loc = data.as_ref().and_then(|d| d.address);
                Some(Value::CModifiedVariable(CModifiedValue {
                    type_id: Some(type_id),
                    type_ident: ctx.type_graph.identity(type_id),
                    modifier: *modifier,
                    value: inner.and_then(|inner_type| {
                        Some(Box::new(self.parse_inner(ctx, data, inner_type)?))
                    }),
                    address: in_debugee_loc,
                }))
            }
        }
    }

    pub(super) fn parse_inner(
        &self,
        ctx: &ParseContext,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeId,
    ) -> Option<Value> {
        self.parse_inner_with_modifiers(ctx, data, type_id, &ValueModifiers::default())
    }

    /// Return a new value of a root type from the underlying type graph.
    ///
    /// # Arguments
    ///
    /// * `ctx`: parsing context
    /// * `bin_data`: binary value representation from debugee memory
    /// * `modifiers`: value addition info
    pub fn parse(
        self,
        ctx: &ParseContext,
        bin_data: Option<ObjectBinaryRepr>,
        modifiers: &ValueModifiers,
    ) -> Option<Value> {
        if modifiers.const_tls_duplicate {
            return None;
        }

        self.parse_inner_with_modifiers(ctx, bin_data, ctx.type_graph.root(), modifiers)
    }
}

#[inline(never)]
fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> T {
    let ptr = bytes.as_ptr();
    unsafe { std::ptr::read_unaligned::<T>(ptr as *const T) }
}
