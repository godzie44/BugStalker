use std::env;

fn raise_signal() {
    unsafe {
        libc::raise(libc::SIGTERM);
    }
}

fn loop_until_signal() {
    let mut counter = 0u64;
    loop {
        counter += 1;
        if counter % 10_000_000 == 0 {
            println!("working: {counter}");
        }
    }
}

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("signal") => raise_signal(),
        Some("loop") => loop_until_signal(),
        Some(value) => eprintln!("unknown mode: {value}"),
        None => raise_signal(),
    }
}
