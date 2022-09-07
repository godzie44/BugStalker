mod breakpoint;
mod dwarf;
mod register;

use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::dwarf::DwarfContext;
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
use std::str::from_utf8;
use std::{fs, io, u64};

pub struct Debugger<'a, R: gimli::Reader> {
    _program: &'a str,
    load_addr: Cell<u64>,
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
            let addr = u64::from_str_radix(addr, 16)?;
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

    fn handle_cmd(&self, cmd: &str) -> anyhow::Result<()> {
        let args = cmd.split(' ').collect::<Vec<_>>();
        let command = args[0];

        match command.to_lowercase().as_str() {
            "c" | "continue" => self
                .continue_execution()
                .context("Failed to continue execution")?,
            "b" | "break" => self.set_breakpoint(
                usize::from_str_radix(args[1], 16).context("Failed to parse input argument")?,
            ),
            "rm" | "rmbreak" => self.remove_breakpoint(
                usize::from_str_radix(args[1], 16).context("Failed to parse input argument")?,
            ),
            "r" | "register" => match args[1].to_lowercase().as_str() {
                "dump" => self.print_registers()?,
                "read" => println!(
                    "{:#0X}",
                    get_register_value(self.pid, get_register_from_name(args[2])?)
                        .context("Failed to get register value")?
                ),
                "write" => set_register_value(
                    self.pid,
                    get_register_from_name(args[2])?,
                    u64::from_str_radix(args[3], 16).context("Failed to parse input argument")?,
                )?,
                _ => eprintln!("unknown subcommand"),
            },
            "m" | "memory" => match args[1].to_lowercase().as_str() {
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
            },
            // "gl" => {
            //     let addr = self.offset_load_addr(self.get_pc()?);
            //     let line = self
            //         .dwarf
            //         .find_line_from_pc(addr)
            //         .ok_or_else(|| anyhow!("line not found"))?;
            //     println!("current line: {:?}", line);
            // }
            "q" | "quit" => exit(0),
            _ => eprintln!("unknown command"),
        };

        Ok(())
    }

    fn offset_load_addr(&self, addr: u64) -> u64 {
        addr - self.load_addr.get()
    }

    fn continue_execution(&self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;
        sys::ptrace::cont(self.pid, None)?;
        self.wait_for_signal()
    }

    fn set_breakpoint(&self, addr: usize) {
        let bp = Breakpoint::new(addr, self.pid);
        bp.enable().unwrap();
        self.breakpoints.borrow_mut().insert(addr, bp);
    }

    fn remove_breakpoint(&self, addr: usize) {
        let bp = self.breakpoints.borrow_mut().remove(&addr);
        if let Some(bp) = bp {
            bp.disable().unwrap();
        }
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

    fn read_memory(&self, addr: uintptr_t) -> nix::Result<uintptr_t> {
        sys::ptrace::read(self.pid, addr as *mut c_void).map(|v| v as uintptr_t)
    }

    fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> nix::Result<()> {
        unsafe { sys::ptrace::write(self.pid, addr as *mut c_void, value as *mut c_void) }
    }

    fn get_pc(&self) -> nix::Result<u64> {
        get_register_value(self.pid, Register::Rip)
    }

    fn set_pc(&self, value: u64) -> nix::Result<()> {
        set_register_value(self.pid, Register::Rip, value)
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
                let place = self.dwarf.find_line_from_pc(offset_pc).unwrap();
                println!("{}:{}", place.0, place.1.line);
                println!("{}", self.render_source(&place.0, place.1.line, 1)?);

                Ok(())
            }
            0x2 => Ok(()),
            _ => Err(anyhow!("Unknown SIGTRAP code: {}", info.si_code)),
        }
    }

    fn render_source(
        &self,
        file_name: &str,
        line_number: u64,
        bounds: u64,
    ) -> anyhow::Result<String> {
        const DELIMITER: &str = "--------------------";
        let line_pos = line_number - 1;
        let start = line_pos - bounds;

        let file = fs::File::open(file_name)?;
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
