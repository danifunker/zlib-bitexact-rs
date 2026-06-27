//! Byte-exact **golden-vector** regression test — pure Rust, no C, no features.
//!
//! `deflate_raw` must reproduce the committed bytes in `tests/vectors/<name>.bin`, which were
//! captured once from the stock zlib 1.3.1 C oracle. This is the in-repo guard that the encoder
//! stays byte-identical to zlib 1.3.1; to re-verify against a live C oracle (or regenerate /
//! extend these vectors), follow `docs/verifying-against-zlib.md`.

mod common;

use std::path::PathBuf;

#[test]
fn deflate_raw_matches_golden_vectors() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("vectors");

    let mut failures = 0usize;
    for (name, input) in common::golden_corpus() {
        let path = dir.join(format!("{name}.bin"));
        let expected = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "missing golden vector {}: {e}\n\
                 regenerate it with the C oracle per docs/verifying-against-zlib.md",
                path.display()
            )
        });
        let ours = zlib_bitexact_rs::deflate_raw(&input);
        if ours != expected {
            failures += 1;
            let at = ours
                .iter()
                .zip(&expected)
                .position(|(a, b)| a != b)
                .unwrap_or(ours.len().min(expected.len()));
            eprintln!(
                "MISMATCH [{name}] {}-byte input: ours={} bytes, golden={} bytes, first diff @ {at}",
                input.len(),
                ours.len(),
                expected.len(),
            );
        }
    }
    assert_eq!(
        failures, 0,
        "{failures} case(s) diverged from the committed zlib 1.3.1 golden vectors"
    );
}
