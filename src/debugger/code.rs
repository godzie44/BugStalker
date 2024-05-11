#![allow(unused)]

/// Sent by the kernel from somewhere
pub const SI_KERNEL: i32 = 0x80;

// ---------------- SIGTRAP si_codes ---------------------------------------------------------------

/// Process breakpoint
pub const TRAP_BRKPT: i32 = 0x1;
/// Process trace trap
pub const TRAP_TRACE: i32 = 0x2;
/// Process taken branch trap
pub const TRAP_BRANCH: i32 = 0x3;
/// Hardware breakpoint/watchpoint
pub const TRAP_HWBKPT: i32 = 0x4;
/// Undiagnosed trap
pub const TRAP_UNK: i32 = 0x5;
/// Perf event with sigtrap=1
pub const TRAP_PERF: i32 = 0x6;
