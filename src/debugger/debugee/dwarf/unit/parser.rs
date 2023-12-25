use crate::debugger::debugee::dwarf::unit::{
    ArrayDie, ArraySubrangeDie, AtomicDie, BaseTypeDie, ConstTypeDie, DieAttributes, DieRange,
    DieRef, DieVariant, Entry, EnumTypeDie, EnumeratorDie, FunctionDie, InlineSubroutineDie,
    LexicalBlockDie, LineRow, Namespace, Node, ParameterDie, PointerType, RestrictDie,
    StructTypeDie, SubroutineDie, TemplateTypeParameter, TypeDefDie, TypeMemberDie, UnionTypeDie,
    Unit, UnitLazyPart, UnitProperties, VariableDie, Variant, VariantPart, VolatileDie,
};
use crate::debugger::debugee::dwarf::utils::PathSearchIndex;
use crate::debugger::debugee::dwarf::{EndianArcSlice, NamespaceHierarchy};
use crate::debugger::error::Error;
use crate::debugger::rust::Environment;
use fallible_iterator::FallibleIterator;
use gimli::{
    AttributeValue, DW_AT_address_class, DW_AT_byte_size, DW_AT_call_column, DW_AT_call_file,
    DW_AT_call_line, DW_AT_const_value, DW_AT_count, DW_AT_data_member_location, DW_AT_decl_file,
    DW_AT_decl_line, DW_AT_discr, DW_AT_discr_value, DW_AT_encoding, DW_AT_frame_base,
    DW_AT_linkage_name, DW_AT_location, DW_AT_lower_bound, DW_AT_name, DW_AT_type,
    DW_AT_upper_bound, Range, Reader, UnitHeader, UnitOffset,
};
use once_cell::sync::OnceCell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::PathBuf;
use uuid::Uuid;

pub struct DwarfUnitParser<'a> {
    dwarf: &'a gimli::Dwarf<EndianArcSlice>,
}

impl<'a> DwarfUnitParser<'a> {
    pub fn new(dwarf: &'a gimli::Dwarf<EndianArcSlice>) -> Self {
        Self { dwarf }
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

        Ok(Unit {
            header: RefCell::new(Some(header)),
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
        let mut die_offsets_index: HashMap<UnitOffset, usize> = HashMap::new();
        let mut function_index = PathSearchIndex::new("::");

        let mut cursor = unit.entries();
        while let Some((delta_depth, die)) = cursor.next_dfs()? {
            let current_idx = entries.len();
            let prev_index = if entries.is_empty() {
                None
            } else {
                Some(entries.len() - 1)
            };

            let name = die
                .attr(DW_AT_name)?
                .and_then(|attr| self.dwarf.attr_string(&unit, attr.value()).ok());

            let parent_idx = match delta_depth {
                // if 1 then previous die is a parent
                1 => prev_index,
                // if 0 then previous die is a sibling
                0 => entries.last().and_then(|dd| dd.node.parent),
                // if < 0 then parent of previous die is a sibling
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

            let mut base_attrs = DieAttributes {
                _tag: die.tag(),
                name: name
                    .map(|s| s.to_string_lossy().map(|s| s.to_string()))
                    .transpose()?,
                ranges,
            };

            let parsed_die = match die.tag() {
                gimli::DW_TAG_subprogram => {
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

                    let fn_ns = match mb_linkage_name {
                        Some(linkage_name) => {
                            let linkage_name = linkage_name.to_string_lossy()?;
                            let (ns, fn_name) = NamespaceHierarchy::from_mangled(&linkage_name);
                            // assume that function name from linkage name is better
                            base_attrs.name = Some(fn_name);
                            ns
                        }
                        None => NamespaceHierarchy::for_node(
                            &Node {
                                parent: parent_idx,
                                children: vec![],
                            },
                            &entries,
                        ),
                    };

                    if let Some(ref fn_name) = base_attrs.name {
                        // subroutine without range or declaration line are useless for this index
                        if !base_attrs.ranges.is_empty() || decl_file_line.is_some() {
                            function_index.insert_w_head(fn_ns.iter(), fn_name, current_idx);
                        }
                    }

                    DieVariant::Function(FunctionDie {
                        namespace: fn_ns,
                        base_attributes: base_attrs,
                        fb_addr: die.attr(DW_AT_frame_base)?,
                        decl_file_line,
                    })
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
                gimli::DW_TAG_formal_parameter => DieVariant::Parameter(ParameterDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                    location: die.attr(DW_AT_location)?,
                }),
                gimli::DW_TAG_variable => {
                    let mut lexical_block_idx = None;
                    let mut fn_block_idx = None;

                    let mut mb_parent_idx = parent_idx;
                    while let Some(parent_idx) = mb_parent_idx {
                        if let DieVariant::LexicalBlock(_) = entries[parent_idx].die {
                            if lexical_block_idx.is_none() {
                                // save closest lexical block and ignore others
                                lexical_block_idx = Some(parent_idx);
                            }
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
                        None => NamespaceHierarchy::for_node(
                            &Node {
                                parent: parent_idx,
                                children: vec![],
                            },
                            &entries,
                        ),
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

                    DieVariant::BaseType(BaseTypeDie {
                        base_attributes: base_attrs,
                        encoding,
                        byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    })
                }
                gimli::DW_TAG_structure_type => DieVariant::StructType(StructTypeDie {
                    base_attributes: base_attrs,
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                }),
                gimli::DW_TAG_member => DieVariant::TypeMember(TypeMemberDie {
                    base_attributes: base_attrs,
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    location: die.attr(DW_AT_data_member_location)?,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                }),
                gimli::DW_TAG_union_type => DieVariant::UnionTypeDie(UnionTypeDie {
                    base_attributes: base_attrs,
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                }),
                gimli::DW_TAG_lexical_block => DieVariant::LexicalBlock(LexicalBlockDie {
                    base_attributes: base_attrs,
                }),
                gimli::DW_TAG_array_type => DieVariant::ArrayType(ArrayDie {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                }),
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
                gimli::DW_TAG_pointer_type => DieVariant::PointerType(PointerType {
                    base_attributes: base_attrs,
                    type_ref: die.attr(DW_AT_type)?.and_then(DieRef::from_attr),
                    address_class: die.attr(DW_AT_address_class)?.and_then(|v| v.udata_value()),
                }),
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
            die_offsets_index,
            function_index,
        })
    }
}

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

        lines.push(LineRow {
            address: line_row.address(),
            file_index: line_row.file_index(),
            line: line_row.line().map(NonZeroU64::get).unwrap_or(0),
            column,
            is_stmt: line_row.is_stmt(),
            prolog_end: line_row.prologue_end(),
            epilog_begin: line_row.epilogue_begin(),
        })
    }
    Ok(lines)
}

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

    Ok(files)
}

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

    if file.directory_index() != 0 {
        if let Some(directory) = file.directory(header) {
            path.push(
                sections
                    .attr_string(dw_unit, directory)?
                    .to_string_lossy()?
                    .as_ref(),
            );
        }
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
