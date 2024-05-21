use crate::debugger::error::Error;
use crate::debugger::error::Error::{Ptrace, RegisterNotFound};
use nix::libc::user_regs_struct;
use nix::sys;
use nix::unistd::Pid;
use smallvec::{smallvec, SmallVec};
use strum_macros::Display;
use strum_macros::EnumString;

/// x86_64 registers.
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

/// x86_64 register values.
#[derive(Debug)]
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
    /// Return current register values for selected thread.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread id.
    pub fn current(pid: Pid) -> Result<Self, Error> {
        let regs = sys::ptrace::getregs(pid).map_err(Ptrace)?;
        Ok(regs.into())
    }

    /// Return register value.
    ///
    /// # Arguments
    ///
    /// * `register`: target register.
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

    /// Set new register value.
    ///
    /// # Arguments
    ///
    /// * `register`: target register.
    /// * `value`: new value.
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

    /// Replace tread registers with values taken from this map.
    ///
    /// # Arguments
    ///
    /// * `pid`: target thread.
    pub fn persist(self, pid: Pid) -> Result<(), Error> {
        sys::ptrace::setregs(pid, self.into()).map_err(Ptrace)
    }
}

/// x86_64 register values, using DWARF register number as index.
#[derive(Debug, Clone)]
pub struct DwarfRegisterMap(SmallVec<[Option<u64>; 0x80]>);

impl DwarfRegisterMap {
    /// Return register value.
    ///
    /// # Arguments
    ///
    /// * `register`: target register.
    pub fn value(&self, register: gimli::Register) -> Result<u64, Error> {
        self.0
            .get(register.0 as usize)
            .copied()
            .and_then(|v| v)
            .ok_or(RegisterNotFound(register))
    }

    /// Set new register value.
    ///
    /// # Arguments
    ///
    /// * `register`: target register.
    /// * `value`: new value.
    pub fn update(&mut self, register: gimli::Register, value: u64) {
        self.0[register.0 as usize] = Some(value);
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
        dwarf_map.insert(16, Some(map.rip));
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

pub mod debug {
    use crate::debugger::Error;
    use crate::debugger::Error::Ptrace;
    use bit_field::BitField;
    use nix::sys;
    use nix::sys::ptrace::AddressType;
    use nix::unistd::Pid;
    use std::ffi::c_void;
    use std::fmt::{Display, Formatter};
    use std::mem::offset_of;
    use strum_macros::FromRepr;

    /// Debug register representation.
    #[repr(usize)]
    #[derive(Clone, Copy, Debug, PartialEq, FromRepr)]
    pub enum DebugRegisterNumber {
        DR0,
        DR1,
        DR2,
        DR3,
    }

    pub type DebugAddressRegister = usize;

    #[derive(Clone, Copy, PartialEq, Debug)]
    pub struct DebugStatusRegister(usize);

    macro_rules! impl_trap {
        ($fn_name: ident, $trap: path) => {
            #[doc = "Return true if breakpoint X condition was detected."]
            #[doc = "Reset the corresponding flag."]
            pub fn $fn_name(&mut self) -> bool {
                let is_set = (self.0 & $trap) == $trap;
                self.0 &= !$trap;
                is_set
            }
        };
    }

    impl DebugStatusRegister {
        /// Breakpoint condition 0 was detected.
        const TRAP0: usize = 1;
        /// Breakpoint condition 1 was detected.
        const TRAP1: usize = 1 << 1;
        /// Breakpoint condition 2 was detected.
        const TRAP2: usize = 1 << 2;
        /// Breakpoint condition 3 was detected.
        const TRAP3: usize = 1 << 3;

        impl_trap!(trap0, Self::TRAP0);
        impl_trap!(trap1, Self::TRAP1);
        impl_trap!(trap2, Self::TRAP2);
        impl_trap!(trap3, Self::TRAP3);

        /// Return debug register number and flush it if breakpoint was hit.
        pub fn detect_and_flush(&mut self) -> Option<DebugRegisterNumber> {
            let dr = if self.trap0() {
                DebugRegisterNumber::DR0
            } else if self.trap1() {
                DebugRegisterNumber::DR1
            } else if self.trap2() {
                DebugRegisterNumber::DR2
            } else if self.trap3() {
                DebugRegisterNumber::DR3
            } else {
                return None;
            };
            Some(dr)
        }
    }

    #[derive(Clone, Copy, PartialEq, Debug)]
    pub struct DebugControlRegister(usize);

    impl DebugControlRegister {
        /// Enable detection of exact instruction causing a data breakpoint condition for the current task.
        /// This is not supported by `x86_64` processors,
        /// but is recommended to be enabled for backward and forward compatibility.
        const LOCAL_EXACT_BREAKPOINT_ENABLE_BIT: usize = 8;
        /// Enable detection of exact instruction causing a data breakpoint condition for all tasks.
        /// This is not supported by `x86_64` processors,
        /// but is recommended to be enabled for backward and forward compatibility.
        const GLOBAL_EXACT_BREAKPOINT_ENABLE_BIT: usize = 9;

        /// Return true if breakpoint enabled.
        ///
        /// # Arguments
        ///
        /// * `dr_num`: address debug register number
        /// * `global`: whether the breakpoint is global or local
        #[inline(always)]
        pub fn dr_enabled(&self, dr: DebugRegisterNumber, global: bool) -> bool {
            let dr = dr as usize;
            let idx = if global { dr * 2 + 1 } else { dr * 2 };
            debug_assert!(idx <= 7);
            self.0.get_bit(idx)
        }

        /// Configures a breakpoint condition and size for the associated breakpoint.
        ///
        /// # Arguments
        ///
        /// * `dr`: address debug register number
        /// * `cond`: breakpoint condition
        /// * `size`: breakpoint size
        #[inline(always)]
        pub fn configure_bp(
            &mut self,
            dr: DebugRegisterNumber,
            cond: BreakCondition,
            size: BreakSize,
        ) {
            let dr = dr as usize;
            // set condition
            let idx = 16 + (dr * 4);
            self.0.set_bits(idx..=idx + 1, cond as usize);
            // set size
            let idx = 18 + (dr * 4);
            self.0.set_bits(idx..=idx + 1, size as usize);
        }

        /// Enable/disable a breakpoint either as global or local.
        ///
        /// # Arguments
        /// * `dr_num` - address debug register to enable/disable
        /// * `global` - whether the breakpoint is global or local
        /// * `enable` - whether to enable or disable the breakpoint
        #[inline(always)]
        pub fn set_dr(&mut self, dr: DebugRegisterNumber, global: bool, enable: bool) {
            let dr = dr as usize;
            let idx = if global { dr * 2 + 1 } else { dr * 2 };
            self.0.set_bit(idx, enable);

            let detection_bit = if global {
                Self::GLOBAL_EXACT_BREAKPOINT_ENABLE_BIT
            } else {
                Self::LOCAL_EXACT_BREAKPOINT_ENABLE_BIT
            };

            if enable {
                self.0.set_bit(detection_bit, true);
            } else {
                let all_disabled = [0, 1, 2, 3].iter().all(|&n| {
                    !self.dr_enabled(
                        DebugRegisterNumber::from_repr(n).expect("infallible"),
                        global,
                    )
                });
                if all_disabled {
                    self.0.set_bit(detection_bit, false);
                }
            }
        }
    }

    #[derive(PartialEq, Debug)]
    pub struct HardwareDebugState {
        /// Four (dr0, dr1, dr2, dr3 for x86_64) address debug registers.
        pub address_regs: [DebugAddressRegister; 4],
        /// Debug status register.
        pub dr6: DebugStatusRegister,
        /// Debug control register.
        pub dr7: DebugControlRegister,
    }

    impl HardwareDebugState {
        /// Return the current state of hardware debug registers.
        ///
        /// # Arguments
        ///
        /// * `pid`: thread id for which state is loaded
        pub fn current(pid: Pid) -> Result<Self, Error> {
            use nix::libc::user;

            fn get_dr(pid: Pid, num: usize) -> Result<usize, Error> {
                let offset = offset_of!(user, u_debugreg) + num * 8;
                Ok(sys::ptrace::read_user(pid, offset as AddressType).map_err(Ptrace)? as usize)
            }

            Ok(Self {
                address_regs: [
                    get_dr(pid, 0)?,
                    get_dr(pid, 1)?,
                    get_dr(pid, 2)?,
                    get_dr(pid, 3)?,
                ],
                dr6: DebugStatusRegister(get_dr(pid, 6)?),
                dr7: DebugControlRegister(get_dr(pid, 7)?),
            })
        }

        /// Synchronize state and debug registers.
        ///
        /// # Arguments
        ///
        /// * `pid`: thread id into which registers data is saved
        pub fn sync(&self, pid: Pid) -> Result<(), Error> {
            fn set_dr(pid: Pid, num: usize, data: usize) -> Result<(), Error> {
                let offset = offset_of!(nix::libc::user, u_debugreg);
                let offset = offset + num * 8;
                unsafe {
                    sys::ptrace::write_user(pid, offset as AddressType, data as *mut c_void)
                        .map_err(Ptrace)
                }
            }

            for (reg_num, val) in self.address_regs.iter().enumerate() {
                set_dr(pid, reg_num, *val)?;
            }
            set_dr(pid, 6, self.dr6.0)?;
            set_dr(pid, 7, self.dr7.0)?;
            Ok(())
        }
    }

    /// Specifies the breakpoint condition for a corresponding breakpoint.
    ///
    /// Instruction and i/o read-write conditions are unused and aren't presented here.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd)]
    pub enum BreakCondition {
        /// 01 — Break on data writes only.
        DataWrites = 0b01,
        /// 11 — Break on data reads or writes but not instruction fetches.
        DataReadsWrites = 0b11,
    }

    impl Display for BreakCondition {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                BreakCondition::DataWrites => f.write_str("w"),
                BreakCondition::DataReadsWrites => f.write_str("rw"),
            }
        }
    }

    /// Specify the size of the memory location at the address specified in the
    /// corresponding breakpoint address register (DR0 through DR3).
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum BreakSize {
        /// 1-byte length.
        Bytes1 = 0b00,
        /// 2-byte length.
        Bytes2 = 0b01,
        /// 8 byte length (or undefined, on older processors).
        Bytes8 = 0b10,
        /// 4-byte length.
        Bytes4 = 0b11,
    }

    impl TryFrom<u8> for BreakSize {
        type Error = Error;

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            let size = match value {
                1 => BreakSize::Bytes1,
                2 => BreakSize::Bytes2,
                4 => BreakSize::Bytes4,
                8 => BreakSize::Bytes8,
                _ => return Err(Error::WatchpointWrongSize),
            };
            Ok(size)
        }
    }

    impl Display for BreakSize {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                BreakSize::Bytes1 => f.write_str("1b"),
                BreakSize::Bytes2 => f.write_str("2b"),
                BreakSize::Bytes8 => f.write_str("8b"),
                BreakSize::Bytes4 => f.write_str("4b"),
            }
        }
    }
}
