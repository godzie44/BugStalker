extern "C" {
    pub fn add(a: u32, b: u32) -> u32;
    pub fn sub(a: u32, b: u32) -> u32;
}

pub fn main() {
    let sum_1_2 = unsafe { add(1, 2) };
    let sub_2_1 = unsafe { sub(2, 1) };

    let print_lib =
        unsafe { libloading::Library::new("./examples/target/debug/libprinter_lib.so").unwrap() };

    let print_sum_fn: libloading::Symbol<unsafe extern "C" fn(u32)> =
        unsafe { print_lib.get(b"print_sum").unwrap() };
    let print_sub_fn: libloading::Symbol<unsafe extern "C" fn(u32)> =
        unsafe { print_lib.get(b"print_sub").unwrap() };

    unsafe {
        print_sum_fn(sum_1_2);
        print_sub_fn(sub_2_1);
    }
}
