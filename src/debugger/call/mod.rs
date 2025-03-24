pub mod fmt;

use super::{
    Debugger, Error, FunctionDie, TypeDeclaration,
    address::RelocatedAddress,
    debugee::dwarf::{AsAllocatedData, ContextualDieRef, DebugInformation, r#type::ComplexType},
    register::{Register, RegisterMap},
    utils::PopIf,
    variable::dqe::Literal,
};
use crate::{
    debugger::{read_memory_by_pid, utils},
    disable_when_not_stared, type_from_cache, weak_error,
};
use log::debug;
use nix::sys::{self, wait::WaitStatus};
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
    #[error("JMP instruction failed")]
    Jmp,
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
        Literal::Address(addr) => {
            let TypeDeclaration::Pointer { .. } = r#type else {
                lit_cast_bail!(no, to_type, root_type_id)
            };

            (*addr as u64, RegType::General)
        }
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

/// Program state before a call.
struct CallContext<'a> {
    dbg: &'a Debugger,
    pid: nix::unistd::Pid,
    pc: RelocatedAddress,
    regs: RegisterMap,
    text: usize,
}

impl<'a> CallContext<'a> {
    fn new(dbg: &'a Debugger) -> Result<Self, Error> {
        let pid = dbg.exploration_ctx().pid_on_focus();
        let pc = dbg.exploration_ctx().location().pc;
        let text = read_memory_by_pid(pid, pc.into(), size_of::<u64>()).map_err(Error::Ptrace)?;
        let text = usize::from_ne_bytes(text.try_into().expect("unexpected size"));
        let regs = RegisterMap::current(pid)?;

        Ok(Self {
            dbg,
            pid,
            pc,
            regs,
            text,
        })
    }

    fn retrieve_original_state(self) -> Result<(), Error> {
        self.regs.clone().persist(self.pid)?; // TODO clone
        self.dbg.write_memory(self.pc.as_usize(), self.text)?;
        Ok(())
    }

    fn with_ctx<F, T>(mut self, f: F) -> Result<T, Error>
    where
        F: FnOnce(&mut Self) -> Result<T, Error>,
    {
        let result = f(&mut self);

        debug!(target: "debugger", "retrieve original registers and instructions");
        self.retrieve_original_state()
            .expect("failed to retrieve original program state after a call");

        result
    }
}

struct CallHelper;

impl CallHelper {
    fn call_fn(ctx: &CallContext, mem_ptr: u64, fn_addr: u64, args: CallArgs) -> Result<(), Error> {
        // new text:
        // FF D0 - CALL %rax
        // CC - break
        const CALL_FN: usize = 0xFFusize | (0xD0usize << 0x8) | (0xCCusize << 0x10);

        debug!(target: "debugger", "add call instructions");
        ctx.dbg.write_memory(mem_ptr as usize, CALL_FN)?;

        debug!(target: "debugger", "prepare function arguments");
        let mut regs: RegisterMap = ctx.regs.clone();
        args.prepare_registers(&mut regs);
        regs.update(Register::Rax, fn_addr);
        regs.persist(ctx.pid)?;

        debug!(target: "debugger", "call a function, wait until breakpoint are hit");
        sys::ptrace::cont(ctx.pid, None).map_err(Error::Ptrace)?;
        let res = nix::sys::wait::waitpid(ctx.pid, None).map_err(Error::Waitpid)?;
        debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

        Ok(())
    }

    fn jump(ctx: &CallContext, dest_ptr: u64) -> Result<(), Error> {
        debug_assert!(ctx.regs.value(Register::Rip) == ctx.pc.as_u64());

        let mut regs = ctx.regs.clone();
        regs.update(Register::Rax, dest_ptr);
        regs.persist(ctx.pid)?;

        const JMP_RAX: usize = 0x000000000000E0FF;
        const JMP_RAX_MASK: usize = 0xFFFFFFFFFFFF0000;

        let new_text = (ctx.text & JMP_RAX_MASK) | JMP_RAX;

        ctx.dbg.write_memory(ctx.pc.as_usize(), new_text)?;

        sys::ptrace::step(ctx.pid, None).map_err(Error::Ptrace)?;
        let res = nix::sys::wait::waitpid(ctx.pid, None).map_err(Error::Waitpid)?;
        debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

        if RegisterMap::current(ctx.pid)?.value(Register::Rip) != dest_ptr {
            return Err(CallError::Jmp.into());
        }

        Ok(())
    }

    fn mmap(ctx: &CallContext) -> Result<u64, Error> {
        debug_assert!(ctx.regs.value(Register::Rip) == ctx.pc.as_u64());

        // Update registers for calling a `mmap` syscall
        let mut regs = ctx.regs.clone();
        const MMAP: u64 = 9;
        const PROT: u64 =
            (nix::libc::PROT_READ | nix::libc::PROT_EXEC | nix::libc::PROT_WRITE) as u64;
        const FLAGS: u64 = (nix::libc::MAP_PRIVATE | nix::libc::MAP_ANONYMOUS) as u64;
        regs.update(Register::Rax, MMAP);
        regs.update(Register::Rdi, 0);
        let page_size = unsafe { nix::libc::sysconf(nix::libc::_SC_PAGESIZE) as u64 };
        regs.update(Register::Rsi, page_size);
        regs.update(Register::Rdx, PROT);
        regs.update(Register::R10, FLAGS);
        regs.update(Register::R8, -1i32 as u64);
        regs.update(Register::R9, 0);

        regs.persist(ctx.pid)?;

        const SYSCALL: usize = 0x000000000000050F;
        const SYSCALL_MASK: usize = 0xFFFFFFFFFFFF0000;

        let new_instructions = (ctx.text & SYSCALL_MASK) | SYSCALL;

        ctx.dbg.write_memory(ctx.pc.as_usize(), new_instructions)?;

        sys::ptrace::step(ctx.pid, None).map_err(Error::Ptrace)?;
        let res = nix::sys::wait::waitpid(ctx.pid, None).map_err(Error::Waitpid)?;
        debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

        let regs = RegisterMap::current(ctx.pid)?;
        let alloc_ptr: u64 = regs.value(Register::Rax);
        if alloc_ptr as i64 == -1 {
            return Err(CallError::Mmap.into());
        }

        debug_assert!(utils::region_exist(ctx.pid, alloc_ptr)?);

        Ok(alloc_ptr)
    }

    fn munmap(ctx: &CallContext, addr: u64) -> Result<(), Error> {
        const SYSCALL: usize = 0x000000000000050F;
        const SYSCALL_MASK: usize = 0xFFFFFFFFFFFF0000;

        let new_text = (ctx.text & SYSCALL_MASK) | SYSCALL;
        ctx.dbg.write_memory(ctx.pc.as_usize(), new_text)?;

        // Update registers for calling a `munmap` syscall
        let mut regs = ctx.regs.clone();
        const MUNMAP: u64 = 11;
        regs.update(Register::Rax, MUNMAP);
        regs.update(Register::Rdi, addr);
        let page_size = unsafe { nix::libc::sysconf(nix::libc::_SC_PAGESIZE) as u64 };
        regs.update(Register::Rsi, page_size);
        regs.persist(ctx.pid)?;

        sys::ptrace::step(ctx.pid, None).map_err(Error::Ptrace)?;
        let res = nix::sys::wait::waitpid(ctx.pid, None).map_err(Error::Waitpid)?;
        debug_assert!(matches!(res, WaitStatus::Stopped(_, _)));

        let regs: RegisterMap = RegisterMap::current(ctx.pid)?;
        if regs.value(Register::Rax) != 0 {
            return Err(CallError::Munmap.into());
        }
        debug_assert!(utils::region_non_exist(ctx.pid, addr)?);

        ctx.dbg.write_memory(ctx.pc.as_usize(), ctx.text)?;

        Ok(())
    }
}

impl Debugger {
    fn search_fn_to_call(
        &self,
        linkage_name_tpl: &str,
        name: Option<&str>,
    ) -> Result<(&DebugInformation, ContextualDieRef<'_, '_, FunctionDie>), CallError> {
        let dwarfs = self.debugee.debug_info_all();

        let mut candidates = dwarfs
            .iter()
            .filter(|dwarf| {
                dwarf.has_debug_info() && dwarf.tpl_in_pub_names(linkage_name_tpl) != Some(false)
            })
            .filter_map(|&dwarf| {
                let funcs = weak_error!(dwarf.search_functions(linkage_name_tpl))?;
                if funcs.is_empty() {
                    return None;
                }
                Some((dwarf, funcs))
            })
            .collect::<Vec<_>>();

        candidates
            .pop_if_cond(|c| c.len() == 1)
            .and_then(|(dwarf, mut funcs)| {
                if name.is_some() {
                    funcs.retain(|f| f.die.base_attributes.name.as_deref() == name);
                }

                funcs.retain(|f| {
                    let low = f
                        .prolog_start_place()
                        .map(|p| usize::from(p.address))
                        .unwrap_or_default();
                    low != 0
                });

                Some((
                    dwarf,
                    // TODO take first suitable, is this a good approach?
                    funcs.pop()?,
                ))
            })
            .ok_or(CallError::FunctionNotFoundOrTooMany)
    }

    fn with_disabled_brkpts<F>(&self, f: F) -> Result<(), Error>
    where
        F: FnOnce(&Self) -> Result<(), Error>,
    {
        debug!(target: "debugger", "disable all active breakpoints");
        for brkpt in self.breakpoints.active_breakpoints() {
            brkpt.disable()?;
        }

        let cb_result = f(self);

        debug!(target: "debugger", "enable all active breakpoints");
        for brkpt in self.breakpoints.active_breakpoints() {
            brkpt
                .enable()
                .expect("enable breakpoint after disable should not leads to error");
        }

        cb_result
    }

    fn call_fn_raw(&self, fn_addr: RelocatedAddress, args: CallArgs) -> Result<(), Error> {
        let call_context = CallContext::new(self)?;

        call_context.with_ctx(|ctx| {
            debug!(target: "debugger", "alloc temporary memory area");
            let alloc_ptr = CallHelper::mmap(ctx)?;

            debug!(target: "debugger", "jump into mmap'ed region");
            CallHelper::jump(ctx, alloc_ptr)?;

            debug!(target: "debugger", "call a given function");
            CallHelper::call_fn(ctx, alloc_ptr, fn_addr.as_u64(), args)?;

            debug!(target: "debugger", "going to original rip");
            ctx.regs.clone().persist(ctx.pid)?;

            debug!(target: "debugger", "dealloc temporary memory area");
            CallHelper::munmap(ctx, alloc_ptr)?;

            Ok(())
        })
    }

    fn call_fn(&self, fn_name: &str, arguments: &[Literal]) -> Result<(), Error> {
        debug!(target: "debugger", "find function address and prepare arguments");

        let (dwarf, func) = self.search_fn_to_call(fn_name, None)?;

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
        self.call_fn_raw(fn_addr, args)
    }

    /// Do a function call.
    ///
    /// # Arguments
    ///
    /// * `fn_name`: function to call.
    /// * `arguments`: list of literals.
    pub fn call(&mut self, fn_name: &str, arguments: &[Literal]) -> Result<(), Error> {
        disable_when_not_stared!(self);

        self.with_disabled_brkpts(|dbg| dbg.call_fn(fn_name, arguments))
    }
}
