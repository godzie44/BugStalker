use anyhow::anyhow;
use nix::libc::user_regs_struct;
use nix::sys;
use nix::unistd::Pid;
use smallvec::{smallvec, SmallVec};
use strum_macros::Display;
use strum_macros::EnumString;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, EnumString, Display)]
#[strum(serialize_all = "snake_case")]
pub enum Register {
    Rax,
    Rbx,
    Rcx,
    Rdx,
    Rdi,
    Rsi,
    Rbp,
    Rsp,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
    Rip,
    Eflags,
    Cs,
    OrigRax,
    FsBase,
    GsBase,
    Fs,
    Gs,
    Ss,
    Ds,
    Es,
}

impl From<gimli::Register> for Register {
    fn from(value: gimli::Register) -> Self {
        match value.0 as i32 {
            -1 => Register::Rip,
            //-1 => Register::OrigRax,
            0 => Register::Rax,
            1 => Register::Rdx,
            2 => Register::Rcx,
            3 => Register::Rbx,
            4 => Register::Rsi,
            5 => Register::Rdi,
            6 => Register::Rbp,
            7 => Register::Rsp,
            8 => Register::R8,
            9 => Register::R9,
            10 => Register::R10,
            11 => Register::R11,
            12 => Register::R12,
            13 => Register::R13,
            14 => Register::R14,
            15 => Register::R15,
            49 => Register::Eflags,
            50 => Register::Es,
            51 => Register::Cs,
            52 => Register::Ss,
            53 => Register::Ds,
            54 => Register::Fs,
            55 => Register::Gs,
            58 => Register::FsBase,
            59 => Register::GsBase,
            _ => {
                panic!("unknown dwarf register number");
            }
        }
    }
}

pub struct RegisterMap {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rsp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    eflags: u64,
    cs: u64,
    orig_rax: u64,
    fs_base: u64,
    gs_base: u64,
    fs: u64,
    gs: u64,
    ss: u64,
    ds: u64,
    es: u64,
}

impl From<user_regs_struct> for RegisterMap {
    fn from(value: user_regs_struct) -> Self {
        Self {
            rax: value.rax,
            rbx: value.rbx,
            rcx: value.rcx,
            rdx: value.rdx,
            rdi: value.rdi,
            rsi: value.rsi,
            rbp: value.rbp,
            rsp: value.rsp,
            r8: value.r8,
            r9: value.r9,
            r10: value.r10,
            r11: value.r11,
            r12: value.r12,
            r13: value.r13,
            r14: value.r14,
            r15: value.r15,
            rip: value.rip,
            eflags: value.eflags,
            cs: value.cs,
            orig_rax: value.orig_rax,
            fs_base: value.fs_base,
            gs_base: value.gs_base,
            fs: value.fs,
            gs: value.gs,
            ss: value.ss,
            ds: value.ds,
            es: value.es,
        }
    }
}

impl From<RegisterMap> for user_regs_struct {
    fn from(reg_map: RegisterMap) -> user_regs_struct {
        user_regs_struct {
            rax: reg_map.rax,
            rbx: reg_map.rbx,
            rcx: reg_map.rcx,
            rdx: reg_map.rdx,
            rdi: reg_map.rdi,
            rsi: reg_map.rsi,
            rbp: reg_map.rbp,
            rsp: reg_map.rsp,
            r8: reg_map.r8,
            r9: reg_map.r9,
            r10: reg_map.r10,
            r11: reg_map.r11,
            r12: reg_map.r12,
            r13: reg_map.r13,
            r14: reg_map.r14,
            r15: reg_map.r15,
            rip: reg_map.rip,
            eflags: reg_map.eflags,
            cs: reg_map.cs,
            orig_rax: reg_map.orig_rax,
            fs_base: reg_map.fs_base,
            gs_base: reg_map.gs_base,
            fs: reg_map.fs,
            gs: reg_map.gs,
            ss: reg_map.ss,
            ds: reg_map.ds,
            es: reg_map.es,
        }
    }
}

impl RegisterMap {
    pub fn current(pid: Pid) -> nix::Result<Self> {
        let regs = sys::ptrace::getregs(pid)?;
        Ok(regs.into())
    }

    pub fn value(&self, register: impl Into<Register>) -> u64 {
        let register = register.into();
        match register {
            Register::Rax => self.rax,
            Register::Rbx => self.rbx,
            Register::Rcx => self.rcx,
            Register::Rdx => self.rdx,
            Register::Rdi => self.rdi,
            Register::Rsi => self.rsi,
            Register::Rbp => self.rbp,
            Register::Rsp => self.rsp,
            Register::R8 => self.r8,
            Register::R9 => self.r9,
            Register::R10 => self.r10,
            Register::R11 => self.r11,
            Register::R12 => self.r12,
            Register::R13 => self.r13,
            Register::R14 => self.r14,
            Register::R15 => self.r15,
            Register::Rip => self.rip,
            Register::Eflags => self.eflags,
            Register::Cs => self.cs,
            Register::OrigRax => self.orig_rax,
            Register::FsBase => self.fs_base,
            Register::GsBase => self.gs_base,
            Register::Fs => self.fs,
            Register::Gs => self.gs,
            Register::Ss => self.ss,
            Register::Ds => self.ds,
            Register::Es => self.es,
        }
    }

    pub fn update(&mut self, register: impl Into<Register>, value: u64) {
        match register.into() {
            Register::Rax => self.rax = value,
            Register::Rbx => self.rbx = value,
            Register::Rcx => self.rcx = value,
            Register::Rdx => self.rdx = value,
            Register::Rdi => self.rdi = value,
            Register::Rsi => self.rsi = value,
            Register::Rbp => self.rbp = value,
            Register::Rsp => self.rsp = value,
            Register::R8 => self.r8 = value,
            Register::R9 => self.r9 = value,
            Register::R10 => self.r10 = value,
            Register::R11 => self.r11 = value,
            Register::R12 => self.r12 = value,
            Register::R13 => self.r13 = value,
            Register::R14 => self.r14 = value,
            Register::R15 => self.r15 = value,
            Register::Rip => self.rip = value,
            Register::Eflags => self.eflags = value,
            Register::Cs => self.cs = value,
            Register::OrigRax => self.orig_rax = value,
            Register::FsBase => self.fs_base = value,
            Register::GsBase => self.gs_base = value,
            Register::Fs => self.fs = value,
            Register::Gs => self.gs = value,
            Register::Ss => self.ss = value,
            Register::Ds => self.ds = value,
            Register::Es => self.es = value,
        };
    }

    pub fn persist(self, pid: Pid) -> nix::Result<()> {
        sys::ptrace::setregs(pid, self.into())
    }
}

#[derive(Debug)]
pub struct DwarfRegisterMap(SmallVec<[Option<u64>; 0x80]>);

impl DwarfRegisterMap {
    pub fn value(&self, register: gimli::Register) -> anyhow::Result<u64> {
        self.0
            .get(register.0 as usize)
            .copied()
            .and_then(|v| v)
            .ok_or(anyhow!("register {} not found", register.0))
    }
}

/// Mapping dwarf registers to machine registers.
/// See https://docs.rs/gimli/0.13.0/gimli/struct.UnwindTableRow.html#method.register
impl From<RegisterMap> for DwarfRegisterMap {
    fn from(map: RegisterMap) -> Self {
        let mut dwarf_map = smallvec![None; 0x80];
        dwarf_map.insert(0, Some(map.rax));
        dwarf_map.insert(1, Some(map.rdx));
        dwarf_map.insert(2, Some(map.rcx));
        dwarf_map.insert(3, Some(map.rbx));
        dwarf_map.insert(4, Some(map.rsi));
        dwarf_map.insert(5, Some(map.rdi));
        dwarf_map.insert(6, Some(map.rbp));
        dwarf_map.insert(7, Some(map.rsp));
        dwarf_map.insert(8, Some(map.r8));
        dwarf_map.insert(9, Some(map.r9));
        dwarf_map.insert(10, Some(map.r10));
        dwarf_map.insert(11, Some(map.r11));
        dwarf_map.insert(12, Some(map.r12));
        dwarf_map.insert(13, Some(map.r13));
        dwarf_map.insert(14, Some(map.r14));
        dwarf_map.insert(15, Some(map.r15));
        dwarf_map.insert(49, Some(map.eflags));
        dwarf_map.insert(50, Some(map.es));
        dwarf_map.insert(51, Some(map.cs));
        dwarf_map.insert(52, Some(map.ss));
        dwarf_map.insert(53, Some(map.ds));
        dwarf_map.insert(54, Some(map.fs));
        dwarf_map.insert(55, Some(map.gs));
        dwarf_map.insert(58, Some(map.fs_base));
        dwarf_map.insert(59, Some(map.gs_base));
        DwarfRegisterMap(dwarf_map)
    }
}
