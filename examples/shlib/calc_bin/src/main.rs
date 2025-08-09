unsafe extern "C" {
    pub fn calc_add(a: u32, b: u32) -> u32;
    pub fn calc_sub(a: u32, b: u32) -> u32;
}

pub fn main() {
    let sum_1_2 = unsafe { calc_add(1, 2) };
    let sub_2_1 = unsafe { calc_sub(2, 1) };

    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string()
        + "/examples/target/debug";
    let print_lib =
        unsafe { libloading::Library::new(format!("{cwd}/libprinter_lib.so")).unwrap() };

    let print_sum_fn: libloading::Symbol<unsafe extern "C" fn(u32)> =
        unsafe { print_lib.get(b"print_sum").unwrap() };
    let print_sub_fn: libloading::Symbol<unsafe extern "C" fn(u32)> =
        unsafe { print_lib.get(b"print_sub").unwrap() };

    unsafe {
        print_sum_fn(sum_1_2);
    }
    unsafe {
        print_sub_fn(sub_2_1);
    }
}
