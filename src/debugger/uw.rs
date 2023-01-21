use crate::debugger::RelocatedAddress;
use nix::unistd::Pid;
use unwind::{Accessors, AddressSpace, Byteorder, Cursor, PTraceState, RegNum};

pub struct KnownPlace {
    pub func_name: String,
    pub start_ip: u64,
    pub offset: u64,
    pub signal_frame: bool,
}

pub struct BacktracePart {
    pub ip: u64,
    pub place: Option<KnownPlace>,
}

pub type Backtrace = Vec<BacktracePart>;

pub fn backtrace(pid: Pid) -> unwind::Result<Backtrace> {
    let state = PTraceState::new(pid.as_raw() as u32)?;
    let address_space = AddressSpace::new(Accessors::ptrace(), Byteorder::DEFAULT)?;
    let mut cursor = Cursor::remote(&address_space, &state)?;
    let mut backtrace = vec![];

    loop {
        let ip = cursor.register(RegNum::IP)?;

        match (cursor.procedure_info(), cursor.procedure_name()) {
            (Ok(ref info), Ok(ref name)) if ip == info.start_ip() + name.offset() => {
                let fn_name = format!("{:#}", rustc_demangle::demangle(name.name()));

                let in_main = fn_name == "main"
                    || fn_name.contains("::main")
                    || fn_name.contains("::thread_start");

                backtrace.push(BacktracePart {
                    ip,
                    place: Some(KnownPlace {
                        func_name: fn_name,
                        start_ip: info.start_ip(),
                        offset: name.offset(),
                        signal_frame: cursor.is_signal_frame().unwrap_or_default(),
                    }),
                });

                if in_main {
                    break;
                }
            }
            _ => {
                backtrace.push(BacktracePart { ip, place: None });
            }
        }

        if !cursor.step()? {
            break;
        }
    }

    Ok(backtrace)
}

pub fn return_addr(pid: Pid) -> unwind::Result<Option<RelocatedAddress>> {
    let state = PTraceState::new(pid.as_raw() as u32)?;
    let address_space = AddressSpace::new(Accessors::ptrace(), Byteorder::DEFAULT)?;
    let mut cursor = Cursor::remote(&address_space, &state)?;

    if !cursor.step()? {
        return Ok(None);
    }

    Ok(Some(
        RelocatedAddress(cursor.register(RegNum::IP)? as usize),
    ))
}
