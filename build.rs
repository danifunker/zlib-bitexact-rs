//! Build script. Under the `cref` (C reference) feature, compiles the vendored
//! stock zlib 1.3.1 plus a small FFI shim so tests can diff the Rust deflate
//! against C `deflate()` byte-for-byte. Does nothing for normal builds — the
//! published crate excludes both this script and `cref/`, so consumers get a
//! pure-Rust, dependency-free library.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=cref/shim.c");
    println!("cargo:rerun-if-env-changed=ZLIB_C_DIR");

    // Only touch the C toolchain when the differential rig is requested.
    if std::env::var_os("CARGO_FEATURE_CREF").is_none() {
        return;
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // Default to the vendored copy of stock zlib 1.3.1 (zlib license), so CI and a
    // fresh checkout are self-contained. Override with ZLIB_C_DIR to diff against a
    // different zlib checkout (must be 1.3.1 for byte-identity).
    let c_dir = std::env::var_os("ZLIB_C_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("cref").join("vendor").join("zlib"));

    if !c_dir.join("deflate.c").exists() {
        panic!(
            "feature `cref` is enabled but the zlib C sources were not found at {}.\n\
             Point ZLIB_C_DIR at a directory containing deflate.c (zlib 1.3.1).",
            c_dir.display()
        );
    }

    cc::Build::new()
        .include(&c_dir)
        .warnings(false)
        // Quiet the MSVC CRT-deprecation noise from stock zlib.
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .define("_CRT_NONSTDC_NO_DEPRECATE", None)
        .file(manifest.join("cref").join("shim.c"))
        .file(c_dir.join("deflate.c"))
        .file(c_dir.join("trees.c"))
        .file(c_dir.join("zutil.c"))
        .file(c_dir.join("adler32.c"))
        .file(c_dir.join("crc32.c"))
        .compile("zlib_cref");
}
