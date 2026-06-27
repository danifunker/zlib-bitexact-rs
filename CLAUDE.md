# CLAUDE.md — `zlib-bitexact-rs`

Architecture, bit-exactness hazards, and the verification workflow for this crate. Read this
plus `ROADMAP.md` before writing code.

## What this crate is

A pure-Rust port of **stock zlib 1.3.1's deflate** whose output is **byte-identical** to the C
`deflate()` at the configuration MAME's CHD codec uses. It is consumed by `chd-rs` (and anyone
needing zlib-1.3.1-exact deflate). It is **not** a general zlib: encode-only, raw DEFLATE only,
one configuration.

### The one configuration that matters (`chdcodec.cpp:910`)

```c
deflateInit2(&strm, Z_BEST_COMPRESSION /*9*/, Z_DEFLATED,
             -15 /* raw: no header/trailer */, 8 /* memLevel */, Z_DEFAULT_STRATEGY);
deflate(&strm, Z_FINISH);   // one-shot over the whole hunk
```

Level 9 ⇒ `configuration_table[9] = {good=32, lazy=258, nice=258, chain=4096, deflate_slow}`.
memLevel 8 ⇒ `hash_bits=15`, `hash_size=32768`, `lit_bufsize=16384`. Raw ⇒ no 2-byte zlib
header, no adler32 trailer — a bare DEFLATE stream. **Success = byte-identical to that.**

## Why it can't be an existing crate

`zlib-rs`, `miniz_oxide`, `flate2`, `libdeflater` all emit *valid* DEFLATE but **different
bytes** than stock zlib (proven: `zlib-rs` 0.4.2 is ~2 bytes smaller than zlib 1.3.1 on a CHD
hunk). Byte-identical output to a specific zlib version is not a goal of any of them. This crate
is a faithful port of zlib 1.3.1's `deflate.c` + `trees.c`, so it matches exactly.

## Source layout

```
src/
  deflate.rs        // deflate_state, deflate_slow, fill_window, level-9 config, Z_FINISH drive
  longest_match.rs  // hash chains + the match finder (max_chain/nice/good/lazy)
  trees.rs          // build_tree, _tr_flush_block (stored/static/dynamic), compress_block
  bitwriter.rs      // LSB-first bit accumulator (send_bits/bi_buf/bi_windup)
  lib.rs            // pub fn deflate_raw(&[u8]) -> Vec<u8>
tests/
  golden.rs         // assert deflate_raw == tests/vectors/*.bin (pure Rust, no C)
  common/mod.rs     // shared deterministic golden corpus
  vectors/*.bin     // raw-DEFLATE outputs frozen from the zlib 1.3.1 oracle
docs/
  verifying-against-zlib.md  // how to rebuild a C oracle and re-verify byte-for-byte
```

C reference for every Rust module is the matching file in stock zlib 1.3.1 (`deflate.c`, `trees.c`,
`zutil.h`); the crate carries no C. See `docs/verifying-against-zlib.md` to diff against a live
oracle.

## Bit-exactness hazards (where ports go wrong)

1. **LSB-first bit order.** deflate's `send_bits` packs least-significant-first and `put_short`
   is little-endian — the opposite of MAME's MSB-first bitstream. See `bitwriter.rs`.
2. **Lazy matching (`deflate_slow`).** The one-byte match deferral hinges on exact
   `match_length`/`prev_length` comparisons against `good_match`/`max_lazy`. Off-by-one diverges.
3. **`longest_match` tie-breaks.** Chain order, `max_chain_length`, `nice_match` early-out, and
   "prefer the closer match of equal length" must match exactly.
4. **Block-type choice (`_tr_flush_block`).** Stored vs static vs dynamic is a cost comparison
   (`opt_len`/`static_len`/stored size). Picking the wrong one diverges the whole stream.
5. **Hash function.** `UPDATE_HASH` with `hash_shift=(hash_bits+MIN_MATCH-1)/MIN_MATCH`,
   `hash_bits=15` at memLevel 8 — determines chain contents → matches found.
6. **`fill_window` bookkeeping.** `strstart`/`lookahead`/`block_start`/`match_start`. The whole
   hunk fits the 32 KiB window for one-shot, but the window-fill order still matters.

## The verification workflow (the whole game)

Nothing is "done" until it is **byte-identical to stock zlib 1.3.1** across the corpus. The crate
carries no C; verification is in two layers:

- **In-repo guard (always on, pure Rust):** `tests/golden.rs` asserts `deflate_raw(x)` equals the
  committed `tests/vectors/<name>.bin`, captured from the zlib 1.3.1 C oracle. Run: `cargo test`.
  The curated corpus covers stored/static/dynamic blocks, multi-block (>16383 symbols), window
  sliding (>64 KiB), the bit-length-overflow path, and CHD hunk sizes (4096; 2448-multiples; 19584).
- **Full re-verification (when changing the encoder or retargeting zlib):** rebuild a real C oracle
  and diff arbitrary inputs byte-for-byte — see `docs/verifying-against-zlib.md` (standalone
  `oracle.c`, or restore the original FFI `cref` rig from git history). **Never** regenerate a
  golden vector from `deflate_raw` itself — only from the oracle, or the test becomes circular.
- Lint: `cargo fmt && cargo clippy --all-targets -- -D warnings`. `#![forbid(unsafe_code)]`.

## Final integration (in `chd-rs`, not here)

`chd-rs` will swap its zlib *encoder* to `deflate_raw`, then flip on the already-written
`chdman_compat::zlib_bit_exact_vs_chdman` test (currently `#[ignore]`) — it must pass against
chdman 0.288. The CD codecs (`cdzl`, and the `cdlz`/`cdfl` subcode streams) use the same
`deflate_raw`. Don't add chd-rs glue here; this crate stays a general zlib-1.3.1 deflate.

## Conventions

- Edition 2024, MSRV 1.85, **zero dependencies, no build script** (`cargo tree` is empty).
- Version `0.131.x` (131 = zlib 1.3.1); keep in sync with the git tag.
- The published crate excludes only the AI dev guide and CI config (`Cargo.toml` `exclude`); the
  golden-vector tests ship. `tests/vectors/*.bin` are binary fixtures (see `.gitattributes`).
- Commit per ROADMAP phase (for this repo, directly on `main`).
