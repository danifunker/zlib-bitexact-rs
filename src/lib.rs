//! `zlib-bitexact-rs` — a pure-Rust, **bit-exact** port of **stock zlib 1.3.1's deflate**.
//!
//! Its raw DEFLATE output is **byte-identical** to the C `deflate()` for the configuration
//! MAME's CHD codec uses: `deflateInit2(Z_BEST_COMPRESSION, Z_DEFLATED, -15, 8,
//! Z_DEFAULT_STRATEGY)` followed by a single `deflate(Z_FINISH)`. It exists for consumers that
//! must *recreate* the exact bytes zlib/chdman produce (e.g. `chd-rs` reproducing MAME CHD
//! hunks), not merely emit valid DEFLATE — every encoded byte is continuously
//! differential-tested against the real zlib 1.3.1, compiled from source as a dev-only oracle.
//!
//! ⚠️ This is NOT `zlib-rs` / `miniz_oxide` / `flate2` — those are valid DEFLATE encoders but
//! their *compressed bytes* differ from stock zlib (zlib-ng-style improvements). This crate
//! reproduces stock zlib 1.3.1 exactly.
//!
//! Decode is out of scope: inflate is unambiguous, so any zlib-compatible inflater reads this
//! crate's output.
//!
//! # Example
//! ```
//! // `raw` is byte-identical to zlib 1.3.1 deflateInit2(9, Z_DEFLATED, -15, 8, default)
//! // + deflate(Z_FINISH) over the same input.
//! let raw = zlib_bitexact_rs::deflate_raw(b"the quick brown fox jumps over the lazy dog");
//! assert!(!raw.is_empty());
//! ```
//!
//! See `ROADMAP.md` for the build plan and `CLAUDE.md` for the architecture + bit-exactness
//! hazards.

#![forbid(unsafe_code)]

mod bitwriter;
mod deflate;
mod longest_match;
mod trees;

/// Compress `input` into a **raw DEFLATE** stream (no zlib header, no adler32 trailer),
/// byte-identical to stock zlib 1.3.1 with `deflateInit2(9, Z_DEFLATED, -15, 8,
/// Z_DEFAULT_STRATEGY)` + a single `deflate(Z_FINISH)`.
pub fn deflate_raw(input: &[u8]) -> Vec<u8> {
    deflate::deflate_raw_level9(input)
}
