fn sum2(a: u64, b: u64) {
    println!("my sum is {}", a + b);
}

fn sum6i(a: i8, b: i16, c: i32, d: i64, e: isize, f: i8) {
    println!(
        "my sum is {}",
        a as i64 + b as i64 + c as i64 + d + e as i64 + f as i64
    );
}

fn sum6u(a: u8, b: u16, c: u32, d: u64, e: usize, f: u8) {
    println!(
        "my sum is {}",
        a as u64 + b as u64 + c as u64 + d as u64 + e as u64 + f as u64
    );
}

fn print_bool(b: bool) {
    println!("bool is {}", b);
}

fn main() {
    sum2(1, 2);
    sum6i(1, 2, 3, 4, 5, 6);
    sum6u(1, 2, 3, 4, 5, 6);
    print_bool(true);
}
