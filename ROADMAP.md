# ROADMAP — `zlib-bitexact-rs`

An ordered build plan for a **bit-exact** Rust port of stock zlib 1.3.1's deflate, byte-identical
to the C reference. Phases run top to bottom; each lists the C reference, the data shapes, the
bit-exactness traps, and how to verify. See [`CLAUDE.md`](CLAUDE.md) for architecture + hazards.

Legend: `[ ]` todo · `[~]` in progress · `[x]` done & verified

## Target

Byte-identical to C zlib **1.3.1** `deflateInit2(9, Z_DEFLATED, -15, 8, Z_DEFAULT_STRATEGY)` +
one `deflate(Z_FINISH)`. Encode-only, raw DEFLATE, that one configuration.

## The verification rule (governs all work)

Nothing is "done" until `deflate_raw(x)` equals the C oracle byte-for-byte across the corpus —
not merely "valid DEFLATE". Build/test with the oracle:

```sh
cargo test --features cref
cargo fmt && cargo clippy --all-targets --features cref -- -D warnings
```

`build.rs` + `cref/shim.c` compile the vendored zlib 1.3.1 and diff Rust output **byte-for-byte**
(`tests/differential.rs`). Add a corpus per phase; diff at the first mismatching byte.

## Verification rig (the enabler)

- [x] `cref` feature builds the vendored zlib + shim via `cc` (MSVC/clang/gcc); a C-only
      sanity test deflates + inflates a buffer to prove the rig links.
- [x] `zlib_bitexact_rs_cref_version()` asserts the oracle is `"1.3.1"`.
- [x] `differential.rs` harness: `assert_eq!(deflate_raw(x), c_oracle(x))` over a broad corpus
      (ignore removed; first-mismatch offset reported on divergence).

## D0 — Bit writer + stored blocks

- [x] `bitwriter.rs`: LSB-first `send_bits` / `bi_buf` / `bi_windup` / `put_short` (little-endian).
- [x] Stored-block path (`_tr_stored_block`): the 5-byte stored header + raw copy + alignment.
- [x] **Byte-exact vs C** on incompressible input (forces stored blocks). Ref: `trees.c`
      `_tr_stored_block`, `bi_windup`.

## D1 — Static-Huffman blocks

- [x] The fixed literal/length + distance trees — produced by a faithful `tr_static_init` port
      (rather than transcribing `trees.h`, so the tables cannot drift).
- [x] `compress_block` symbol emission via the static trees; `_tr_flush_block` choosing the
      static path when cheapest.
- [x] **Byte-exact vs C** on small/simple inputs that pick static blocks.

## D2 — Match finder

- [x] `longest_match.rs`: hash chains (`head`/`prev`, `INSERT_STRING`/`UPDATE_HASH`), the
      `max_chain`/`nice`/`good` walk and the closer-equal-length tie-break.
- [x] `deflate_slow` lazy matching (the one-byte deferral, `match_available`, the level-9
      thresholds, the `TOO_FAR` length-3 drop) feeding `_tr_tally`.
- [x] Match sequence verified via full byte-exact streams on compressible input (the match
      sequence is implied by byte-identical output).

## D3 — Dynamic Huffman + full deflate

- [x] `trees.c` dynamic trees: `build_tree`/`gen_bitlen`/`gen_codes`/`pqdownheap`, `scan_tree`/
      `send_tree`/`build_bl_tree`/`send_all_trees`, the `_tr_flush_block` stored-vs-static-vs-dynamic
      cost decision, `compress_block` dynamic path. Bit-length-overflow recompute verified hit.
- [x] The `lit_bufsize`/`sym_end` full-symbol-buffer block trigger (multi-block inputs covered).
- [x] **Full corpus byte-exact vs C** at CHD hunk sizes (4096; 2448-multiples; 19584) and beyond.
      (Real captured CHD hunks are exercised in the `chd-rs` integration step below.)

## D4 — Polish & publish

- [x] Removed the scaffold `#![allow(dead_code)]`; `cargo clippy --all-targets --features cref
      -- -D warnings`, `cargo fmt --check`, and doctests all clean.
- [x] Public API: `pub fn deflate_raw(&[u8]) -> Vec<u8>`. README status table.
- [ ] CI (pure-Rust matrix + a Linux cref differential job) + crates.io publish workflow.
- [~] Version `0.131.0` set in `Cargo.toml`; tag `v0.131.0` + confirm `cargo publish --dry-run`
      ships the slim tarball (no `cref/`, no `build.rs`) still pending.
- [ ] In `chd-rs`: swap the zlib encoder to this crate and un-ignore
      `chdman_compat::zlib_bit_exact_vs_chdman` — it must pass against chdman 0.288 (incl. real
      captured CHD hunks).

## Session log

- (start) Scaffold staged from chd-rs: Cargo/build.rs/shim/vendored zlib 1.3.1/src skeleton/docs.
- 2026-06-19: Implemented the full level-9 raw-DEFLATE encoder — `bitwriter` (LSB-first), `trees`
  (static tables via `tr_static_init`, build/flush/compress/stored), `longest_match` (hash-chain
  finder), and the `deflate` engine (`fill_window`, `deflate_slow`, the `Z_FINISH` drive) — as a
  faithful zlib 1.3.1 port. Byte-identical to the C oracle across the corpus (D0–D3 done, incl. the
  bit-length-overflow path verified hit). clippy `-D warnings` / `fmt` / doctests clean and the
  scaffold `#![allow(dead_code)]` removed. D4 remainder: CI, `v0.131.0` tag + publish dry-run, and
  the `chd-rs` integration.
