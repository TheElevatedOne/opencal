fn main() {
    // Give the C library a soname so it can be installed as libopencal.so.<major>.
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap();
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libopencal.so.{major}");
}
