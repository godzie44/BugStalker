use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::{DebugeeContext, EndianArcSlice};
use anyhow::{anyhow, Error};
use gimli::Range;
use nix::unistd::Pid;
use proc_maps::MapRange;
use std::collections::HashMap;
use std::path::PathBuf;

struct TextRange {
    from: RelocatedAddress,
    to: RelocatedAddress,
}

pub struct DwarfRegistry {
    pid: Pid,
    program_path: PathBuf,
    files: HashMap<PathBuf, DebugeeContext>,
    ranges: HashMap<PathBuf, TextRange>,
    /// regions map addresses, each region is a shared lib or debugee program.
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
        program_dwarf: DebugeeContext<EndianArcSlice>,
    ) -> Self {
        Self {
            pid,
            program_path: program_path.clone(),
            files: HashMap::from([(program_path, program_dwarf)]),
            ranges: HashMap::new(),
            mappings: HashMap::new(),
        }
    }

    /// Update ranges with respect of VAS segments addresses.
    ///
    /// Must called after program is loaded into memory.
    pub fn update_mappings(&mut self) -> anyhow::Result<Vec<Error>> {
        let proc_maps: Vec<MapRange> = proc_maps::get_process_maps(self.pid.as_raw())?;

        let mut mappings = HashMap::with_capacity(self.files.len());
        let mut ranges = HashMap::with_capacity(self.files.len());
        let mut errors = Vec::new();

        self.files.iter().for_each(|(file, dwarf)| {
            let absolute_debugee_path_buf =
                file.canonicalize().expect("canonicalize path must exists");
            let absolute_debugee_path = absolute_debugee_path_buf.as_path();
            let maps = proc_maps
                .iter()
                .filter(|map| map.filename() == Some(absolute_debugee_path));

            let lowest_map = maps
                .min_by(|map1, map2| map1.start().cmp(&map2.start()))
                .ok_or_else(|| anyhow!("mapping not found for {file:?}"));
            let lowest_map = match lowest_map {
                Ok(m) => m,
                Err(e) => {
                    errors.push(e);
                    return;
                }
            };

            let mapping = lowest_map.start();
            let range = dwarf.range().unwrap_or(Range { begin: 0, end: 0 });

            mappings.insert(file.clone(), mapping);
            ranges.insert(
                file.clone(),
                TextRange {
                    from: RelocatedAddress::from(range.begin as usize + mapping),
                    to: RelocatedAddress::from(range.end as usize + mapping),
                },
            );
        });

        self.mappings = mappings;
        self.ranges = ranges;

        Ok(errors)
    }

    /// Add new dwarf information.
    ///
    /// # Arguments
    ///
    /// * `file`: path to executable or shared lib
    /// * `dwarf`: parsed dwarf information
    pub fn add(&mut self, file: &str, dwarf: DebugeeContext<EndianArcSlice>) -> anyhow::Result<()> {
        let path = PathBuf::from(file);
        // validate path
        path.canonicalize()?;
        self.files.insert(file.into(), dwarf);
        Ok(())
    }

    pub fn find_by_addr(&self, addr: RelocatedAddress) -> Option<&DebugeeContext<EndianArcSlice>> {
        let file = self.ranges.iter().find_map(|(file, range)| {
            if range.from >= addr && range.to <= addr {
                return Some(file);
            }
            None
        })?;

        self.files.get(file)
    }

    /// Get program mapping offset.
    pub fn get_program_mapping(&self) -> Option<usize> {
        self.mappings.get(&self.program_path).copied()
    }

    /// Get executable program dwarf information.
    pub fn get_program_dwarf(&self) -> Option<&DebugeeContext<EndianArcSlice>> {
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
            ranges: HashMap::default(),
            mappings: HashMap::default(),
        }
    }
}
