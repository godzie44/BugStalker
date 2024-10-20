fn main() {
    let pwd = std::env::var("PWD")
        .map(|pwd| pwd + "/..")
        .unwrap_or_default();
    let base_path = std::env::var("SHLIB_SO_PATH").unwrap_or(pwd);

    println!("cargo:rustc-link-lib=dylib=calc_lib");
    println!("cargo:rustc-link-search=native={base_path}/examples/target/debug");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{base_path}/examples/target/debug");
}
