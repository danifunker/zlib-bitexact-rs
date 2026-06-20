# zlib-bitexact-rs

[![crates.io](https://img.shields.io/crates/v/zlib-bitexact-rs.svg)](https://crates.io/crates/zlib-bitexact-rs)
[![docs.rs](https://img.shields.io/docsrs/zlib-bitexact-rs)](https://docs.rs/zlib-bitexact-rs)
[![CI](https://github.com/danifunker/zlib-bitexact-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/danifunker/zlib-bitexact-rs/actions/workflows/ci.yml)

A pure-Rust, **bit-exact** port of **stock zlib 1.3.1's deflate**. Its raw DEFLATE output is
**byte-identical** to the C `deflate()` for the configuration used by MAME's CHD codec:
`deflateInit2(Z_BEST_COMPRESSION, Z_DEFLATED, -15, 8, Z_DEFAULT_STRATEGY)` + one
`deflate(Z_FINISH)`.

> ⚠️ This is **not** `zlib-rs` / `miniz_oxide` / `flate2`. Those are valid DEFLATE encoders, but
> their compressed *bytes* differ from stock zlib (zlib-ng-style improvements). This crate
> reproduces stock zlib **1.3.1** exactly — needed by consumers that must recreate the precise
> bytes zlib/chdman produce (e.g. [`chd-rs`](https://github.com/SnowflakePowered/chd-rs)
> reproducing MAME CHD hunks bit-for-bit).

Encode only — decode is unambiguous, so any zlib-compatible inflater reads this crate's output.

## Status

✅ **Published on [crates.io](https://crates.io/crates/zlib-bitexact-rs) (v0.131.0) — byte-exact.**
`deflate_raw` reproduces zlib 1.3.1 byte-for-byte across the differential corpus
(`cargo test --features cref`):

| Class | Cases |
|---|---|
| Tiny / boundary | empty, 1–8 bytes, `MIN_MATCH`/`MAX_MATCH`/`MIN_LOOKAHEAD` edges |
| Highly compressible | long zero/0xff runs, periodic patterns, byte ramps |
| Incompressible | xorshift random → stored blocks |
| Block type | dynamic / static / stored, plus multi-regime switches |
| Multi-block | inputs > 16383 symbols (forces `lit_bufsize` flush) |
| Window sliding | inputs > 64 KiB (`fill_window` slide + `slide_hash`) |
| Adversarial | skewed/geometric/Fibonacci frequencies (bit-length-overflow path) |
| CHD hunk sizes | 4096; 2448-multiples (CD subcode); 19584 |

Every encoded byte is differential-tested against the vendored zlib 1.3.1 C source (the `cref`
feature). See [`ROADMAP.md`](ROADMAP.md) for the build plan and [`CLAUDE.md`](CLAUDE.md) for the
architecture and bit-exactness hazards.

## Usage

```rust
let data = b"the quick brown fox jumps over the lazy dog";
let raw = zlib_bitexact_rs::deflate_raw(data);
// == zlib 1.3.1 deflateInit2(9, Z_DEFLATED, -15, 8, Z_DEFAULT_STRATEGY) + deflate(Z_FINISH)
```

Pure Rust, `#![forbid(unsafe_code)]`, zero runtime dependencies.

## Versioning

`0.<zlib-digits>.<patch>` — the minor tracks the upstream zlib release this crate is bit-exact
with (`1.3.1` → `0.131.x`). zlib's deflate output is version-sensitive; this crate targets 1.3.1
exactly (the version MAME 0.288 bundles).

## License

BSD-3-Clause. Ports stock zlib 1.3.1 (`cref/vendor/zlib`), which is under the permissive zlib
license (© Jean-loup Gailly & Mark Adler) — retained in the source headers.
