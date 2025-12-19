fn main() {
    if !(cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") && cfg!(target_env = "gnu")) {
        panic!(
            "{} only works with linux using glibc on x86_64",
            env!("CARGO_PKG_NAME")
        );
    }

    println!("cargo:rustc-link-arg=-Wl,--export-dynamic");
	println!("cargo:rustc-link-lib=lzma");
    println!("cargo:rustc-link-tests=-Wl,--export-dynamic");
}
