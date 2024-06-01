use std::sync::Mutex;
use std::thread;
use std::time::Duration;

#[allow(clippy::deref_addrof)]
fn calculation_single_value() {
    let mut int8 = 1_i8;
    int8 += 1;
    int8 /= 3;
    println!("{int8}");
    *&mut int8 = -5;
    int8 += 11;
    println!("{int8}");
}

fn calculation_four_value() {
    let mut a = 1_u64;
    let mut b = 2_u64;
    let mut c = 3_u64;
    let mut d = 4_u64;

    a += 5;
    b += 1;
    c -= 2;
    d -= 1;

    println!("{}", a + b + c + d);
}

static mut GLOBAL_1: i64 = 1;

fn calculation_global_value() {
    unsafe {
        GLOBAL_1 -= 1;
        GLOBAL_1 += 3;
        GLOBAL_1 /= 2;

        println!("{GLOBAL_1}");
    }
}

static GLOBAL_2: Mutex<u64> = Mutex::new(1);

fn calculation_global_value_2() {
    let mut lock = GLOBAL_2.lock().unwrap();
    *lock += 1;
    thread::sleep(Duration::from_millis(100));
    drop(lock);
    let mut lock = GLOBAL_2.lock().unwrap();
    *lock += 1;
    thread::sleep(Duration::from_millis(100));
    drop(lock);
}

fn calculation_global_value_mt() {
    let t1 = thread::spawn(calculation_global_value_2);
    let t2 = thread::spawn(calculation_global_value_2);

    t1.join().unwrap();
    t2.join().unwrap();

    println!("{}", GLOBAL_2.lock().unwrap());
}

fn calculation_local_value_mt() {
    let mut a = 1;
    a += 1;
    thread::scope(|t| {
        t.spawn(|| {
            a += 5;
        });
    });

    println!("{a}");
}

#[allow(clippy::useless_vec)]
fn calculation_with_complex_types() {
    let mut vector = vec![1, 2, 3, 4];
    for v in vector.iter_mut() {
        *v += 1;
    }

    struct S {
        b: f64,
    }
    let mut s = S { b: 1_f64 };
    s.b *= 2_f64;

    let mut vector2 = vec![1, 2];
    vector2.push(3);
    vector2.push(4);

    let mut string = "foo".to_string();
    string += " bar";

    println!("v[2] = {}, s.b = {}", vector[2], s.b)
}

fn calculate_from_arg(mut arg: i32) -> i32 {
    arg += 1;
    arg += 2;
    arg -= 5;
    arg
}

pub fn main() {
    calculation_single_value();
    calculation_four_value();
    calculation_global_value();
    calculation_global_value_mt();
    calculation_local_value_mt();
    calculation_with_complex_types();
    calculate_from_arg(1);
}
