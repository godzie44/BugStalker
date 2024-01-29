use crate::debugger::address::GlobalAddress;
use crate::debugger::debugee::dwarf::unit::DieRef;
use crate::debugger::debugee::RendezvousError;
use crate::debugger::variable::ParsingError;
use gimli::UnitOffset;
use nix::unistd::Pid;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // --------------------------------- generic errors --------------------------------------------
    #[error("debugee already run")]
    AlreadyRun,
    #[error(transparent)]
    IO(#[from] std::io::Error),
    #[error(transparent)]
    Utf8(#[from] Utf8Error),
    #[error(transparent)]
    FromUtf8(#[from] FromUtf8Error),
    #[error(transparent)]
    RegEx(#[from] regex::Error),

    // --------------------------------- debugger entity not found----------------------------------
    #[error("no debug information for {0}")]
    NoDebugInformation(&'static str),
    #[error("unknown register {0:?}")]
    RegisterNotFound(gimli::Register),
    #[error("unknown register {0:?}")]
    RegisterNameNotFound(String),
    #[error("source place not found at address {0}")]
    PlaceNotFound(GlobalAddress),
    #[error("there are no suitable places for this request")]
    NoSuitablePlace,
    #[error("unit not found at address {0}")]
    UnitNotFound(GlobalAddress),
    #[error("function not found at address {0}")]
    FunctionNotFound(GlobalAddress),
    #[error("type not found")]
    TypeNotFound,
    #[error("frame number {0} not found")]
    FrameNotFound(u32),
    #[error("tracee number {0} not found")]
    TraceeNotFound(u32),
    #[error("debug information entry (die) not found, reference: {0:?}")]
    DieNotFound(DieRef),
    #[error("section \"{0}\" not found")]
    SectionNotFound(&'static str),

    // --------------------------------- remote memory errors --------------------------------------
    #[error("invalid binary representation of type `{0}`: {1:?}")]
    TypeBinaryRepr(&'static str, Box<[u8]>),
    #[error("unknown address")]
    UnknownAddress,
    #[error("memory region offset not found ({0})")]
    MappingOffsetNotFound(&'static str),
    #[error("memory region not found for a file: {0}")]
    MappingNotFound(String),

    // --------------------------------- syscall errors --------------------------------------------
    #[error("waitpid syscall error: {0}")]
    Waitpid(nix::Error),
    #[error("ptrace syscall error: {0}")]
    Ptrace(nix::Error),
    #[error("{0} syscall error: {1}")]
    Syscall(&'static str, nix::Error),
    #[error("multiple syscall errors {0:?}")]
    MultipleErrors(Vec<Self>),

    // --------------------------------- parsing errors --------------------------------------------
    #[error("dwarf file parsing error: {0}")]
    DwarfParsing(#[from] gimli::Error),
    #[error("invalid debug-id note format")]
    DebugIDFormat,
    #[error("object file parsing error: {0}")]
    ObjParsing(#[from] object::Error),
    #[error(transparent)]
    VariableParsing(#[from] ParsingError),
    #[error("function specification ({0:?}) reference to unseen declaration")]
    InvalidSpecification(UnitOffset),

    // --------------------------------- unwind errors ---------------------------------------------
    #[error("unwind: no unwind context")]
    UnwindNoContext,
    #[error("unwind: too deep frame number")]
    UnwindTooDeepFrame,

    #[cfg(feature = "libunwind")]
    #[error("libunwind error: {0}")]
    LibUnwind(#[from] unwind::Error),

    // --------------------------------- dwarf errors ----------------------------------------------
    #[error("dwarf expression evaluation: eval option `{0}` required")]
    EvalOptionRequired(&'static str),
    #[error("dwarf expression evaluation: unsupported evaluation require ({0})")]
    EvalUnsupportedRequire(&'static str),
    #[error("no frame base address")]
    NoFBA,
    #[error("frame base address attribute not an expression")]
    FBANotAnExpression,
    #[error("range information for function `{0:?}` not exists")]
    NoFunctionRanges(Option<String>),
    #[error("die type not exists")]
    NoDieType,
    #[error("fail to read/evaluate implicit pointer address")]
    ImplicitPointer,

    // --------------------------------- libthread_db errors ---------------------------------------
    #[error("libthread_db not enabled")]
    NoThreadDB,
    #[error("libthread_db: {0}")]
    ThreadDB(#[from] thread_db::ThreadDbError),

    // --------------------------------- linker errors ---------------------------------------------
    #[error(transparent)]
    Rendezvous(#[from] RendezvousError),

    // --------------------------------- debugee process errors ------------------------------------
    #[error("debugee process exit with code {0}")]
    ProcessExit(i32),
    #[error("program is not being started")]
    ProcessNotStarted,

    // --------------------------------- rust toolchain errors -------------------------------------
    #[error("default toolchain not found")]
    DefaultToolchainNotFound,
    #[error("unrecognized rustup output")]
    UnrecognizedRustupOut,

    // --------------------------------- disasm ----------------------------------------------------
    #[error("install disassembler: {0}")]
    DisAsmInit(capstone::Error),
    #[error("instructions disassembly error: {0}")]
    DisAsm(capstone::Error),

    // --------------------------------- third party errors ----------------------------------------
    #[error("hook: {0}")]
    Hook(anyhow::Error),

    // --------------------------------- attach debugee errors -------------------------------------
    #[error("process pid {0} not found")]
    AttachedProcessNotFound(Pid),
    #[error("attach a running process: {0}")]
    Attach(nix::Error),
}

impl Error {
    /// Return a hint to an interface - continue debugging after error or stop whole process.
    pub fn is_fatal(&self) -> bool {
        match self {
            Error::AlreadyRun => false,
            Error::IO(_) => false,
            Error::Utf8(_) => false,
            Error::FromUtf8(_) => false,
            Error::RegEx(_) => false,
            Error::NoDebugInformation(_) => false,
            Error::RegisterNotFound(_) => false,
            Error::RegisterNameNotFound(_) => false,
            Error::PlaceNotFound(_) => false,
            Error::NoSuitablePlace => false,
            Error::UnitNotFound(_) => false,
            Error::FunctionNotFound(_) => false,
            Error::TypeNotFound => false,
            Error::FrameNotFound(_) => false,
            Error::TraceeNotFound(_) => false,
            Error::DieNotFound(_) => false,
            Error::TypeBinaryRepr(_, _) => false,
            Error::UnknownAddress => false,
            Error::MappingOffsetNotFound(_) => false,
            Error::MappingNotFound(_) => false,
            Error::Waitpid(_) => false,
            Error::Ptrace(_) => false,
            Error::MultipleErrors(_) => false,
            Error::DebugIDFormat => false,
            Error::VariableParsing(_) => false,
            Error::UnwindNoContext => false,
            Error::UnwindTooDeepFrame => false,
            #[cfg(feature = "libunwind")]
            Error::LibUnwind(_) => false,
            Error::EvalOptionRequired(_) => false,
            Error::EvalUnsupportedRequire(_) => false,
            Error::NoFBA => false,
            Error::FBANotAnExpression => false,
            Error::NoFunctionRanges(_) => false,
            Error::NoDieType => false,
            Error::ImplicitPointer => false,
            Error::ThreadDB(_) => false,
            Error::Rendezvous(_) => false,
            Error::ProcessExit(_) => false,
            Error::ProcessNotStarted => false,
            Error::DefaultToolchainNotFound => false,
            Error::UnrecognizedRustupOut => false,
            Error::Hook(_) => false,
            Error::SectionNotFound(_) => false,
            Error::DisAsm(_) => false,
            Error::InvalidSpecification(_) => false,

            // currently fatal errors
            Error::DwarfParsing(_) => true,
            Error::ObjParsing(_) => true,
            Error::Syscall(_, _) => true,
            Error::NoThreadDB => true,
            Error::DisAsmInit(_) => true,
            Error::AttachedProcessNotFound(_) => true,
            Error::Attach(_) => true,
        }
    }
}

#[macro_export]
macro_rules! _error {
    ($log_fn: path, $res: expr) => {
        match $res {
            Ok(value) => Some(value),
            Err(e) => {
                $log_fn!(target: "debugger", "{:#}", e);
                None
            }
        }
    };
    ($log_fn: path, $res: expr, $msg: tt) => {
        match $res {
            Ok(value) => Some(value),
            Err(e) => {
                $log_fn!(target: "debugger", concat!($msg, " {:#}"), e);
                None
            }
        }
    };
}

/// Transforms `Result` into `Option` and logs an error if it occurs.
#[macro_export]
macro_rules! weak_error {
    ($res: expr) => {
        $crate::_error!(log::warn, $res)
    };
    ($res: expr, $msg: tt) => {
        $crate::_error!(log::warn, $res, $msg)
    };
}

/// Transforms `Result` into `Option` and put error into debug logs if it occurs.
#[macro_export]
macro_rules! muted_error {
    ($res: expr) => {
        $crate::_error!(log::debug, $res)
    };
    ($res: expr, $msg: tt) => {
        $crate::_error!(log::debug, $res, $msg)
    };
}

/// Macro for handle an error lists as warnings.
#[macro_export]
macro_rules! print_warns {
    ($errors:expr) => {
        $errors.iter().for_each(|e| {
            log::warn!(target: "debugger", "{:#}", e);
        })
    };
}
