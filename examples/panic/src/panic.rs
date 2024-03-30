use std::env;

fn user_panic() {
    let a = 1;
    let b = a + 1;
    println!("b: {b}");
    panic!("then panic!");
}

#[allow(unconditional_panic)]
fn divided_by_zero() {
    println!("{}", 1 / 0)
}

pub fn main() {
    let args: Vec<String> = env::args().collect();
    let panic_type = args[1].as_str();

    match panic_type {
        "user" => user_panic(),
        "system" => divided_by_zero(),
        _ => panic!("unsupported"),
    }
}
