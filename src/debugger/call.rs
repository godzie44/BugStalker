use super::{
    Debugger, Error, FunctionDie, TypeDeclaration,
    address::RelocatedAddress,
    debugee::dwarf::{AsAllocatedData, ContextualDieRef, DebugInformation, r#type::ComplexType},
    register::{Register, RegisterMap},
    utils::PopIf,
    variable::dqe::Literal,
};
use crate::{debugger::read_memory_by_pid, disable_when_not_stared, type_from_cache, weak_error};
use log::debug;
use nix::sys::{self, wait::WaitStatus};
use proc_maps::MapRange;
use std::{collections::hash_map::Entry, rc::Rc};

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("Invalid argument count, expect {0}, got {1}")]
    InvalidArgumentCount(usize, usize),
    #[error("At most 6 8-byte arguments allowed at this moment")]
    TooManyArguments,
    #[error("`{0}` literal type are not supported")]
    UnsupportedLiteral(&'static str),
    #[error("Type of argument {0} is unknown")]
    UnknownArgumentType(usize),
    #[error("The conversion of literal {0} to argument of type {1} is not allowed")]
    LiteralCast(usize, String),
    #[error("Argument {0} of type {0} is unsupported")]
    UnsupportedArgumentType(usize, String),
    #[error("Function not found or too many candidates")]
    FunctionNotFoundOrTooMany,
    #[error("mmap call failed")]
    Mmap,
    #[error("munmap call failed")]
    Munmap,
}

/// Use general registers or floating point registers.
#[derive(Clone, Copy)]
enum RegType {
    General,
    #[allow(unused)]
    Floating,
}

/// Function call arguments.
#[derive(Default)]
struct CallArgs(Box<[(u64, RegType)]>);

fn liter_to_arg_bin_repr(
    no: usize,
    lit: &Literal,
    to_type: &ComplexType,
) -> Result<(u64, RegType), CallError> {
    let root_type_id = to_type.root();
    let r#type = &to_type.types[&root_type_id];

    macro_rules! unsup_arg_bail {
        ($no: expr, $t: expr, $root: expr) => {
            return Err(CallError::UnsupportedArgumentType(
                $no,
                $t.identity($root).name_fmt().to_string(),
            ))
        };
    }

    macro_rules! lit_cast_bail {
        ($no: expr, $t: expr, $root: expr) => {
            return Err(CallError::LiteralCast(
                $no,
                $t.identity($root).name_fmt().to_string(),
            ))
        };
    }

    Ok(match lit {
        Literal::String(_) => return Err(CallError::UnsupportedLiteral("string")),
        Literal::Int(val) => {
            let TypeDeclaration::Scalar(scalar_type) = r#type else {
                lit_cast_bail!(no, to_type, root_type_id)
            };

            let Some(encoding) = scalar_type.encoding else {
                return Err(CallError::UnknownArgumentType(no));
            };

            let mut bytes = [0u8; 8];
            match encoding {
                gimli::DW_ATE_signed_char => {
                    let int8: i8 = *val as i8;
                    bytes[0] = int8 as u8;
                }
                gimli::DW_ATE_unsigned_char => {
                    bytes[0] = *val as u8;
                }
                gimli::DW_ATE_signed => {
                    match scalar_type.byte_size.unwrap_or(0) {
                        1 => {
                            let int8: i8 = *val as i8;
                            bytes[0] = int8 as u8;
                        }
                        2 => {
                            let int16: i16 = *val as i16;
                            let b = int16.to_le_bytes();
                            bytes[..2].copy_from_slice(&b);
                        }
                        4 => {
                            let int32: i32 = *val as i32;
                            let b = int32.to_le_bytes();
                            bytes[..4].copy_from_slice(&b);
                        }
                        8 => {
                            bytes.copy_from_slice(&((*val).to_ne_bytes()));
                        }
                        _ => unsup_arg_bail!(no, to_type, root_type_id),
                    };
                }
                gimli::DW_ATE_unsigned => match scalar_type.byte_size.unwrap_or(0) {
                    1 => {
                        bytes[0] = *val as u8;
                    }
                    2 => {
                        let b = (*val as u16).to_le_bytes();
                        bytes[..2].copy_from_slice(&b);
                    }
                    4 => {
                        let b = (*val as u32).to_le_bytes();
                        bytes[..4].copy_from_slice(&b);
                    }
                    8 => {
                        bytes.copy_from_slice(&((*val as u64).to_le_bytes()));
                    }
                    _ => unsup_arg_bail!(no, to_type, root_type_id),
                },
                _ => {
                    lit_cast_bail!(no, to_type, root_type_id)
                }
            };

            (u64::from_le_bytes(bytes), RegType::General)
        }
        Literal::Float(_) => return Err(CallError::UnsupportedLiteral("float")),
        Literal::Address(_) => return Err(CallError::UnsupportedLiteral("pointer")),
        Literal::Bool(val) => {
            let TypeDeclaration::Scalar(scalar_type) = r#type else {
                lit_cast_bail!(no, to_type, root_type_id)
            };

            if scalar_type.encoding != Some(gimli::DW_ATE_boolean) {
                lit_cast_bail!(no, to_type, root_type_id)
            }

            (*val as u64, RegType::General)
        }
        Literal::EnumVariant(_, _) => return Err(CallError::UnsupportedLiteral("enum")),
        Literal::Array(_) => return Err(CallError::UnsupportedLiteral("array")),
        Literal::AssocArray(_) => return Err(CallError::UnsupportedLiteral("assoc array")),
    })
}

/// Map argument to the register according to System V AMD64 ABI.
fn get_reg_for_no(no: usize, reg_type: RegType) -> Register {
    match (no, reg_type) {
        (0, RegType::General) => Register::Rdi,
        (1, RegType::General) => Register::Rsi,
        (2, RegType::General) => Register::Rdx,
        (3, RegType::General) => Register::Rcx,
        (4, RegType::General) => Register::R8,
        (5, RegType::General) => Register::R9,
        _ => unreachable!("unsupported arg no or unknown register"),
    }
}

#[allow(unused)]
impl CallArgs {
    fn new(literals: &[Literal], fn_params: &[Rc<ComplexType>]) -> Result<Self, CallError> {
        if literals.len() != fn_params.len() {
            return Err(CallError::InvalidArgumentCount(
                fn_params.len(),
                literals.len(),
            ));
        }

        if literals.len() > 6 {
            return Err(CallError::TooManyArguments);
        }

        let args = literals
            .iter()
            .enumerate()
            .map(|(idx, lit)| liter_to_arg_bin_repr(idx, lit, &fn_params[idx]))
            .collect::<Result<Box<[(u64, RegType)]>, CallError>>()?;

        Ok(CallArgs(args))
    }

    /// Fill registers with arguments.
    fn prepare_registers(self, reg_map: &mut RegisterMap) {
        debug_assert!(
            self.0.len() < 7,
            "only 6 6-byte arguments allowed at this moment"
        );
        for (idx, (val, reg_type)) in self.0.iter().enumerate() {
            reg_map.update(get_reg_for_no(idx, *reg_type), *val);
        }
    }
}

// Update registers state for calling `mmap` syscall.
fn prepare_mmap(reg_map: &mut RegisterMap) {
    const MMAP: u64 = 9;
    reg_map.update(Register::Rax, MMAP);
    reg_map.update(Register::Rdi, 0);
    let page_size = unsafe { nix::libc::sysconf(nix::libc::_SC_PAGESIZE) as u64 };
    reg_map.update(Register::Rsi, page_size);
    reg_map.update(
        Register::Rdx,
        (nix::libc::PROT_READ | nix::libc::PROT_EXEC) as u64,
    );
    reg_map.update(
        Register::R10,
        (nix::libc::MAP_PRIVATE | nix::libc::MAP_ANONYMOUS) as u64,
    );
    let no_fd_bytes_tmp = (-1i32).to_ne_bytes();
    let no_fd_bytes = [
        no_fd_bytes_tmp[0],
        no_fd_bytes_tmp[1],
        no_fd_bytes_tmp[2],
        no_fd_bytes_tmp[3],
        0,
        0,
        0,
        0,
    ];
    reg_map.update(Register::R8, u64::from_ne_bytes(no_fd_bytes));
    reg_map.update(Register::R9, 0);
}

// Update registers state for calling `munmap` syscall.
fn prepare_munmap(reg_map: &mut RegisterMap, addr: u64) {
    const MUNMAP: u64 = 11;
    reg_map.update(Register::Rax, MUNMAP);
    reg_map.update(Register::Rdi, addr);
    let page_size = unsafe { nix::libc::sysconf(nix::libc::_SC_PAGESIZE) as u64 };
    reg_map.update(Register::Rsi, page_size);
}

impl Debugger {
    fn search_fn_to_call(
        &self,
        tpl: &str,
    ) -> Result<(&DebugInformation, ContextualDieRef<'_, '_, FunctionDie>), CallError> {
        let dwarfs = self.debugee.debug_info_all();

        let mut candidates = dwarfs
            .iter()
            .filter(|dwarf| dwarf.has_debug_info() && dwarf.tpl_in_pub_names(tpl) != Some(false))
            .filter_map(|&dwarf| {
                let funcs = weak_error!(dwarf.search_functions(tpl))?;
                if funcs.is_empty() {
                    None
                } else {
                    Some((dwarf, funcs))
                }
            })
            .collect::<Vec<_>>();

        candidates
            .pop_if_cond(|c| c.len() == 1)
            .and_then(|(dwarf, mut funcs)| Some((dwarf, funcs.pop_if_cond(|f| f.len() == 1)?)))
            .ok_or(CallError::FunctionNotFoundOrTooMany)
    }

    /// Do a function call.
    ///
    /// # Arguments
    ///
    /// * `fn_name`: function to call.
    /// * `arguments`: list of literals.
    pub fn call(&mut self, fn_name: &str, arguments: &[Literal]) -> Result<(), Error> {
        disable_when_not_stared!(self);

        debug!(target: "debugger", "find function address and prepare arguments");

        let (dwarf, func) = self.search_fn_to_call(fn_name)?;

        let fn_addr = func
            .prolog_start_place()?
            .address
            .relocate_to_segment(&self.debugee, dwarf)?;

        let params = {
            let mut type_cache = self.type_cache.borrow_mut();
            func.parameters()
                .into_iter()
                .map(|die| type_from_cache!(die, type_cache))
                .collect::<Result<Vec<_>, _>>()?
        };

        let args = CallArgs::new(arguments, params.as_slice())?;

        debug!(target: "debugger", "disable all active breakpoints");

        for brkpt in self.breakpoints.active_breakpoints() {
            brkpt.disable()?;
        }

        let call_result = call_user_fn(self, fn_addr, args);

        debug!(target: "debugger", "enable all active breakpoints");

        for brkpt in self.breakpoints.active_breakpoints() {
            brkpt
                .enable()
                .expect("enable breakpoint after disable should not leads to error");
        }

        call_result
    }
}

// Return true if address exist in VAS.
fn region_exist(pid: nix::unistd::Pid, addr: u64) -> std::io::Result<bool> {
    let proc_maps: Vec<MapRange> = proc_maps::get_process_maps(pid.as_raw())?;
    Ok(proc_maps.iter().any(|range| range.start() == addr as usize))
}

// Return true if address not exist in VAS.
fn region_non_exist(pid: nix::unistd::Pid, addr: u64) -> std::io::Result<bool> {
    region_exist(pid, addr).map(|exist| !exist)
}

fn call_user_fn(
    debugger: &mut Debugger,
    fn_addr: RelocatedAddress,
    args: CallArgs,
) -> Result<(), Error> {
    let pid_in_focus = debugger.exploration_ctx().pid_on_focus();
    let original_pc = debugger.exploration_ctx().location().pc;
    let original_regs = RegisterMap::current(pid_in_focus)?;
    let original_instructions =
        read_memory_by_pid(pid_in_focus, original_pc.as_usize(), size_of::<u64>())
            .map_err(Error::Ptrace)?;

    debug!(target: "debugger", "allocate space for instructions and data using mmap");

    let mut regs: RegisterMap = original_regs.clone();
    prepare_mmap(&mut regs);
    regs.persist(pid_in_focus)?;

    const SYSCALL: u16 = 0x0F05;
    const JMP_RAX: u16 = 0xFFE0;

    let mut new_instructions = original_instructions.clone();
    new_instructions[0] = (SYSCALL >> 8) as u8; // SYSCALL
    new_instructions[1] = SYSCALL as u8; // SYSCALL
    new_instructions[2] = (JMP_RAX >> 8) as u8; // JMP %rax
    new_instructions[3] = JMP_RAX as u8; // JMP %rax

    debugger.write_memory(
        original_pc.as_usize(),
        u64::from_ne_bytes(new_instructions.try_into().expect("unexpected size")) as usize,
    )?;

    sys::ptrace::step(pid_in_focus, None).map_err(Error::Ptrace)?;
    let res = nix::sys::wait::waitpid(pid_in_focus, None).map_err(Error::Waitpid)?;
    debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

    let regs = RegisterMap::current(pid_in_focus)?;
    let mmap_res: u64 = regs.value(Register::Rax);
    if mmap_res as i64 == -1 {
        return Err(CallError::Mmap.into());
    }

    debug_assert!(region_exist(pid_in_focus, mmap_res)?);

    debug!(target: "debugger", "jump into mmap'ed region");

    sys::ptrace::step(pid_in_focus, None).map_err(Error::Ptrace)?;
    let res = nix::sys::wait::waitpid(pid_in_focus, None).map_err(Error::Waitpid)?;
    debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

    debug!(target: "debugger", "add call instructions");

    const CALL_RAX: u16 = 0xFFD0;
    const BRKPT: u8 = 0xCC;

    let mut call_text: [u8; 8] = [0; 8];
    call_text[0] = (CALL_RAX >> 8) as u8;
    call_text[1] = CALL_RAX as u8;
    call_text[2] = BRKPT;

    let mut regs: RegisterMap = RegisterMap::current(pid_in_focus)?;
    debugger.write_memory(
        regs.value(Register::Rip) as usize,
        u64::from_ne_bytes(call_text) as usize,
    )?;

    debug!(target: "debugger", "prepare function arguments");

    args.prepare_registers(&mut regs);
    regs.update(Register::Rax, fn_addr.as_u64());
    regs.persist(pid_in_focus)?;

    debug!(target: "debugger", "call a function, wait until breakpoint are hit");

    sys::ptrace::cont(pid_in_focus, None).map_err(Error::Ptrace)?;
    let res = nix::sys::wait::waitpid(pid_in_focus, None).map_err(Error::Waitpid)?;
    debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

    debug!(target: "debugger", "going to original rip");

    original_regs.clone().persist(pid_in_focus)?;

    debug!(target: "debugger", "return allocated space using munmap");

    let mut regs: RegisterMap = original_regs.clone();
    prepare_munmap(&mut regs, mmap_res);
    regs.persist(pid_in_focus)?;
    sys::ptrace::step(pid_in_focus, None).map_err(Error::Ptrace)?;
    let res = nix::sys::wait::waitpid(pid_in_focus, None).map_err(Error::Waitpid)?;
    debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

    let regs: RegisterMap = RegisterMap::current(pid_in_focus)?;
    if regs.value(Register::Rax) != 0 {
        return Err(CallError::Munmap.into());
    }
    debug_assert!(region_non_exist(pid_in_focus, mmap_res)?);

    debug!(target: "debugger", "retrieve original registers and instructions");

    original_regs.persist(pid_in_focus)?;

    debugger.write_memory(
        original_pc.as_usize(),
        u64::from_ne_bytes(original_instructions.try_into().expect("unexpected size")) as usize,
    )?;

    Ok(())
}
