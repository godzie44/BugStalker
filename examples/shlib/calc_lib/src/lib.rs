#[no_mangle]
pub extern "C" fn calc_add(a: u32, b: u32) -> u32 {
    a + b
}

#[no_mangle]
pub extern "C" fn calc_sub(a: u32, b: u32) -> u32 {
    a - b
}
