use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::{DebugeeContext, EndianArcSlice};
use anyhow::anyhow;
use gimli::Range;
use nix::unistd::Pid;
use proc_maps::MapRange;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct DwarfRegistry {
    pid: Pid,
    program_path: String,
    files: HashMap<String, DebugeeContext<EndianArcSlice>>,
    ranges: HashMap<String, Range>,
    /// regions map addresses, each region is a shared lib or debugee program.
    mappings: HashMap<String, usize>,
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
        let program = program_path.to_str().unwrap_or_default().to_string();
        Self {
            pid,
            program_path: program.clone(),
            files: HashMap::from([(program, program_dwarf)]),
            ranges: HashMap::new(),
            mappings: HashMap::new(),
        }
    }

    /// Update ranges with respect of VAS segments addresses.
    ///
    /// Must called after program is loaded into memory.
    pub fn update_mappings(&mut self) -> anyhow::Result<()> {
        let proc_maps: Vec<MapRange> = proc_maps::get_process_maps(self.pid.as_raw())?;

        let mut mappings = HashMap::with_capacity(self.files.len());
        let mut ranges = HashMap::with_capacity(self.files.len());

        self.files
            .iter()
            .try_for_each(|(file, dwarf)| -> anyhow::Result<()> {
                let absolute_debugee_path_buf = PathBuf::from(file).canonicalize()?;
                let absolute_debugee_path = absolute_debugee_path_buf.as_path();
                let maps = proc_maps
                    .iter()
                    .filter(|map| map.filename() == Some(absolute_debugee_path));

                let lowest_map = maps
                    .min_by(|map1, map2| map1.start().cmp(&map2.start()))
                    .ok_or_else(|| anyhow!("mapping not found for {file}"))?;

                let mapping = lowest_map.start();
                let mut range = dwarf.range().ok_or(anyhow!(
                    "determine range fail, cannot process dwarf from {file}"
                ))?;
                range.begin += mapping as u64;
                range.end += mapping as u64;

                mappings.insert(file.to_string(), mapping);
                ranges.insert(file.to_string(), range);

                Ok(())
            })?;

        self.mappings = mappings;
        self.ranges = ranges;

        Ok(())
    }

    /// Add new dwarf information.
    ///
    /// # Arguments
    ///
    /// * `file`: path to executable or shared lib
    /// * `dwarf`: parsed dwarf information
    pub fn add(&mut self, file: &str, dwarf: DebugeeContext<EndianArcSlice>) {
        self.files.insert(file.into(), dwarf);
    }

    pub fn find_by_addr(&self, addr: RelocatedAddress) -> Option<&DebugeeContext<EndianArcSlice>> {
        let file = self.ranges.iter().find_map(|(file, range)| {
            if range.begin >= addr.as_u64() && range.end <= addr.as_u64() {
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

    /// Get program dwarf information.
    pub fn get_program_dwarf(&self) -> &DebugeeContext<EndianArcSlice> {
        self.files
            .get(&self.program_path)
            .expect("dwarf info must exists")
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
