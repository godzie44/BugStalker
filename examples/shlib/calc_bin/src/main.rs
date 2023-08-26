extern "C" {
    pub fn add(a: u32, b: u32) -> u32;
    pub fn sub(a: u32, b: u32) -> u32;
}

pub fn main() {
    let sum_1_2 = unsafe { add(1, 2) };
    println!("1 + 2 = {sum_1_2}");

    let sub_2_1 = unsafe { sub(2, 1) };
    println!("2 - 1 = {sub_2_1}");
}
