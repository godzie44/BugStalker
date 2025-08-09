use crate::debugger::debugee::dwarf::unit::{
    ArrayDie, ArraySubrangeDie, AtomicDie, BaseTypeDie, ConstTypeDie, DieAttributes, DieRange,
    DieRef, DieVariant, END_SEQUENCE, EPILOG_BEGIN, Entry, EnumTypeDie, EnumeratorDie, FunctionDie,
    IS_STMT, InlineSubroutineDie, LexicalBlockDie, LineRow, Namespace, Node, PROLOG_END,
    ParameterDie, PointerType, RestrictDie, StructTypeDie, SubroutineDie, TemplateTypeParameter,
    TypeDefDie, TypeMemberDie, UnionTypeDie, Unit, UnitLazyPart, UnitProperties, VariableDie,
    Variant, VariantPart, VolatileDie,
};
use crate::debugger::debugee::dwarf::utils::PathSearchIndex;
use crate::debugger::debugee::dwarf::{EndianArcSlice, NamespaceHierarchy};
use crate::debugger::error::Error;
use crate::debugger::rust::Environment;
use crate::weak_error;
use fallible_iterator::FallibleIterator;
use gimli::{
    AttributeValue, DW_AT_address_class, DW_AT_byte_size, DW_AT_call_column, DW_AT_call_file,
    DW_AT_call_line, DW_AT_const_value, DW_AT_count, DW_AT_data_member_location, DW_AT_decl_file,
    DW_AT_decl_line, DW_AT_declaration, DW_AT_discr, DW_AT_discr_value, DW_AT_encoding,
    DW_AT_frame_base, DW_AT_language, DW_AT_linkage_name, DW_AT_location, DW_AT_lower_bound,
    DW_AT_name, DW_AT_producer, DW_AT_specification, DW_AT_type, DW_AT_upper_bound,
    DebuggingInformationEntry, DwAt, Range, Reader, UnitHeader, UnitOffset,
};
use log::warn;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

pub struct DwarfUnitParser<'a> {
    dwarf: &'a gimli::Dwarf<EndianArcSlice>,
}

impl<'a> DwarfUnitParser<'a> {
    pub fn new(dwarf: &'a gimli::Dwarf<EndianArcSlice>) -> Self {
        Self { dwarf }
    }

    fn attr_to_string(
        &self,
        unit: &gimli::Unit<EndianArcSlice, usize>,
        die: &DebuggingInformationEntry<EndianArcSlice, usize>,
        attr: DwAt,
    ) -> gimli::Result<Option<String>> {
        die.attr(attr)?
            .and_then(|attr| self.dwarf.attr_string(unit, attr.value()).ok())
            .map(|l| l.to_string_lossy().map(|s| s.to_string()))
            .transpose()
    }

    pub fn parse(&self, header: UnitHeader<EndianArcSlice>) -> gimli::Result<Unit> {
        let unit = self.dwarf.unit(header.clone())?;

        let name = unit
            .name
            .as_ref()
            .and_then(|n| n.to_string_lossy().ok().map(|s| s.to_string()));

        let mut files = vec![];
        let mut lines = vec![];
        if let Some(ref lp) = unit.line_program {
            let mut rows = lp.clone().rows();
            lines = parse_lines(&mut rows)?;
            files = parse_files(self.dwarf, &unit, &rows)?;
        }
        lines.sort_unstable_by_key(|x| x.address);

        let mut ranges = self.dwarf.unit_ranges(&unit)?.collect::<Vec<_>>()?;
        ranges.sort_unstable_by_key(|r| r.begin);

        let mut cursor = unit.header.entries(&unit.abbreviations);
        cursor.next_dfs()?;
        let root = cursor.current().ok_or(gimli::Error::MissingUnitDie)?;

        let language = root.attr(DW_AT_language)?.and_then(|attr| {
            if let AttributeValue::Language(lang) = attr.value() {
                return Some(lang);
            }
            None
        });
        let producer = self.attr_to_string(&unit, root, DW_AT_producer)?;

        Ok(Unit {
            header: Mutex::new(Some(header)),
            idx: usize::MAX,
            properties: UnitProperties {
                encoding: unit.encoding(),
                offset: unit.header.offset().as_debug_info_offset(),
                low_pc: unit.low_pc,
                addr_base: unit.addr_base,
                loclists_base: unit.loclists_base,
                address_size: unit.header.address_size(),
            },
            id: Uuid::new_v4(),
            name,
            files,
            lines,
            ranges,
            lazy_part: OnceCell::new(),
            language,
            producer,
        })
    }

    pub(super) fn parse_additional(
        &self,
        header: UnitHeader<EndianArcSlice>,
    ) -> Result<UnitLazyPart, Error> {
        let unit = self.dwarf.unit(header)?;

        let mut entries: Vec<Entry> = vec![];
        let mut die_ranges: Vec<DieRange> = vec![];
        let mut variable_index: HashMap<String, Vec<(NamespaceHierarchy, usize)>> = HashMap::new();
        let mut type_index: HashMap<String, UnitOffset> = HashMap::new();
        let mut die_offsets_index: HashMap<UnitOffset, usize> = HashMap::new();
        let mut function_index = PathSearchIndex::new("::");
        let mut fn_declarations = HashMap::new();

        let mut cursor = unit.entries();
        while let Some((delta_depth, die)) = cursor.next_dfs()? {
            let current_idx = entries.len();
            let prev_index = if entries.is_empty() {
                None
            } else {
                Some(entries.len() - 1)
            };

            let parent_idx = match delta_depth {
                // if 1 then previous die is a parent
                1 => prev_index,
                // if 0, then previous die is a sibling
                0 => entries.last().and_then(|dd| dd.node.parent),
                // if < 0 then the parent of previous die is a sibling
                mut x if x < 0 => {
                    let mut parent = entries.last().unwrap();
                    while x != 0 {
                        parent = &entries[parent.node.parent.unwrap()];
                        x += 1;
                    }
                    parent.node.parent
                }
                _ => unreachable!(),
            };

            if let Some(parent_idx) = parent_idx {
                entries[parent_idx].node.children.push(current_idx)
            }

            let ranges: Box<[Range]> = self
                .dwarf
                .die_ranges(&unit, die)?
                .collect::<Vec<Range>>()?
                .into();

            ranges.iter().for_each(|r| {
                die_ranges.push(DieRange {
                    range: *r,
                    die_idx: current_idx,
                })
            });

            let name = self.attr_to_string(&unit, die, DW_AT_name)?;
            let base_attrs = DieAttributes { name, ranges };

            let parsed_die = match die.tag() {
                gimli::DW_TAG_subprogram => {
                    let is_declaration = die.attr(DW_AT_declaration)?.and_then(|attr| {
                        if let AttributeValue::Flag(f) = attr.value() {
                            return Some(f);
                        }
                        None
                    });
                    if is_declaration == Some(true) {
                        // add declaration in a special map, this die's may be used later, when
                        // parse implementation
                        fn_declarations.insert(die.offset(), current_idx);
                    }

                    let mb_file = die
                        .attr(DW_AT_decl_file)?
                        .and_then(|attr| attr.udata_value());
                    let mb_line = die
                        .attr(DW_AT_decl_line)?
                        .and_then(|attr| attr.udata_value());
                    let decl_file_line = mb_file.and_then(|file_idx| Some((file_idx, mb_line?)));

                    let mb_linkage_name = die
                        .attr(DW_AT_linkage_name)?
                        .and_then(|attr| self.dwarf.attr_string(&unit, attr.value()).ok());

                    let (fn_ns, linkage_name) = match mb_linkage_name {
                        Some(linkage_name) => {
                            let linkage_name = linkage_name.to_string_lossy()?;
                            let (ns, fn_name) = NamespaceHierarchy::from_mangled(&linkage_name);
                            (ns, Some(fn_name))
                        }
                        None => (
                            NamespaceHierarchy::for_node(&Node::new_leaf(parent_idx), &entries),
                            None,
                        ),
                    };

                    let mut fn_die = FunctionDie {
                        namespace: fn_ns,
                        base_attributes: base_attrs,
                        fb_addr: die.attr(DW_AT_frame_base)?,
                        decl_file_line,
                        linkage_name,
                    };

                    let specification = die.attr(DW_AT_specification)?.and_then(|attr| {
                        if let AttributeValue::UnitRef(r) = attr.value() {
                            return Some(r);
                        }
                        warn!(target: "parser", "unexpected non-local (unit) reference to function declaration");
                        None
                    });
                    if let Some(decl_ref) = specification {
                        let declaration_idx = weak_error!(
                            fn_declarations
                                .get(&decl_ref)
                                .ok_or(Error::InvalidSpecification(decl_ref))
                        );
                        debug_assert!(declaration_idx.is_some(), "reference to unseen declaration");

                        if let Some(&idx) = declaration_idx {
                            let declaration = &entries[idx];
                            let declaration = declaration.die.unwrap_function();
                            fn_die.complete_from_decl(declaration);
                        }
                    }

                    let any_name = fn_die
                        .linkage_name
                        .as_ref()
                        .or(fn_die.base_attributes.name.as_ref());
                    if let Some(fn_name) = any_name {
                        // subprograms without a range are useless for this index
                        if !fn_die.base_attributes.ranges.is_empty() {
                            function_index.insert_w_head(
                                fn_die.namespace.iter(),
                                fn_name,
                                current_idx,
                            );
                        }
                    }

                    DieVariant::Function(fn_die)
                }
                gimli::DW_TAG_subroutine_type => DieVariant::Subroutine(SubroutineDie {
                    base_attributes: base_attrs,
                    return_type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_inlined_subroutine => {
                    DieVariant::InlineSubroutine(InlineSubroutineDie {
                        base_attributes: base_attrs,
                        call_file: die.attr(DW_AT_call_file)?.and_then(|v| match v.value() {
                            AttributeValue::FileIndex(idx) => Some(idx),
                            _ => None,
                        }),
                        call_line: die.attr(DW_AT_call_line)?.and_then(|v| v.udata_value()),
                        call_column: die.attr(DW_AT_call_column)?.and_then(|v| v.udata_value()),
                    })
                }
                gimli::DW_TAG_formal_parameter => {
                    let mut fn_block_idx = None;
                    let mut mb_parent_idx = parent_idx;
                    while let Some(parent_idx) = mb_parent_idx {
                        if let DieVariant::Function(_) = entries[parent_idx].die {
                            fn_block_idx = Some(parent_idx);
                            break;
                        }
                        mb_parent_idx = entries[parent_idx].node.parent;
                    }

                    DieVariant::Parameter(ParameterDie {
                        base_attributes: base_attrs,
                        type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                        location: die.attr(DW_AT_location)?,
                        fn_block_idx,
                    })
                }
                gimli::DW_TAG_variable => {
                    let mut lexical_block_idx = None;
                    let mut fn_block_idx = None;

                    let mut mb_parent_idx = parent_idx;
                    while let Some(parent_idx) = mb_parent_idx {
                        if let DieVariant::LexicalBlock(_) = entries[parent_idx].die
                            && lexical_block_idx.is_none()
                        {
                            // save the closest lexical block and ignore others
                            lexical_block_idx = Some(parent_idx);
                        }
                        if let DieVariant::Function(_) = entries[parent_idx].die {
                            fn_block_idx = Some(parent_idx);
                            break;
                        }
                        mb_parent_idx = entries[parent_idx].node.parent;
                    }

                    let mb_linkage_name = die
                        .attr(DW_AT_linkage_name)?
                        .and_then(|attr| self.dwarf.attr_string(&unit, attr.value()).ok());

                    let variable_ns = match mb_linkage_name {
                        Some(linkage_name) => {
                            let linkage_name = linkage_name.to_string_lossy()?;
                            let (ns, _) = NamespaceHierarchy::from_mangled(&linkage_name);
                            ns
                        }
                        None => NamespaceHierarchy::for_node(&Node::new_leaf(parent_idx), &entries),
                    };

                    let die = VariableDie {
                        base_attributes: base_attrs,
                        type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                        location: die.attr(DW_AT_location)?,
                        lexical_block_idx,
                        fn_block_idx,
                    };

                    if let Some(ref name) = die.base_attributes.name {
                        variable_index
                            .entry(name.to_string())
                            .or_default()
                            .push((variable_ns, current_idx));
                    }

                    DieVariant::Variable(die)
                }
                gimli::DW_TAG_base_type => {
                    let encoding = die.attr(DW_AT_encoding)?.and_then(|attr| {
                        if let AttributeValue::Encoding(enc) = attr.value() {
                            Some(enc)
                        } else {
                            None
                        }
                    });

                    if let Some(ref name) = base_attrs.name {
                        type_index.insert(name.to_string(), die.offset());
                    }
                    DieVariant::BaseType(BaseTypeDie {
                        base_attributes: base_attrs,
                        encoding,
                        byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    })
                }
                gimli::DW_TAG_structure_type => {
                    if let Some(ref name) = base_attrs.name {
                        type_index.insert(name.to_string(), die.offset());
                    }
                    DieVariant::StructType(StructTypeDie {
                        base_attributes: base_attrs,
                        byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    })
                }
                gimli::DW_TAG_member => DieVariant::TypeMember(TypeMemberDie {
                    base_attributes: base_attrs,
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    location: die.attr(DW_AT_data_member_location)?,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_union_type => {
                    if let Some(ref name) = base_attrs.name {
                        type_index.insert(name.to_string(), die.offset());
                    }
                    DieVariant::UnionTypeDie(UnionTypeDie {
                        base_attributes: base_attrs,
                        byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    })
                }
                gimli::DW_TAG_lexical_block => DieVariant::LexicalBlock(LexicalBlockDie {
                    base_attributes: base_attrs,
                }),
                gimli::DW_TAG_array_type => {
                    if let Some(ref name) = base_attrs.name {
                        type_index.insert(name.to_string(), die.offset());
                    }
                    DieVariant::ArrayType(ArrayDie {
                        base_attributes: base_attrs,
                        type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                        byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    })
                }
                gimli::DW_TAG_subrange_type => DieVariant::ArraySubrange(ArraySubrangeDie {
                    base_attributes: base_attrs,
                    lower_bound: die.attr(DW_AT_lower_bound)?,
                    upper_bound: die.attr(DW_AT_upper_bound)?,
                    count: die.attr(DW_AT_count)?,
                }),
                gimli::DW_TAG_enumeration_type => DieVariant::EnumType(EnumTypeDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                }),
                gimli::DW_TAG_enumerator => DieVariant::Enumerator(EnumeratorDie {
                    base_attributes: base_attrs,
                    const_value: die
                        .attr(DW_AT_const_value)?
                        .and_then(|val| val.sdata_value()),
                }),
                gimli::DW_TAG_variant_part => DieVariant::VariantPart(VariantPart {
                    base_attributes: base_attrs,
                    discr_ref: die.attr(DW_AT_discr)?.and_then(DieRef::from_attr),
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_variant => DieVariant::Variant(Variant {
                    base_attributes: base_attrs,
                    discr_value: die
                        .attr(DW_AT_discr_value)?
                        .and_then(|val| val.sdata_value()),
                }),
                gimli::DW_TAG_pointer_type => {
                    if let Some(ref name) = base_attrs.name {
                        type_index.insert(name.to_string(), die.offset());
                    }
                    DieVariant::PointerType(PointerType {
                        base_attributes: base_attrs,
                        type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                        address_class: die
                            .attr(DW_AT_address_class)?
                            .and_then(|v| v.udata_value().map(|u| u as u8)),
                    })
                }
                gimli::DW_TAG_template_type_parameter => {
                    DieVariant::TemplateType(TemplateTypeParameter {
                        base_attributes: base_attrs,
                        type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                    })
                }
                gimli::DW_TAG_typedef => DieVariant::TypeDef(TypeDefDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_const_type => DieVariant::ConstType(ConstTypeDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_atomic_type => DieVariant::Atomic(AtomicDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_volatile_type => DieVariant::Volatile(VolatileDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_restrict_type => DieVariant::Restrict(RestrictDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_namespace => DieVariant::Namespace(Namespace {
                    base_attributes: base_attrs,
                }),
                _ => DieVariant::Default(base_attrs),
            };

            entries.push(Entry::new(parsed_die, parent_idx));
            die_offsets_index.insert(die.offset(), current_idx);
        }
        die_ranges.sort_unstable_by_key(|dr| dr.range.begin);

        Ok(UnitLazyPart {
            entries,
            die_ranges,
            variable_index,
            type_index,
            die_offsets_index,
            function_index,
        })
    }
}

#[inline(always)]
fn parse_lines<R, Offset>(
    rows: &mut gimli::LineRows<R, gimli::IncompleteLineProgram<R, Offset>, Offset>,
) -> gimli::Result<Vec<LineRow>>
where
    R: Reader<Offset = Offset>,
    Offset: gimli::ReaderOffset,
{
    let mut lines = vec![];
    while let Some((_, line_row)) = rows.next_row()? {
        let column = match line_row.column() {
            gimli::ColumnType::LeftEdge => 0,
            gimli::ColumnType::Column(x) => x.get(),
        };

        let mut flags = 0_u8;
        if line_row.is_stmt() {
            flags |= IS_STMT;
        }
        if line_row.prologue_end() {
            flags |= PROLOG_END;
        }
        if line_row.epilogue_begin() {
            flags |= EPILOG_BEGIN;
        }
        if line_row.end_sequence() {
            flags |= END_SEQUENCE;
        }

        lines.push(LineRow {
            address: line_row.address(),
            file_index: line_row.file_index(),
            line: line_row.line().map(NonZeroU64::get).unwrap_or(0),
            column,
            flags,
        })
    }

    lines.shrink_to_fit();
    Ok(lines)
}

#[inline(always)]
fn parse_files<R, Offset>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    rows: &gimli::LineRows<R, gimli::IncompleteLineProgram<R, Offset>, Offset>,
) -> gimli::Result<Vec<PathBuf>>
where
    R: Reader<Offset = Offset>,
    Offset: gimli::ReaderOffset,
{
    let mut files = vec![];
    let header = rows.header();
    match header.file(0) {
        Some(file) => files.push(render_file_path(unit, file, header, dwarf)?),
        None => files.push(PathBuf::default()),
    }
    let mut index = 1;
    while let Some(file) = header.file(index) {
        files.push(render_file_path(unit, file, header, dwarf)?);
        index += 1;
    }

    files.shrink_to_fit();
    Ok(files)
}

#[inline(always)]
fn render_file_path<R: Reader>(
    dw_unit: &gimli::Unit<R>,
    file: &gimli::FileEntry<R, R::Offset>,
    header: &gimli::LineProgramHeader<R, R::Offset>,
    sections: &gimli::Dwarf<R>,
) -> Result<PathBuf, gimli::Error> {
    let mut path = if let Some(ref comp_dir) = dw_unit.comp_dir {
        PathBuf::from(comp_dir.to_string_lossy()?.as_ref())
    } else {
        PathBuf::new()
    };

    if file.directory_index() != 0
        && let Some(directory) = file.directory(header)
    {
        path.push(
            sections
                .attr_string(dw_unit, directory)?
                .to_string_lossy()?
                .as_ref(),
        );
    }

    if path.starts_with("/rustc/") {
        let rust_env = Environment::current();
        if let Some(ref std_lib_path) = rust_env.std_lib_path {
            let mut new_path = std_lib_path.clone();
            path.iter().skip(3).for_each(|part| {
                new_path.push(part);
            });
            path = new_path;
        }
    }

    path.push(
        sections
            .attr_string(dw_unit, file.path_name())?
            .to_string_lossy()?
            .as_ref(),
    );

    Ok(path)
}
