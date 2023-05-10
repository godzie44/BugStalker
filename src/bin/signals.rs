use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn single_thread_signal() {
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGUSR1, Arc::clone(&term)).unwrap();
    while !term.load(Ordering::Relaxed) {
        thread::sleep(std::time::Duration::from_millis(500));
    }
    println!("got SIGUSR1");
}

fn multi_thread_signal() {
    let t2_lock = Arc::new(Mutex::new(false));

    let t1 = {
        let t2_lock = t2_lock.clone();
        thread::spawn(move || {
            let term = Arc::new(AtomicBool::new(false));
            signal_hook::flag::register(signal_hook::consts::SIGUSR1, Arc::clone(&term)).unwrap();
            while !term.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_millis(100));
            }
            *t2_lock.lock().unwrap() = true;
        })
    };

    let t2 = thread::spawn(move || loop {
        let t2_unlocked = *t2_lock.lock().unwrap();

        if t2_unlocked {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    });

    t1.join().unwrap();
    t2.join().unwrap();

    println!("threads join");
}

fn multi_thread_signal_2() {
    fn wait_sign(sig: nix::libc::c_int) -> impl FnOnce() {
        move || {
            let term = Arc::new(AtomicBool::new(false));
            signal_hook::flag::register(sig, Arc::clone(&term)).unwrap();
            while !term.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_millis(1000));
            }
        }
    }

    let t1 = thread::spawn(wait_sign(signal_hook::consts::SIGUSR1));
    let t2 = thread::spawn(wait_sign(signal_hook::consts::SIGUSR2));

    t1.join().unwrap();
    t2.join().unwrap();

    println!("threads join");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args[1].as_str() {
        "single_thread" => single_thread_signal(),
        "multi_thread" => multi_thread_signal(),
        "multi_thread_multi_signal" => multi_thread_signal_2(),
        _ => panic!("unknown opt"),
    }
}
