fn main() {
	unsafe {
		let p = 0 as *mut i32;
		*p = 123; // SIGSEGV
	}
}
