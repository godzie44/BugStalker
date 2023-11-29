use crate::debugger;
use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::debugee::dwarf::{ContextualDieRef, DebugInformation};
use crate::debugger::debugee::Debugee;
use crate::debugger::{Error, FunctionDie};
use capstone::prelude::*;
use lru::LruCache;
use std::cell::RefCell;
use std::num::NonZeroUsize;

/// Single assembly instruction.
#[derive(Clone)]
pub struct Instruction {
    /// Address in file.
    pub address: GlobalAddress,
    /// Instruction mnemonic.
    pub mnemonic: Option<String>,
    /// Operands string representation.
    pub operands: Option<String>,
}

/// Generate disassembled code of a .text section.
pub struct Disassembler {
    cs: Capstone,
    cache: RefCell<LruCache<(RelocatedAddress, RelocatedAddress), Vec<Instruction>>>,
}

impl Disassembler {
    /// Create a new [`Disassembler`].
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            cs: Capstone::new()
                .x86()
                .mode(arch::x86::ArchMode::Mode64)
                .syntax(arch::x86::ArchSyntax::Att)
                .build()
                .map_err(Error::DisAsmInit)?,
            cache: RefCell::new(LruCache::new(NonZeroUsize::new(1000).expect("infallible"))),
        })
    }

    /// Return disassembled function representation.
    ///
    /// # Arguments
    ///
    /// * `debugee`: debugee instance
    /// * `debug_info`: parsed dwarf information
    /// * `function`: function for disassemble
    /// * `breakpoints`: list of active breakpoints
    pub fn disasm_function(
        &self,
        debugee: &Debugee,
        debug_info: &DebugInformation,
        function: ContextualDieRef<FunctionDie>,
        breakpoints: &[&Breakpoint],
    ) -> Result<Vec<Instruction>, Error> {
        let fn_glob_pc_start = function.start_pc()?;
        let fn_reloc_pc_start = fn_glob_pc_start.relocate_to_segment(debugee, debug_info)?;
        let fn_reloc_pc_end = function
            .end_pc()?
            .relocate_to_segment(debugee, debug_info)?;

        let cache_key = (fn_reloc_pc_start, fn_reloc_pc_end);
        let mut cache = self.cache.borrow_mut();
        let instructions = cache.try_get_or_insert(cache_key, || -> Result<_, Error> {
            let text_len = usize::from(fn_reloc_pc_end) - usize::from(fn_reloc_pc_start);
            let mut text = debugger::read_memory_by_pid(
                debugee.tracee_ctl().proc_pid(),
                fn_reloc_pc_start.into(),
                text_len,
            )
            .map_err(Error::Ptrace)?;

            breakpoints
                .iter()
                .filter(|brkpt| brkpt.addr >= fn_reloc_pc_start && brkpt.addr <= fn_reloc_pc_end)
                .for_each(|brkpt| {
                    let byte_idx = usize::from(brkpt.addr) - usize::from(fn_reloc_pc_start);
                    text[byte_idx] = brkpt.saved_data.get();
                });

            let instructions = self
                .cs
                .disasm_all(&text, fn_glob_pc_start.into())
                .map_err(Error::DisAsm)?
                .iter()
                .map(|i| Instruction {
                    address: i.address().into(),
                    mnemonic: i.mnemonic().map(ToString::to_string),
                    operands: i.op_str().map(ToString::to_string),
                })
                .collect();
            Ok(instructions)
        })?;

        Ok(instructions.clone())
    }
}
