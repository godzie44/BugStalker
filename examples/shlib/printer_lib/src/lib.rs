#[no_mangle]
pub extern "C" fn print_sum(num: u32) {
    println!("sum is {num}")
}

#[no_mangle]
pub extern "C" fn print_sub(num: u32) {
    println!("sub is {num}")
}
