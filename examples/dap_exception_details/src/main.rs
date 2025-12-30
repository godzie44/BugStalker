fn main() {
    let input = 7;
    let computed = level_one(input);
    println!("computed: {computed}");
}

fn level_one(value: i32) -> i32 {
    level_two(value + 1)
}

fn level_two(value: i32) -> i32 {
    level_three(value * 2)
}

fn level_three(value: i32) -> i32 {
    let pointer = std::ptr::null_mut::<i32>();
    unsafe {
        *pointer = value; // SIGSEGV for exceptionInfo stack trace
    }
    value
}
