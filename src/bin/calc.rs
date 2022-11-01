fn main() {
    let s: i64 = sum(1, 2);
    print(s);
}

fn sum(a: i64, b: i64) -> i64 {
    a + b
}

fn print(v: i64) {
    let output = format!("result: {v}");

    println!("{output}")
}
