//! uniffi-bindgen CLI entry point (library mode).
//!
//! Built only with `--features cli`; `scripts/gen-bindings.sh` runs it
//! against the compiled cdylib to emit the Swift and Kotlin bindings.

/// Delegates straight to uniffi's bundled bindgen CLI.
fn main() {
    uniffi::uniffi_bindgen_main()
}
