use std::thread;
use std::time::Duration;

#[derive(Debug)]
struct Stats {
    count: u64,
    flag: bool,
}

fn main() {
    let mut stats = Stats { count: 0, flag: false };
    let mut buffer = [0u8; 8];

    // data breakpoint: watch stats.count or buffer[3] (by expression or address)
    for i in 0..5u8 {
        stats.count += 1;
        buffer[3] = i;
        if i == 2 {
            stats.flag = true;
        }
    }

    println!("{stats:?} {buffer:?}");
    thread::sleep(Duration::from_secs(60));
}
