use crate::debugger::debugee::dwarf::EndianArcSlice;
use crate::debugger::error::Error;
use gimli::{
    AbbreviationsCache, DebugAbbrev, DebugAddr, DebugAranges, DebugInfo, DebugLine, DebugLineStr,
    DebugLoc, DebugLocLists, DebugRanges, DebugRngLists, DebugStr, DebugStrOffsets, DebugTypes,
    Dwarf, DwarfFileType, LocationLists, RangeLists, RunTimeEndian, Section, SectionId,
};
use object::{File, Object, ObjectSection};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::Mutex;

/// List of required sections for create a [`gimli::Dwarf`]
#[derive(Default)]
struct Sections {
    debug_abbrev: Option<DebugAbbrev<EndianArcSlice>>,
    debug_addr: Option<DebugAddr<EndianArcSlice>>,
    debug_aranges: Option<DebugAranges<EndianArcSlice>>,
    debug_info: Option<DebugInfo<EndianArcSlice>>,
    debug_line: Option<DebugLine<EndianArcSlice>>,
    debug_line_str: Option<DebugLineStr<EndianArcSlice>>,
    debug_str: Option<DebugStr<EndianArcSlice>>,
    debug_str_offsets: Option<DebugStrOffsets<EndianArcSlice>>,
    debug_types: Option<DebugTypes<EndianArcSlice>>,
    debug_loc: Option<DebugLoc<EndianArcSlice>>,
    debug_loclists: Option<DebugLocLists<EndianArcSlice>>,
    debug_ranges: Option<DebugRanges<EndianArcSlice>>,
    debug_rnglists: Option<DebugRngLists<EndianArcSlice>>,
}

pub fn load_section(
    id: SectionId,
    file: &File,
    endian: RunTimeEndian,
) -> Result<EndianArcSlice, Error> {
    let data = file
        .section_by_name(id.name())
        .and_then(|section| section.uncompressed_data().ok())
        .unwrap_or(Cow::Borrowed(&[]));
    Ok(gimli::EndianArcSlice::new(Arc::from(&*data), endian))
}

/// Create a function that load section and put in [`Sections`] struct in right place.
macro_rules! make_sect_loader {
    ($file: expr, $endian: expr, $field: tt) => {{
        move |dest: Arc<Mutex<Option<Sections>>>| -> Result<(), Error> {
            let sect = Section::load(|id| load_section(id, $file, $endian))?;
            let mut lock = dest.lock().expect("unexpected: panic in another lock");
            let sections = lock.as_mut().expect("unexpected: sections must exists");
            sections.$field = Some(sect);
            Ok(())
        }
    }};
}

/// Load debug information from file. For better loading time all sections
/// loads in parallel inside a thread pool.
///
/// # Arguments
///
/// * `file`: object file with debug information
/// * `endian`: file endian
pub fn load_par(file: &File, endian: RunTimeEndian) -> Result<Dwarf<EndianArcSlice>, Error> {
    let load_debug_abbrev = make_sect_loader!(file, endian, debug_abbrev);
    let load_debug_addr = make_sect_loader!(file, endian, debug_addr);
    let load_debug_aranges = make_sect_loader!(file, endian, debug_aranges);
    let load_debug_info = make_sect_loader!(file, endian, debug_info);
    let load_debug_line = make_sect_loader!(file, endian, debug_line);
    let load_debug_line_str = make_sect_loader!(file, endian, debug_line_str);
    let load_debug_str = make_sect_loader!(file, endian, debug_str);
    let load_debug_str_offsets = make_sect_loader!(file, endian, debug_str_offsets);
    let load_debug_types = make_sect_loader!(file, endian, debug_types);
    let load_debug_loc = make_sect_loader!(file, endian, debug_loc);
    let load_debug_loclists = make_sect_loader!(file, endian, debug_loclists);
    let load_debug_ranges = make_sect_loader!(file, endian, debug_ranges);
    let load_debug_rnglists = make_sect_loader!(file, endian, debug_rnglists);

    type SectLoaders<'a> =
        Vec<Box<dyn FnOnce(Arc<Mutex<Option<Sections>>>) -> Result<(), Error> + Send + Sync + 'a>>;

    let loaders: SectLoaders = vec![
        Box::new(load_debug_abbrev),
        Box::new(load_debug_addr),
        Box::new(load_debug_aranges),
        Box::new(load_debug_info),
        Box::new(load_debug_line),
        Box::new(load_debug_line_str),
        Box::new(load_debug_str),
        Box::new(load_debug_str_offsets),
        Box::new(load_debug_types),
        Box::new(load_debug_loc),
        Box::new(load_debug_loclists),
        Box::new(load_debug_ranges),
        Box::new(load_debug_rnglists),
    ];

    let sections = Arc::new(Mutex::new(Some(Sections::default())));
    loaders
        .into_par_iter()
        .try_for_each(|loader| loader(sections.clone()))?;

    // at this moment all sections must be loaded
    let sections = sections
        .lock()
        .expect("unexpected: panic in another lock")
        .take()
        .expect("unexpected: sections must exists");

    const SECT_MUST_EXISTS: &str = "section must exists";
    Ok(Dwarf {
        debug_abbrev: sections.debug_abbrev.expect(SECT_MUST_EXISTS),
        debug_addr: sections.debug_addr.expect(SECT_MUST_EXISTS),
        debug_aranges: sections.debug_aranges.expect(SECT_MUST_EXISTS),
        debug_info: sections.debug_info.expect(SECT_MUST_EXISTS),
        debug_line: sections.debug_line.expect(SECT_MUST_EXISTS),
        debug_line_str: sections.debug_line_str.expect(SECT_MUST_EXISTS),
        debug_str: sections.debug_str.expect(SECT_MUST_EXISTS),
        debug_str_offsets: sections.debug_str_offsets.expect(SECT_MUST_EXISTS),
        debug_types: sections.debug_types.expect(SECT_MUST_EXISTS),
        locations: LocationLists::new(
            sections.debug_loc.expect(SECT_MUST_EXISTS),
            sections.debug_loclists.expect(SECT_MUST_EXISTS),
        ),
        ranges: RangeLists::new(
            sections.debug_ranges.expect(SECT_MUST_EXISTS),
            sections.debug_rnglists.expect(SECT_MUST_EXISTS),
        ),
        file_type: DwarfFileType::Main,
        sup: None,
        abbreviations_cache: AbbreviationsCache::new(),
    })
}
