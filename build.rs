fn main() {
    println!("cargo:rustc-link-arg=-Wl,--export-dynamic");
    println!("cargo:rustc-link-tests=-Wl,--export-dynamic");
}
