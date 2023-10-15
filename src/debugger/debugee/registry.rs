use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::{DebugInformation, EndianArcSlice};
use crate::debugger::error::Error;
use crate::debugger::error::Error::MappingNotFound;
use nix::unistd::Pid;
use proc_maps::MapRange;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Memory region range.
#[derive(Debug, Clone)]
pub struct RegionRange {
    pub from: RelocatedAddress,
    pub to: RelocatedAddress,
}

/// Information about loaded in VAS region.
pub struct RegionInfo {
    pub path: PathBuf,
    pub has_debug_info: bool,
    pub range: Option<RegionRange>,
}

/// Difference between two registry states (typically between current and needle)
pub struct ReloadPlan {
    pub to_del: Vec<PathBuf>,
    pub to_add: Vec<PathBuf>,
}

/// Registry contains debug information about main executable object and loaded shared libraries.
pub struct DwarfRegistry {
    /// process pid
    pid: Pid,
    /// main executable file
    program_path: PathBuf,
    /// debug information map
    files: HashMap<PathBuf, DebugInformation>,
    /// ordered .text section address ranges, calculates by dwarf units ranges
    ranges: Vec<(PathBuf, RegionRange)>,
    /// regions map addresses, each region is a shared lib or debugee program
    mappings: HashMap<PathBuf, usize>,
}

impl DwarfRegistry {
    /// Create new registry.
    ///
    /// # Arguments
    ///
    /// * `pid`: program process pid
    /// * `program_path`: path to program executable
    /// * `program_dwarf`: program dwarf information
    pub fn new(
        pid: Pid,
        program_path: PathBuf,
        program_dwarf: DebugInformation<EndianArcSlice>,
    ) -> Self {
        Self {
            pid,
            program_path: program_path.clone(),
            files: HashMap::from([(program_path, program_dwarf)]),
            ranges: vec![],
            mappings: HashMap::new(),
        }
    }

    /// Update ranges with respect of VAS segments addresses.
    /// Must be called after program is loaded into memory.
    ///
    /// # Arguments
    ///
    /// * `only_main`: if true - update mappings only for main executable file, false - update all
    pub fn update_mappings(&mut self, only_main: bool) -> Result<Vec<Error>, Error> {
        let proc_maps: Vec<MapRange> = proc_maps::get_process_maps(self.pid.as_raw())?;

        let mut mappings = HashMap::with_capacity(self.files.len());
        let mut ranges = vec![];
        let mut errors = Vec::new();

        let (mut full_it, mut only_main_it);
        let iter: &mut dyn Iterator<Item = (&PathBuf, &DebugInformation)> = if only_main {
            only_main_it = self
                .files
                .iter()
                .filter(|(file, _)| *file == &self.program_path);
            &mut only_main_it
        } else {
            full_it = self.files.iter();
            &mut full_it
        };

        iter.for_each(|(file, dwarf)| {
            let absolute_debugee_path_buf =
                file.canonicalize().expect("canonicalize path must exists");
            let absolute_debugee_path = absolute_debugee_path_buf.as_path();
            let maps = proc_maps
                .iter()
                .filter(|map| map.filename() == Some(absolute_debugee_path))
                .collect::<Vec<_>>();

            if maps.is_empty() {
                errors.push(MappingNotFound(file.to_string_lossy().to_string()));
                return;
            }

            let lower_sect = maps
                .iter()
                .min_by(|map1, map2| map1.start().cmp(&map2.start()))
                .expect("at least one mapping must exists");
            let higher_sect = maps
                .iter()
                .max_by(|map1, map2| map1.start().cmp(&map2.start()))
                .expect("at least one mapping must exists");

            let mapping = lower_sect.start();
            let range = dwarf.range();

            let range = match range {
                None => RegionRange {
                    from: RelocatedAddress::from(lower_sect.start()),
                    to: RelocatedAddress::from(higher_sect.start() + higher_sect.size()),
                },
                Some(range) => RegionRange {
                    from: RelocatedAddress::from(range.begin as usize + mapping),
                    to: RelocatedAddress::from(range.end as usize + mapping),
                },
            };

            mappings.insert(file.clone(), mapping);
            ranges.push((file.clone(), range));
        });

        self.mappings = mappings;
        ranges.sort_unstable_by(|(_, r1), (_, r2)| r1.from.cmp(&r2.from));
        self.ranges = ranges;

        Ok(errors)
    }

    /// Add new debug information into registry.
    ///
    /// # Arguments
    ///
    /// * `file`: path to executable object or shared lib
    /// * `dwarf`: parsed dwarf information
    pub fn add(
        &mut self,
        file: impl Into<PathBuf>,
        dwarf: DebugInformation<EndianArcSlice>,
    ) -> Result<(), Error> {
        let path = file.into();
        // validate path
        path.canonicalize()?;
        self.files.insert(path, dwarf);
        Ok(())
    }

    pub fn remove(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        self.files.remove(path);
        self.mappings.remove(path);
        self.ranges.retain_mut(|(lib_path, _)| lib_path != path);
    }

    /// Calculate a difference between two list of paths - already parsed libs list (from this registry)
    /// and target list of libraries in argument. This difference using for create a
    /// [`ReloadPlan`] - an indication to the caller which library can be deleted from registry and which
    /// need to be parsed and add to the registry. After this plan is executed registry will contains
    /// libraries from `target` list.
    ///
    /// # Arguments
    ///
    /// * `target`: list of shared libraries paths
    ///
    /// returns: [`ReloadPlan`] - must executed at caller side
    pub fn reload_plan(&self, target: Vec<PathBuf>) -> ReloadPlan {
        let current_libs: Vec<_> = self.files.keys().cloned().collect();

        let mut to_del = vec![];
        for current_lib in current_libs {
            // program executable cannot be deleted or reloaded
            if !target.contains(&current_lib) && current_lib != self.program_path {
                to_del.push(current_lib);
            }
        }

        let mut to_add = vec![];
        for target_lib in target {
            if !self.files.contains_key(&target_lib) {
                to_add.push(target_lib);
            }
        }

        ReloadPlan { to_del, to_add }
    }

    /// Return all known debug information. Debug info about main executable object
    /// is located at the zero index, other information ordered from less compilation
    /// unit count to greatest.
    pub fn all_dwarf(&self) -> Vec<&DebugInformation> {
        let mut dwarfs: Vec<_> = self.files.values().collect();
        dwarfs.sort_unstable_by(|d1, d2| {
            if d1.pathname() == self.program_path {
                return Ordering::Less;
            };

            d1.unit_count().cmp(&d2.unit_count())
        });
        dwarfs
    }

    fn find_range(&self, addr: RelocatedAddress) -> Option<&(PathBuf, RegionRange)> {
        self.ranges
            .binary_search_by(|(_, range)| {
                if addr >= range.from && addr <= range.to {
                    Ordering::Equal
                } else if range.from > addr {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            })
            .ok()
            .map(|idx| &self.ranges[idx])
    }

    /// Return debug information that describes .text section determined by given address.
    ///
    /// # Arguments
    ///
    /// * `addr`: memory address that determine .text section.
    pub fn find_by_addr(&self, addr: RelocatedAddress) -> Option<&DebugInformation> {
        let (path, _) = self.find_range(addr)?;
        self.files.get(path)
    }

    /// Return debug information extracted from file.
    ///
    /// # Arguments
    ///
    /// * `path`: already parsed file contains debug information.
    pub fn find_by_file(&self, path: &Path) -> Option<&DebugInformation> {
        self.files.get(path)
    }

    /// Calculate virtual memory region to which the address belongs and return
    /// this region offset.
    ///
    /// # Arguments
    ///
    /// * `pc`: address for determine VAS region.
    pub fn find_mapping_offset(&self, addr: RelocatedAddress) -> Option<usize> {
        let (path, _) = self.find_range(addr)?;
        self.mappings.get(path).copied()
    }

    /// Return offset of mapped memory region.
    ///
    /// # Arguments
    ///
    /// * `dwarf`: debug information for determine memory region.
    pub fn find_mapping_offset_for_file(&self, dwarf: &DebugInformation) -> Option<usize> {
        self.mappings.get(dwarf.pathname()).copied()
    }

    /// Find main executable object debug information.
    pub fn find_main_program_dwarf(&self) -> Option<&DebugInformation> {
        self.files.get(&self.program_path)
    }

    /// Create new [`DwarfRegistry`] with same dwarf info.
    ///
    /// # Arguments
    ///
    /// * `new_pid`: new process pid
    pub fn extend(&self, new_pid: Pid) -> Self {
        Self {
            pid: new_pid,
            program_path: self.program_path.clone(),
            files: self.files.clone(),
            // mappings and ranges must be redefined
            ranges: vec![],
            mappings: HashMap::default(),
        }
    }

    /// Return a ordered list of mapped regions (main executable region at first place).
    pub fn dump(&self) -> Vec<RegionInfo> {
        let mut regions: Vec<_> = self
            .files
            .iter()
            .map(|(path, debug_info)| {
                let file_range = self.ranges.iter().find(|(p, _)| path == p);
                RegionInfo {
                    has_debug_info: debug_info.has_debug_info(),
                    path: path.clone(),
                    range: file_range.map(|(_, range)| range.clone()),
                }
            })
            .collect();

        regions.sort_unstable_by(|i1, i2| {
            if i1.path == self.program_path {
                return Ordering::Less;
            };
            i1.path.cmp(&i2.path)
        });
        regions
    }
}
