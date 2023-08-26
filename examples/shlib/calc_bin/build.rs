fn main() {
    println!("cargo:rustc-link-lib=calc_lib");
    // this program will work only from bugstalker directory (cause relative rpath is used),
    // its ok cause this program only for test purposes
    println!("cargo:rustc-link-arg=-Wl,-rpath,./examples/target/debug");
}
