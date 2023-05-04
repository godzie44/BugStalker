fn fibonacci(v: u64) -> u64 {
    if v == 1 || v == 0 {
        return v;
    }
    fibonacci(v - 1) + fibonacci(v - 2)
}

#[allow(unconditional_recursion)]
#[allow(clippy::only_used_in_recursion)]
fn infinite_inc(i: u64) -> u64 {
    infinite_inc(i + 1)
}

fn main() {
    println!("{}", fibonacci(19));
    println!("{}", infinite_inc(1));
}
