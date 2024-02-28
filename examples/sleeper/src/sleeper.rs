use std::thread::sleep;
use std::time::Duration;
use std::{env, thread};

pub fn main() {
    let args: Vec<String> = env::args().collect();

    let sleep_base_arg_name = &args[1];
    debug_assert!(sleep_base_arg_name == "-s");
    let sleep_base_sec: u64 = args[2].parse().unwrap();

    let mut threads = vec![];
    for i in 0..2 {
        let jh = thread::spawn(move || {
            for _ in 0..2 {
                println!("thread {i} wakeup and sleep again");
                sleep(Duration::from_secs(sleep_base_sec));
            }
        });
        threads.push(jh);
    }
    threads.into_iter().for_each(|t| t.join().unwrap());

    let mut threads = vec![];
    for i in 0..2 {
        let jh = thread::spawn(move || {
            println!("thread {i} created and sleep");
            sleep(Duration::from_secs(sleep_base_sec));
        });
        threads.push(jh);
    }
    threads.into_iter().for_each(|t| t.join().unwrap());
}
