/// Compiles the C++ COFDM modem shim (core/cpp/shim.cc) against the vendored
/// aicodix header libraries. C++17, exceptions enabled; the `cc` crate links
/// libc++ automatically on macOS and picks up the NDK clang++ under cargo-ndk.
fn main() {
    println!("cargo:rerun-if-changed=cpp/shim.cc");
    println!("cargo:rerun-if-changed=cpp/shim.hh");
    println!("cargo:rerun-if-changed=cpp/cofdm_tables.hh");
    println!("cargo:rerun-if-changed=cpp/aicodix");

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("cpp/shim.cc")
        .include("cpp")
        .include("cpp/aicodix/modem")
        .include("cpp/aicodix/code")
        .include("cpp/aicodix/dsp")
        .opt_level(2)
        .flag_if_supported("-fno-strict-aliasing")
        .warnings(false)
        .compile("aetr_shim");
}
