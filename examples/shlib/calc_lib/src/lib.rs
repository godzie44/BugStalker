#[no_mangle]
pub extern "C" fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[no_mangle]
pub extern "C" fn sub(a: u32, b: u32) -> u32 {
    a - b
}
