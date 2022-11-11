use anyhow::anyhow;
use nix::sys;
use nix::unistd::Pid;

#[derive(Copy, Clone, PartialEq)]
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

pub struct RegisterDescription {
    pub r: Register,
    pub dwarf_num: i32,
    pub name: &'static str,
}

impl RegisterDescription {
    const fn new(reg: Register, dwarf_num: i32, name: &'static str) -> Self {
        Self {
            r: reg,
            dwarf_num,
            name,
        }
    }
}

pub const LIST: [RegisterDescription; 27] = [
    RegisterDescription::new(Register::Rip, -1, "rip"),
    RegisterDescription::new(Register::OrigRax, -1, "orig_rax"),
    RegisterDescription::new(Register::Rax, 0, "rax"),
    RegisterDescription::new(Register::Rdx, 1, "rdx"),
    RegisterDescription::new(Register::Rcx, 2, "rcx"),
    RegisterDescription::new(Register::Rbx, 3, "rbx"),
    RegisterDescription::new(Register::Rsi, 4, "rsi"),
    RegisterDescription::new(Register::Rdi, 5, "rdi"),
    RegisterDescription::new(Register::Rbp, 6, "rbp"),
    RegisterDescription::new(Register::Rsp, 7, "rsp"),
    RegisterDescription::new(Register::R8, 8, "r8"),
    RegisterDescription::new(Register::R9, 9, "r9"),
    RegisterDescription::new(Register::R10, 10, "r10"),
    RegisterDescription::new(Register::R11, 11, "r11"),
    RegisterDescription::new(Register::R12, 12, "r12"),
    RegisterDescription::new(Register::R13, 13, "r13"),
    RegisterDescription::new(Register::R14, 14, "r14"),
    RegisterDescription::new(Register::R15, 15, "r15"),
    RegisterDescription::new(Register::Eflags, 49, "eflags"),
    RegisterDescription::new(Register::Es, 50, "es"),
    RegisterDescription::new(Register::Cs, 51, "cs"),
    RegisterDescription::new(Register::Ss, 52, "ss"),
    RegisterDescription::new(Register::Ds, 53, "ds"),
    RegisterDescription::new(Register::Fs, 54, "fs"),
    RegisterDescription::new(Register::Gs, 55, "gs"),
    RegisterDescription::new(Register::FsBase, 58, "fs_base"),
    RegisterDescription::new(Register::GsBase, 59, "gs_base"),
];

pub fn get_register_value(pid: Pid, reg: Register) -> nix::Result<u64> {
    let regs = sys::ptrace::getregs(pid)?;

    Ok(match reg {
        Register::Rax => regs.rax,
        Register::Rbx => regs.rbx,
        Register::Rcx => regs.rcx,
        Register::Rdx => regs.rdx,
        Register::Rdi => regs.rdi,
        Register::Rsi => regs.rsi,
        Register::Rbp => regs.rbp,
        Register::Rsp => regs.rsp,
        Register::R8 => regs.r8,
        Register::R9 => regs.r9,
        Register::R10 => regs.r10,
        Register::R11 => regs.r11,
        Register::R12 => regs.r12,
        Register::R13 => regs.r13,
        Register::R14 => regs.r14,
        Register::R15 => regs.r15,
        Register::Rip => regs.rip,
        Register::Eflags => regs.eflags,
        Register::Cs => regs.cs,
        Register::OrigRax => regs.orig_rax,
        Register::FsBase => regs.fs_base,
        Register::GsBase => regs.gs_base,
        Register::Fs => regs.fs,
        Register::Gs => regs.gs,
        Register::Ss => regs.ss,
        Register::Ds => regs.ds,
        Register::Es => regs.es,
    })
}

pub(super) fn set_register_value(pid: Pid, reg: Register, value: u64) -> nix::Result<()> {
    let mut regs = sys::ptrace::getregs(pid)?;

    match reg {
        Register::Rax => regs.rax = value,
        Register::Rbx => regs.rbx = value,
        Register::Rcx => regs.rcx = value,
        Register::Rdx => regs.rdx = value,
        Register::Rdi => regs.rdi = value,
        Register::Rsi => regs.rsi = value,
        Register::Rbp => regs.rbp = value,
        Register::Rsp => regs.rsp = value,
        Register::R8 => regs.r8 = value,
        Register::R9 => regs.r9 = value,
        Register::R10 => regs.r10 = value,
        Register::R11 => regs.r11 = value,
        Register::R12 => regs.r12 = value,
        Register::R13 => regs.r13 = value,
        Register::R14 => regs.r14 = value,
        Register::R15 => regs.r15 = value,
        Register::Rip => regs.rip = value,
        Register::Eflags => regs.eflags = value,
        Register::Cs => regs.cs = value,
        Register::OrigRax => regs.orig_rax = value,
        Register::FsBase => regs.fs_base = value,
        Register::GsBase => regs.gs_base = value,
        Register::Fs => regs.fs = value,
        Register::Gs => regs.gs = value,
        Register::Ss => regs.ss = value,
        Register::Ds => regs.ds = value,
        Register::Es => regs.es = value,
    };
    sys::ptrace::setregs(pid, regs)
}

pub(super) fn get_register_value_dwarf(pid: Pid, dwarf_num: i32) -> anyhow::Result<u64> {
    let descr = LIST
        .iter()
        .find(|r| r.dwarf_num == dwarf_num)
        .ok_or_else(|| anyhow!("invalid dwarf register number {}", dwarf_num))?;
    Ok(get_register_value(pid, descr.r)?)
}

#[allow(unused)]
pub fn get_register_name(reg: Register) -> &'static str {
    match LIST.iter().find(|r| r.r == reg) {
        None => unreachable!(),
        Some(descr) => descr.name,
    }
}

pub(super) fn get_register_from_name(name: &str) -> anyhow::Result<Register> {
    LIST.iter()
        .find_map(|r| if r.name == name { Some(r.r) } else { None })
        .ok_or_else(|| anyhow!("Register not found"))
}
