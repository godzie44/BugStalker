mod upt {
    use core::ffi::c_void;
}

use nix::unistd::Pid;
use unwind::{Accessors, AddressSpace, Byteorder, Cursor, PTraceState, RegNum};

pub fn print_backtrace(pid: Pid) {
    let state = PTraceState::new(pid.as_raw() as u32).unwrap();
    let address_space = AddressSpace::new(Accessors::ptrace(), Byteorder::DEFAULT).unwrap();
    let mut cursor = Cursor::remote(&address_space, &state).unwrap();

    loop {
        let ip = cursor.register(RegNum::IP).unwrap();

        match (cursor.procedure_info(), cursor.procedure_name()) {
            (Ok(ref info), Ok(ref name)) if ip == info.start_ip() + name.offset() => {
                let fn_name = format!("{:#}", rustc_demangle::demangle(name.name()));
                println!(
                    "{:#016x} - {} ({:#016x}) + {:#x} {}",
                    ip,
                    fn_name,
                    info.start_ip(),
                    name.offset(),
                    cursor.is_signal_frame().unwrap(),
                );

                if fn_name == "main" || fn_name.contains("::main") {
                    break;
                }
            }
            _ => println!("{:#016x} - ????", ip),
        }

        if !cursor.step().unwrap() {
            break;
        }
    }
}

pub fn return_addr(pid: Pid) -> unwind::Result<Option<usize>> {
    let state = PTraceState::new(pid.as_raw() as u32)?;
    let address_space = AddressSpace::new(Accessors::ptrace(), Byteorder::DEFAULT)?;
    let mut cursor = Cursor::remote(&address_space, &state)?;

    if !cursor.step()? {
        return Ok(None);
    }

    Ok(Some(cursor.register(RegNum::IP)? as usize))
}
