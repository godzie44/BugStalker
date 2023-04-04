fn main() {
    let s: i64 = sum3(1, 2, 3);
    print(s);
}

fn sum2(a: i64, b: i64) -> i64 {
    a + b
}

fn sum3(a: i64, b: i64, c: i64) -> i64 {
    let ab = sum2(a, b);
    sum2(ab, c)
}

fn print(v: i64) {
    let output = format!("result: {v}");

    println!("{output}")
}
