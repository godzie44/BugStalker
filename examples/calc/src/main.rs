use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();

    let v1 = &args[1];
    let v2 = &args[2];
    let v3 = &args[3];

    let s: i64 = sum3(
        v1.parse().unwrap(),
        v2.parse().unwrap(),
        v3.parse().unwrap(),
    );
    print(s, &args[5]);
    // just to prevent the compiler from removing dead code
    _ = float::sum3(1f64, 2f64, 3f64);
}

fn sum2(a: i64, b: i64) -> i64 {
    a + b
}

fn sum3(a: i64, b: i64, c: i64) -> i64 {
    let ab = sum2(a, b);
    sum2(ab, c)
}

fn print(v: i64, description: &str) {
    let output = format!("{description}: {v}");

    println!("{output}")
}

pub mod float {
    #[no_mangle]
    pub fn sum2(a: f64, b: f64) -> f64 {
        a + b
    }

    #[no_mangle]
    pub fn sum3(a: f64, b: f64, c: f64) -> f64 {
        let ab = sum2(a, b);
        sum2(ab, c)
    }
}
