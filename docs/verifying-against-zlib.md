# Verifying `deflate_raw` against the zlib 1.3.1 oracle

This crate's whole contract is **byte-identity with stock zlib 1.3.1's `deflate`**. The repo no
longer vendors the C oracle (it was removed to keep the tree pure Rust with zero dependencies and
no build script). Instead:

- **In-repo guard:** committed **golden vectors** prove the encoder still matches what zlib 1.3.1
  produced (`cargo test` — no C, no features).
- **Full re-verification:** this document is the process to rebuild a real C oracle and diff against
  it byte-for-byte, e.g. after changing the encoder, retargeting a new zlib, or vetting the crate.

## The contract

`deflate_raw(input)` must equal, byte-for-byte, the output of C zlib **1.3.1**:

```c
deflateInit2(&strm, 9 /*Z_BEST_COMPRESSION*/, Z_DEFLATED,
             -15 /*raw: no header/trailer*/, 8 /*memLevel*/, Z_DEFAULT_STRATEGY);
deflate(&strm, Z_FINISH);   /* one shot over the whole input */
```

This is the exact configuration MAME's CHD codec uses (`chdcodec.cpp:910`). The version matters:
zlib's deflate output changes across versions, and `zlib-rs`/`miniz_oxide`/`flate2`/`libdeflater`
are **not** byte-identical to it. Confirm any oracle reports `zlibVersion() == "1.3.1"`.

## In-repo guard: golden vectors

`tests/golden.rs` asserts `deflate_raw(input) == tests/vectors/<name>.bin` for the curated corpus
in `tests/common/mod.rs`. Those `.bin` files were captured once from the C oracle, so they *are*
zlib 1.3.1's output frozen into the repo. Run them with no toolchain beyond Rust:

```sh
cargo test            # runs the golden-vector test (and doctests)
```

A mismatch means the encoder drifted from zlib 1.3.1. This is the cheap, always-on guard; it covers
the committed corpus only. To check *arbitrary* inputs, or to regenerate the vectors, rebuild a real
oracle as below.

## Full re-verification against a live C oracle

### Option A — standalone C oracle (recommended; version-independent)

A self-contained program that reads stdin and writes the raw DEFLATE stream at the CHD config:

```c
/* oracle.c — stock zlib 1.3.1 deflate at the CHD configuration. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <zlib.h>

int main(void) {
    size_t cap = 1 << 20, len = 0;
    unsigned char *in = malloc(cap);
    for (;;) {
        if (len == cap) { cap *= 2; in = realloc(in, cap); }
        size_t n = fread(in + len, 1, cap - len, stdin);
        len += n;
        if (n == 0) break;
    }
    if (strcmp(zlibVersion(), "1.3.1") != 0) {
        fprintf(stderr, "oracle: zlib %s, need 1.3.1\n", zlibVersion());
        return 4;
    }
    z_stream s; memset(&s, 0, sizeof s);
    if (deflateInit2(&s, 9, Z_DEFLATED, -15, 8, Z_DEFAULT_STRATEGY) != Z_OK) return 2;
    unsigned long bound = deflateBound(&s, (unsigned long)len);
    unsigned char *out = malloc(bound);
    s.next_in = in;   s.avail_in  = (uInt)len;
    s.next_out = out; s.avail_out = (uInt)bound;
    if (deflate(&s, Z_FINISH) != Z_STREAM_END) return 3;
    fwrite(out, 1, s.total_out, stdout);
    deflateEnd(&s);
    return 0;
}
```

Build it against **zlib 1.3.1**. MAME 0.288 vendors exactly that under
`mame/3rdparty/zlib/` (`ZLIB_VERSION "1.3.1"`):

```sh
# system zlib (only if it is 1.3.1):
cc -O2 oracle.c -lz -o oracle
# or against a pinned source tree (deflate/trees/zutil/adler32/crc32 + the headers):
cc -O2 -I path/to/zlib-1.3.1 oracle.c \
   path/to/zlib-1.3.1/{deflate,trees,zutil,adler32,crc32}.c -o oracle
```

Diff arbitrary inputs:

```sh
./oracle < some_input.bin > expected.bin
# compare expected.bin against deflate_raw(some_input.bin)
```

To drive the crate's corpus through it, add a temporary integration test that shells out (no
`build.rs` needed):

```rust
// tests/oracle_compare.rs  (temporary; delete after verifying)
mod common;
use std::io::Write;
use std::process::{Command, Stdio};

fn oracle(input: &[u8]) -> Vec<u8> {
    let mut c = Command::new("./oracle")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(input).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "oracle failed");
    o.stdout
}

#[test]
fn matches_live_oracle() {
    for (name, input) in common::golden_corpus() {
        assert_eq!(zlib_bitexact_rs::deflate_raw(&input), oracle(&input), "{name}");
    }
}
```

`cargo test --test oracle_compare` then checks every case against the live C library. Swap in any
corpus you like (random inputs, real CHD hunks, fuzzing).

### Option B — restore the original FFI differential rig from git history

The repo originally compiled the vendored zlib + an FFI shim via `build.rs` under a `cref` feature
(`tests/differential.rs`). It is preserved in history:

```sh
git log --oneline -- cref/                 # find the last commit that had it
git checkout <that-commit> -- cref build.rs tests/differential.rs
```

Then temporarily restore the manifest wiring (these were removed when the C was dropped):

```toml
[features]
cref = []
[build-dependencies]
cc = "1.2"
```

and run the original byte-for-byte suite (its corpus is broader than the golden set):

```sh
cargo test --features cref          # oracle_is_zlib_131 + deflate_matches_c_oracle
```

Revert the manifest/files afterward to return to the pure-Rust tree.

## Regenerating or extending the golden vectors

The vectors must always be **the oracle's** output, never `deflate_raw`'s (otherwise the test is
circular). To add a case or refresh the set:

1. Add it to `golden_corpus()` in `tests/common/mod.rs`.
2. Produce its expected bytes with a 1.3.1 oracle and write `tests/vectors/<name>.bin` — either pipe
   the input through the standalone `oracle` (Option A), or restore the rig (Option B), whose
   `capture_golden_vectors` test wrote exactly these files.
3. `cargo test` to confirm `deflate_raw` matches the new vector.

Keep the set compact (it ships in the crate); the goal is path coverage — stored/static/dynamic
blocks, multi-block (>16383 symbols), window sliding (>64 KiB), `MIN_MATCH`/`MAX_MATCH` boundaries,
the deep-Huffman (bit-length-overflow) path, and CHD hunk sizes (4096; 2448 multiples; 19584) — not
volume.

## Reference: why each setting matters

| Setting | Value | Effect on the bytes |
| --- | --- | --- |
| level | 9 | `configuration_table[9]`: `good=32, lazy=258, nice=258, chain=4096`, `deflate_slow` (lazy matching) |
| method | `Z_DEFLATED` | the only method |
| windowBits | −15 | **raw**: no 2-byte zlib header, no adler32 trailer |
| memLevel | 8 | `hash_bits=15`, `hash_size=32768`, `lit_bufsize=16384` (→ block flush at 16383 symbols) |
| strategy | `Z_DEFAULT_STRATEGY` | normal lazy-match + dynamic-tree path |
| flush | `Z_FINISH` (once) | whole input compressed in one call |

See [`CLAUDE.md`](../CLAUDE.md) for the bit-exactness hazards and [`ROADMAP.md`](../ROADMAP.md) for
the build history.
