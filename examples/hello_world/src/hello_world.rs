use std::thread::sleep;
use std::time::Duration;

fn main() {
    myprint("Hello, world!");

    sleep(Duration::from_secs(1));

    myprint("bye!")
}

#[unsafe(no_mangle)]
#[inline(never)]
fn myprint(s: &str) {
    println!("{}", s)
}
