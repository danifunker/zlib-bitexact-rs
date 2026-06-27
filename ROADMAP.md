# ROADMAP — `zlib-bitexact-rs`

An ordered build plan for a **bit-exact** Rust port of stock zlib 1.3.1's deflate, byte-identical
to the C reference. Phases run top to bottom; each lists the C reference, the data shapes, the
bit-exactness traps, and how to verify. See [`CLAUDE.md`](CLAUDE.md) for architecture + hazards.

Legend: `[ ]` todo · `[~]` in progress · `[x]` done & verified

## Target

Byte-identical to C zlib **1.3.1** `deflateInit2(9, Z_DEFLATED, -15, 8, Z_DEFAULT_STRATEGY)` +
one `deflate(Z_FINISH)`. Encode-only, raw DEFLATE, that one configuration.

## The verification rule (governs all work)

Nothing is "done" until `deflate_raw(x)` equals stock zlib 1.3.1 byte-for-byte — not merely "valid
DEFLATE". The in-repo guard is a pure-Rust golden-vector test (no C, no deps):

```sh
cargo test
cargo fmt && cargo clippy --all-targets -- -D warnings
```

`tests/golden.rs` checks `deflate_raw` against the committed `tests/vectors/*.bin`, captured from
the zlib 1.3.1 C oracle. To re-verify against a live C oracle, diff arbitrary inputs, or regenerate
the vectors, follow [`docs/verifying-against-zlib.md`](docs/verifying-against-zlib.md).

## Verification rig (the enabler) — used during the port, since removed

The D0–D3 port was built against an in-repo C oracle: `build.rs` compiled the vendored zlib 1.3.1
+ `cref/shim.c` via `cc` under a `cref` feature, and `tests/differential.rs` asserted
`deflate_raw(x) == c_oracle(x)` over a broad corpus. **That rig has since been removed** to keep the
tree pure Rust (zero deps, no build script); its byte-for-byte verdict is frozen into the golden
vectors. The full rig is recoverable from git history — see
[`docs/verifying-against-zlib.md`](docs/verifying-against-zlib.md).

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
- [x] CI in `.github/workflows/ci.yml` (all branches + PRs): a pure-Rust `test` matrix
      (ubuntu/windows/macos) that runs the golden-vector byte-exactness test on every platform, and
      a `lint` job (`fmt --check`, `clippy --locked -D warnings`, `cargo doc -D warnings`). Release
      via
      `.github/workflows/publish-crates-io.yml` (`workflow_dispatch` with a tag input → preflight:
      tag↔version match, slim-tarball leak check, 512 KiB cap, artifact upload → `cargo publish`,
      dry-run or real). `CHANGELOG.md` tracks releases.
- [x] Version `0.131.0` **published to crates.io** (2026-06-20) via the `publish-crates-io`
      workflow: tag `v0.131.0` → preflight (tag↔version match, slim tarball: 15 files, 74.9 KiB,
      no `cref/`/`build.rs`) → `cargo publish`. Live at
      <https://crates.io/crates/zlib-bitexact-rs>.
- [ ] In `chd-rs`: swap the zlib encoder to this crate and un-ignore
      `chdman_compat::zlib_bit_exact_vs_chdman` — it must pass against chdman 0.288 (incl. real
      captured CHD hunks). **Return handoff written:** `chd-rs/docs/codec-ports/
      zlib-bitexact-integration.md` (add the dep under `write`, swap `compression/zlib.rs` to
      `deflate_raw`, un-ignore the test). chdman 0.288 bundles zlib **1.3.1** — confirmed aligned.

## Session log

- (start) Scaffold staged from chd-rs: Cargo/build.rs/shim/vendored zlib 1.3.1/src skeleton/docs.
- 2026-06-19: Implemented the full level-9 raw-DEFLATE encoder — `bitwriter` (LSB-first), `trees`
  (static tables via `tr_static_init`, build/flush/compress/stored), `longest_match` (hash-chain
  finder), and the `deflate` engine (`fill_window`, `deflate_slow`, the `Z_FINISH` drive) — as a
  faithful zlib 1.3.1 port. Byte-identical to the C oracle across the corpus (D0–D3 done, incl. the
  bit-length-overflow path verified hit). clippy `-D warnings` / `fmt` / doctests clean and the
  scaffold `#![allow(dead_code)]` removed. D4 remainder: CI, `v0.131.0` tag + publish dry-run, and
  the `chd-rs` integration.
- 2026-06-27: Removed the vendored zlib 1.3.1 C oracle, `build.rs`, and the `cref` feature — the
  crate is now pure Rust with zero dependencies and no build script. Byte-exactness is frozen into
  committed golden vectors (`tests/golden.rs` + `tests/vectors/`, captured from the oracle); CI runs
  them on all platforms. Full live re-verification: `docs/verifying-against-zlib.md`.
