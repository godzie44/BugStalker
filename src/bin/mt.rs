use std::process::exit;
use std::thread;
use std::time::Duration;

fn main() {
    let jh1 = thread::spawn(sum1);
    println!("thread 1 spawn");
    let jh2 = thread::spawn(sum2);
    println!("thread 2 spawn");

    let sum1 = jh1.join().unwrap();
    let sum2 = jh2.join().unwrap();

    println!("total {}", sum1 + sum2);

    exit(0);
}

fn sum1() -> i32 {
    let sum3_jh = thread::spawn(sum3);
    thread::sleep(Duration::from_secs(2));
    let mut sum = 0;
    for i in 0..10000 {
        sum += i;
    }
    println!("sum1: {sum}");
    sum3_jh.join().unwrap();
    sum
}

fn sum2() -> i32 {
    let sum3_jh = thread::spawn(sum3);
    thread::sleep(Duration::from_secs(1));
    let mut sum2 = 0;
    for j in 0..20000 {
        sum2 += j;
    }
    println!("sum2: {sum2}");
    sum3_jh.join().unwrap();
    sum2
}

fn sum3() -> i32 {
    thread::sleep(Duration::from_secs(1));
    let mut sum3 = 0;
    for j in 0..10 {
        sum3 += j;
    }
    println!("sum3 (unused): {sum3}");
    sum3
}
