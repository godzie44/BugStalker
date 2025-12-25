use std::thread::sleep;
use std::time::Duration;

#[inline(never)]
fn busy_work(iterations: i32) -> i32 {
    let mut acc = 0i32;
    for i in 0..iterations {
        acc = acc.wrapping_add(i.wrapping_mul(3));
    }
    acc
}

#[inline(never)]
fn trigger_stop() {
    unsafe {
        libc::raise(libc::SIGSTOP);
    }
}

fn main() {
    let value = busy_work(42);
    println!("value={value}");
    trigger_stop();
    sleep(Duration::from_secs(1));
    println!("resumed");
}
