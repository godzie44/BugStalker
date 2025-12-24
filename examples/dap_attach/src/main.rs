use std::thread;
use std::time::Duration;

fn main() {
    let pid = std::process::id();
    println!("dap_attach example running (pid={pid})");

    let mut tick = 0u64;
    loop {
        println!("tick {tick}");
        tick += 1;
        thread::sleep(Duration::from_secs(1));
    }
}
