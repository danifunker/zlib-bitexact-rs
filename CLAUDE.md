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
cref/
  shim.c            // FFI: zlib_bitexact_rs_cref_deflate_raw (the C oracle entry point)
  vendor/zlib/      // stock zlib 1.3.1 C source (the oracle; deflate.c/trees.c/zutil.c/...)
tests/
  differential.rs   // diff Rust vs C oracle, byte-for-byte
```

C reference for every Rust module is the matching file under `cref/vendor/zlib/`.

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

Nothing is "done" until it is **byte-identical to the C oracle** across the corpus. The rig:

- `build.rs` compiles `cref/vendor/zlib` + `cref/shim.c` under the `cref` feature into a static
  lib; `tests/differential.rs` calls `zlib_bitexact_rs_cref_deflate_raw` over FFI and asserts
  `deflate_raw(x) == c_oracle(x)` byte-for-byte.
- Run: `cargo test --features cref`. Corpus must include: all-zeros, runs, repeated text,
  random/incompressible, and real CHD hunks at CHD hunk sizes (4096; 2448-multiples for CD
  subcode; 19584). Add a per-stage corpus as you build (stored → static → dynamic → full).
- A test asserting `zlib_bitexact_rs_cref_version()` == `"1.3.1"` guards the oracle version.
- Lint: `cargo fmt && cargo clippy --all-targets --features cref -- -D warnings`.
- `#![forbid(unsafe_code)]`; remove the scaffold `#![allow(dead_code)]` at D4.

## Final integration (in `chd-rs`, not here)

`chd-rs` will swap its zlib *encoder* to `deflate_raw`, then flip on the already-written
`chdman_compat::zlib_bit_exact_vs_chdman` test (currently `#[ignore]`) — it must pass against
chdman 0.288. The CD codecs (`cdzl`, and the `cdlz`/`cdfl` subcode streams) use the same
`deflate_raw`. Don't add chd-rs glue here; this crate stays a general zlib-1.3.1 deflate.

## Conventions

- Edition 2024, MSRV 1.85, **zero runtime dependencies** (cc is a build-dep, cref-only).
- Version `0.131.x` (131 = zlib 1.3.1); keep in sync with the git tag.
- `cref/` + `build.rs` are excluded from the published crate (see `Cargo.toml` `exclude`).
- Commit per ROADMAP phase. The vendored `cref/vendor/zlib` tree is third-party — stage paths
  explicitly, don't `git add -A` if line-ending noise appears.
