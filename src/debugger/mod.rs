mod breakpoint;
mod dwarf;
mod register;

use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::dwarf::{DwarfContext, EndianRcSlice, Place};
use crate::debugger::register::{
    get_register_from_name, get_register_value, set_register_value, Register,
};
use anyhow::{anyhow, Context};
use nix::errno::Errno;
use nix::libc::{c_void, siginfo_t, uintptr_t};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;
use nix::{libc, sys};
use object::{Object, ObjectKind};
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::error::Error;
use std::io::BufRead;
use std::process::exit;
use std::str::{from_utf8, FromStr};
use std::{fs, io, u64};

pub struct Debugger<'a, R: gimli::Reader> {
    _program: &'a str,
    load_addr: Cell<usize>,
    pid: Pid,
    breakpoints: RefCell<HashMap<usize, Breakpoint>>,
    obj_kind: object::ObjectKind,
    dwarf: DwarfContext<R>,
}

impl<'a> Debugger<'a, gimli::EndianRcSlice<gimli::RunTimeEndian>> {
    pub fn new(program: &'a str, pid: Pid) -> Self {
        let file = fs::File::open(program).unwrap();
        let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
        let object = object::File::parse(&*mmap).unwrap();

        Self {
            load_addr: Cell::new(0),
            _program: program,
            pid,
            breakpoints: Default::default(),
            dwarf: DwarfContext::new(&object).unwrap(),
            obj_kind: object.kind(),
        }
    }

    fn init_load_addr(&self) -> anyhow::Result<()> {
        if self.obj_kind == ObjectKind::Dynamic {
            let addrs = fs::read(format!("/proc/{}/maps", self.pid))?;
            let maps = from_utf8(&addrs)?;
            let first_line = maps
                .lines()
                .next()
                .ok_or_else(|| anyhow!("unexpected line format"))?;
            let addr = first_line
                .split('-')
                .next()
                .ok_or_else(|| anyhow!("unexpected line format"))?;
            let addr = usize::from_str_radix(addr, 16)?;
            self.load_addr.set(addr);
        }
        Ok(())
    }

    pub fn run(&self) -> Result<(), Box<dyn Error>> {
        waitpid(self.pid, None)?;
        self.init_load_addr()?;

        let mut rl = Editor::<()>::new()?;
        if rl.load_history("history.txt").is_err() {
            println!("No previous history.");
        }

        loop {
            let readline = rl.readline(">> ");
            match readline {
                Ok(input) => {
                    rl.add_history_entry(input.as_str());
                    println!("> {}", input);
                    if let Err(e) = self.handle_cmd(&input) {
                        println!("Error: {:?}", e);
                    }
                    rl.add_history_entry(input.as_str());
                }
                Err(ReadlineError::Interrupted) => break,
                Err(ReadlineError::Eof) => break,
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        }

        Ok(())
    }

    fn assert_command_value<'s>(&self, mb_value: Option<&'s str>) -> anyhow::Result<&'s str> {
        mb_value.ok_or_else(|| anyhow!("unknown command format, second parameter must be set"))
    }

    fn handle_cmd(&self, cmd: &str) -> anyhow::Result<()> {
        let args = cmd.split(' ').collect::<Vec<_>>();
        let command = args[0];
        let value = args.get(1).copied();

        match command.to_lowercase().as_str() {
            "c" | "continue" => self
                .continue_execution()
                .context("Failed to continue execution")?,
            "b" | "break" => {
                let value = self.assert_command_value(value)?;
                if value.starts_with("0x") {
                    self.set_breakpoint(
                        usize::from_str_radix(&value[2..], 16)
                            .context("Failed to parse input argument")?,
                    )?
                } else if value.find(':').is_some() {
                    let args = value.split(':').collect::<Vec<_>>();
                    self.set_breakpoint_at_line(args[0], u64::from_str(args[1])?)?
                } else {
                    self.set_breakpoint_at_fn(value)?
                }
            }
            "rm" | "rmbreak" => {
                let value = self.assert_command_value(value)?;
                self.remove_breakpoint(
                    usize::from_str_radix(value, 16).context("Failed to parse input argument")?,
                )?
            }
            "r" | "register" => {
                let value = self.assert_command_value(value)?;
                match value.to_lowercase().as_str() {
                    "dump" => self.print_registers()?,
                    "read" => println!(
                        "{:#0X}",
                        get_register_value(self.pid, get_register_from_name(args[2])?)
                            .context("Failed to get register value")?
                    ),
                    "write" => set_register_value(
                        self.pid,
                        get_register_from_name(args[2])?,
                        u64::from_str_radix(args[3], 16)
                            .context("Failed to parse input argument")?,
                    )?,
                    _ => eprintln!("unknown subcommand"),
                }
            }
            "mem" | "memory" => {
                let value = self.assert_command_value(value)?;
                match value.to_lowercase().as_str() {
                    "read" => println!(
                        "{:#0X}",
                        self.read_memory(
                            u64::from_str_radix(args[2], 16)
                                .context("Failed to parse input argument")?
                                as uintptr_t
                        )
                        .context("Failed to read memory")?
                    ),
                    "write" => self
                        .write_memory(
                            u64::from_str_radix(args[2], 16)
                                .context("Failed to parse input argument")?
                                as uintptr_t,
                            u64::from_str_radix(args[3], 16)
                                .context("Failed to parse input argument")?
                                as uintptr_t,
                        )
                        .context("Failed to write_memory memory")?,
                    _ => eprintln!("unknown subcommand"),
                }
            }
            "step" => {
                if let Some(place) = self.dwarf.find_place_from_pc(self.offset_pc()?) {
                    println!("{}", self.render_source(&place, 1)?);
                }

                self.step_in()?;

                if let Some(place) = self.dwarf.find_place_from_pc(self.offset_pc()?) {
                    println!("{}", self.render_source(&place, 1)?);
                }
            }
            "next" => {
                self.step_over()?;
            }
            "stepout" => {
                self.step_out()?;
            }
            "stepi" => {
                self.single_step_instruction()?;
                let offset_pc = self.offset_pc()?;
                let mb_place = self.dwarf.find_place_from_pc(offset_pc);

                if let Some(place) = mb_place {
                    println!("{}:{}", place.file, place.line_number);
                    println!("{}", self.render_source(&place, 1)?);
                }
            }
            "q" | "quit" => exit(0),
            _ => eprintln!("unknown command"),
        };

        Ok(())
    }

    fn offset_load_addr(&self, addr: usize) -> usize {
        addr - self.load_addr.get()
    }

    fn offset_pc(&self) -> nix::Result<usize> {
        Ok(self.offset_load_addr(self.get_pc()?))
    }

    fn continue_execution(&self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;
        sys::ptrace::cont(self.pid, None)?;
        self.wait_for_signal()
    }

    fn set_breakpoint(&self, addr: usize) -> anyhow::Result<()> {
        let bp = Breakpoint::new(addr, self.pid);
        bp.enable()?;
        self.breakpoints.borrow_mut().insert(addr, bp);
        Ok(())
    }

    fn remove_breakpoint(&self, addr: usize) -> anyhow::Result<()> {
        let bp = self.breakpoints.borrow_mut().remove(&addr);
        if let Some(bp) = bp {
            if bp.is_enabled() {
                bp.disable()?;
            }
        }
        Ok(())
    }

    fn print_registers(&self) -> anyhow::Result<()> {
        register::LIST
            .iter()
            .try_for_each(|descr| -> anyhow::Result<()> {
                let value = get_register_value(self.pid, descr.r)?;
                println!("{:10} {:#0X}", descr.name, value);
                Ok(())
            })
            .context("Failed to print register values")
    }

    fn read_memory(&self, addr: usize) -> nix::Result<uintptr_t> {
        sys::ptrace::read(self.pid, addr as *mut c_void).map(|v| v as uintptr_t)
    }

    fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> nix::Result<()> {
        unsafe { sys::ptrace::write(self.pid, addr as *mut c_void, value as *mut c_void) }
    }

    fn get_pc(&self) -> nix::Result<usize> {
        get_register_value(self.pid, Register::Rip).map(|addr| addr as usize)
    }

    fn set_pc(&self, value: usize) -> nix::Result<()> {
        set_register_value(self.pid, Register::Rip, value as u64)
    }

    fn step_over_breakpoint(&self) -> anyhow::Result<()> {
        let breakpoints = self.breakpoints.borrow();
        let mb_bp = breakpoints.get(&(self.get_pc()? as usize));
        if let Some(bp) = mb_bp {
            if bp.is_enabled() {
                bp.disable()?;
                sys::ptrace::step(self.pid, None)?;
                self.wait_for_signal()?;
                bp.enable()?;
            }
        }
        Ok(())
    }

    fn wait_for_signal(&self) -> anyhow::Result<()> {
        waitpid(self.pid, None)?;

        let info = match sys::ptrace::getsiginfo(self.pid) {
            Ok(info) => info,
            Err(Errno::ESRCH) => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        match info.si_signo {
            libc::SIGTRAP => self.handle_sigtrap(info)?,
            libc::SIGSEGV => println!("Segfault! Reason: {}", info.si_code),
            _ => println!("Receive signal: {}", info.si_signo),
        }
        Ok(())
    }

    fn handle_sigtrap(&self, info: siginfo_t) -> anyhow::Result<()> {
        match info.si_code {
            0x80 | 0x1 => {
                self.set_pc(self.get_pc()? - 1)?;
                let current_pc = self.get_pc()?;
                println!("Hit breakpoint at address {:#X}", current_pc);
                let offset_pc = self.offset_load_addr(current_pc);
                if let Some(place) = self.dwarf.find_place_from_pc(offset_pc) {
                    println!("{}:{}", place.file, place.line_number);
                    println!("{}", self.render_source(&place, 1)?);
                }

                Ok(())
            }
            0x2 => Ok(()),
            _ => Err(anyhow!("Unknown SIGTRAP code: {}", info.si_code)),
        }
    }

    fn single_step_instruction(&self) -> anyhow::Result<()> {
        if self
            .breakpoints
            .borrow()
            .get(&(self.get_pc()? as usize))
            .is_some()
        {
            self.step_over_breakpoint()
        } else {
            sys::ptrace::step(self.pid, None)?;
            self.wait_for_signal()
        }
    }

    fn step_out(&self) -> anyhow::Result<()> {
        let fp = self.get_frame_pointer()?;
        let ret_addr = self.read_memory(fp + 8)?;

        let bp_is_set = self
            .breakpoints
            .borrow()
            .get(&(ret_addr as usize))
            .is_some();
        if bp_is_set {
            self.continue_execution()
        } else {
            self.set_breakpoint(ret_addr)?;
            self.continue_execution()?;
            self.remove_breakpoint(ret_addr)
        }
    }

    fn step_in(&self) -> anyhow::Result<()> {
        let place = self
            .dwarf
            .find_place_from_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        while place
            == self
                .dwarf
                .find_place_from_pc(self.offset_pc()?)
                .ok_or_else(|| anyhow!("unreachable! line not found"))?
        {
            self.single_step_instruction()?
        }

        Ok(())
    }

    fn step_over(&self) -> anyhow::Result<()> {
        let func = self
            .dwarf
            .find_function_from_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        let mut line = self
            .dwarf
            .find_place_from_pc(
                func.low_pc
                    .ok_or_else(|| anyhow!("unreachable: function not found"))?
                    as usize,
            )
            .unwrap();
        let current_line = self.dwarf.find_place_from_pc(self.offset_pc()?).unwrap();

        let mut to_delete = vec![];
        while line.address < func.high_pc.unwrap_or(0) {
            if line.is_stmt {
                let load_addr = self.offset_to_glob_addr(line.address as usize);
                if line.address != current_line.address
                    && self.breakpoints.borrow().get(&load_addr).is_none()
                {
                    self.set_breakpoint(load_addr)?;
                    to_delete.push(load_addr);
                }
            }

            match line.next() {
                None => break,
                Some(n) => line = n,
            }
        }

        let fp = self.get_frame_pointer()?;
        let ret_addr = self.read_memory(fp + 8)?;

        if self.breakpoints.borrow().get(&ret_addr).is_none() {
            self.set_breakpoint(ret_addr)?;
            to_delete.push(ret_addr);
        }

        self.continue_execution()?;

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(addr))?;

        Ok(())
    }

    fn set_breakpoint_at_fn(&self, name: &str) -> anyhow::Result<()> {
        let func = self
            .dwarf
            .find_function_from_name(name)
            .ok_or_else(|| anyhow!("function not found"))?;

        let low_pc = func
            .low_pc
            .ok_or_else(|| anyhow!("invalid function entry"))?;
        let entry = self
            .dwarf
            .find_place_from_pc(low_pc as usize)
            .ok_or_else(|| anyhow!("invalid function entry"))?;
        let entry = entry
            // TODO skip prologue smarter
            .next()
            .ok_or_else(|| anyhow!("invalid function entry"))?;

        self.set_breakpoint(self.offset_to_glob_addr(entry.address as usize))
    }

    fn set_breakpoint_at_line(&self, fine_name: &str, line: u64) -> anyhow::Result<()> {
        if let Some(place) = self.dwarf.find_stmt_line(fine_name, line) {
            self.set_breakpoint(self.offset_to_glob_addr(place.address as usize))?;
        }
        Ok(())
    }

    /// Return value of rbp register.
    /// Note: rust program must compile with -Cforce-frame-pointers=y
    /// TODO make fp calculation with dwarf.
    fn get_frame_pointer(&self) -> nix::Result<usize> {
        register::get_register_value(self.pid, Register::Rbp).map(|fp| fp as usize)
    }

    fn offset_to_glob_addr(&self, addr: usize) -> usize {
        addr + self.load_addr.get()
    }

    fn render_source(&self, place: &Place<EndianRcSlice>, bounds: u64) -> anyhow::Result<String> {
        const DELIMITER: &str = "--------------------";
        let line_number = if place.line_number == 0 {
            1
        } else {
            place.line_number
        };
        let line_pos = line_number - 1;
        let start = if line_pos < bounds {
            0
        } else {
            line_pos - bounds
        };

        let file = fs::File::open(place.file)?;
        let result = io::BufReader::new(file)
            .lines()
            .filter_map(|line| line.ok())
            .enumerate()
            .skip(start as usize)
            .take((bounds * 2 + 1) as usize)
            .fold(DELIMITER.to_string(), |acc, (pos, line)| {
                if pos as u64 == line_pos {
                    acc + "\n" + ">" + &line
                } else {
                    acc + "\n" + &line
                }
            });

        Ok(result + "\n" + DELIMITER)
    }
}
