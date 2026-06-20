# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The version number encodes the targeted upstream release: `0.131.0` is bit-exact
with stock **zlib 1.3.1**, and the patch component counts this crate's own fixes
against that target.

## [Unreleased]

## [0.131.0] - 2026-06-20

First release. A pure-Rust, zero-runtime-dependency port of stock zlib 1.3.1's
deflate whose raw DEFLATE output is **byte-identical to the C `deflate()`** for the
configuration MAME's CHD codec uses — `deflateInit2(9, Z_DEFLATED, -15, 8,
Z_DEFAULT_STRATEGY)` followed by a single `deflate(Z_FINISH)` — enforced by
differential testing against the vendored C on every path.

### Encoder (byte-identical to zlib 1.3.1 `deflate`)

- Raw DEFLATE output: no 2-byte zlib header, no adler32 trailer (windowBits -15).
- The level-9 `configuration_table` entry (`good=32, lazy=258, nice=258,
  chain=4096`, `deflate_slow`) at memLevel 8 (`hash_bits=15`, 32 KiB window,
  16384 `lit_bufsize`).
- LSB-first bit writer (`send_bits` / `bi_buf` / `bi_windup` / `put_short`).
- The `longest_match` hash-chain finder (`max_chain` / `nice` / `good` walk, the
  closer-equal-length tie-break) and `deflate_slow` lazy matching (the one-byte
  deferral, `match_available`, the `TOO_FAR` length-3 drop).
- Huffman trees (`build_tree` / `gen_bitlen` / `gen_codes`, the bit-length-tree
  RLE) and the `_tr_flush_block` stored-vs-static-vs-dynamic cost decision,
  including the bit-length-overflow recompute.
- `fill_window` with window sliding and the high-water guard region; multi-block
  output via the `lit_bufsize` symbol-buffer trigger.

### Testing

- `cref` differential feature: `build.rs` compiles the vendored stock zlib 1.3.1 C
  and the suite asserts byte-for-byte parity across a broad corpus — tiny/boundary
  sizes, long runs, repeated text, incompressible random, skewed / geometric /
  Fibonacci distributions (exercising the bit-length-overflow path), multi-block and
  window-sliding inputs, and CHD hunk sizes (4096 / 2448-multiples / 19584).

[Unreleased]: https://github.com/danifunker/zlib-bitexact-rs/compare/v0.131.0...HEAD
[0.131.0]: https://github.com/danifunker/zlib-bitexact-rs/releases/tag/v0.131.0
