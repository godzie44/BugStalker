#[path = "./nptl/pthread_kill.rs"]
mod pthread_kill;

fn main() {
    let value = pthread_kill::compute_value(21);
    println!("computed={value}");
}
