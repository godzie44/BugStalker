use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::{DebugInformation, EndianArcSlice};
use anyhow::{anyhow, Error};
use nix::unistd::Pid;
use proc_maps::MapRange;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug)]
struct TextRange {
    from: RelocatedAddress,
    to: RelocatedAddress,
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
    ranges: Vec<(PathBuf, TextRange)>,
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
    pub fn update_mappings(&mut self, only_main: bool) -> anyhow::Result<Vec<Error>> {
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
                errors.push(anyhow!("mapping not found for {file:?}"));
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
                None => TextRange {
                    from: RelocatedAddress::from(lower_sect.start()),
                    to: RelocatedAddress::from(higher_sect.start() + higher_sect.size()),
                },
                Some(range) => TextRange {
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
        file: &str,
        dwarf: DebugInformation<EndianArcSlice>,
    ) -> anyhow::Result<()> {
        let path = PathBuf::from(file);
        // validate path
        path.canonicalize()?;
        self.files.insert(file.into(), dwarf);
        Ok(())
    }

    /// Return all known debug information. Debug info about main executable object
    /// is located at the zero index.
    pub fn all_dwarf(&self) -> Vec<&DebugInformation> {
        let mut dwarfs: Vec<_> = self.files.values().collect();
        dwarfs.sort_unstable_by(|d1, d2| {
            if d1.pathname() == self.program_path {
                return Ordering::Less;
            };
            d1.pathname().cmp(&d2.file)
        });
        dwarfs
    }

    fn find_range(&self, addr: RelocatedAddress) -> Option<&(PathBuf, TextRange)> {
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
}
