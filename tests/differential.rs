//! Differential tests: `deflate_raw` vs the vendored stock zlib 1.3.1 C oracle.
//!
//! Only built/run with the `cref` feature (needs a C compiler):
//! `cargo test --features cref`. The published crate excludes the oracle.
//!
//! `oracle_is_zlib_131` proves the `cref` rig links and pins the oracle version; `corpus` is the
//! byte-for-byte check that `deflate_raw(x) == c_oracle(x)` across a broad input set (stored /
//! static / dynamic blocks, multi-block, window sliding, the bit-length-overflow path, and CHD
//! hunk sizes).
#![cfg(feature = "cref")]

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uchar};

unsafe extern "C" {
    fn zlib_bitexact_rs_cref_deflate_raw(
        src: *const c_uchar,
        src_len: usize,
        dst: *mut c_uchar,
        dst_len: *mut usize,
    ) -> c_int;
    fn zlib_bitexact_rs_cref_version() -> *const c_char;
}

/// Deflate `input` with the C zlib 1.3.1 oracle at the CHD configuration.
fn c_deflate(input: &[u8]) -> Vec<u8> {
    // Generous upper bound (raw deflate worst case is ~src + src/16 + a few bytes).
    let mut out = vec![0u8; input.len() + input.len() / 2 + 128];
    let mut out_len = out.len();
    // SAFETY: valid pointers/lengths; the shim writes <= out_len bytes and updates out_len.
    let ret = unsafe {
        zlib_bitexact_rs_cref_deflate_raw(
            input.as_ptr(),
            input.len(),
            out.as_mut_ptr(),
            &mut out_len,
        )
    };
    assert_eq!(
        ret, 1,
        "C deflate(Z_FINISH) did not return Z_STREAM_END (got {ret})"
    );
    out.truncate(out_len);
    out
}

/// Deterministic xorshift32 pseudo-random byte stream (incompressible -> stored blocks).
fn xorshift_bytes(seed: u32, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x & 0xff) as u8
        })
        .collect()
}

const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog. ";

fn repeated(src: &[u8], len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len);
    while v.len() < len {
        let take = (len - v.len()).min(src.len());
        v.extend_from_slice(&src[..take]);
    }
    v
}

/// A broad, named corpus spanning every code path: stored / static / dynamic block choice,
/// single- vs multi-block (>16383 symbols), window sliding (>64 KiB), the TOO_FAR length-3 drop,
/// MIN_MATCH/MAX_MATCH/MIN_LOOKAHEAD boundaries, and real CHD hunk sizes (4096; 2448-multiples;
/// 19584). Each case must be byte-identical to the C oracle.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut xs: Vec<(String, Vec<u8>)> = Vec::new();
    let mut push = |name: String, data: Vec<u8>| xs.push((name, data));

    // --- tiny / boundary sizes ---
    for n in [0usize, 1, 2, 3, 4, 5, 6, 7, 8, 257, 258, 259, 261, 262, 263] {
        push(format!("zeros-{n}"), vec![0u8; n]);
    }
    push("one-byte".into(), vec![0x5a]);
    push("two-same".into(), vec![0xaa, 0xaa]);
    push("three-same".into(), vec![7, 7, 7]); // MIN_MATCH run boundary
    push("abc".into(), b"abc".to_vec());

    // --- highly compressible runs (single-symbol / long matches) ---
    for n in [4096usize, 19584, 65535, 65536, 65537, 70000, 200_000] {
        push(format!("zeros-{n}"), vec![0u8; n]);
        push(format!("ff-{n}"), vec![0xffu8; n]);
    }

    // --- repeated text (matches, dynamic trees), spanning multi-block + sliding ---
    for n in [45usize, 4096, 8192, 19584, 100_000, 300_000] {
        push(format!("text-{n}"), repeated(TEXT, n));
    }

    // --- periodic byte patterns (various match distances) ---
    for period in [1usize, 2, 3, 4, 7, 16, 256, 1000, 5000] {
        let pat = xorshift_bytes(0xC0FFEE ^ period as u32, period);
        push(format!("periodic-{period}"), repeated(&pat, 120_000));
    }

    // --- incrementing bytes: matches at distance 256 ---
    push(
        "ramp-100k".into(),
        (0..100_000u32).map(|i| (i & 0xff) as u8).collect(),
    );

    // --- xorshift random (incompressible -> stored blocks), incl. multi-block + sliding ---
    for n in [100usize, 4096, 5000, 19584, 33000, 60000, 200_000] {
        push(
            format!("rand-{n}"),
            xorshift_bytes(0x1234_5678 ^ n as u32, n),
        );
    }

    // --- TOO_FAR: length-3 matches at distance > 4096 should be dropped as literals ---
    {
        let mut v = Vec::with_capacity(50_000);
        v.extend_from_slice(b"XYZ");
        v.extend(xorshift_bytes(0xBEEF, 20_000));
        v.extend_from_slice(b"XYZ"); // far repeat of the 3-byte seed
        v.extend(xorshift_bytes(0xFACE, 20_000));
        push("too-far".into(), v);
    }

    // --- mixed compressibility, CHD-hunk-sized and larger ---
    for &n in &[4096usize, 19584, 150_000] {
        let mixed: Vec<u8> = (0..n as u32)
            .map(|i| match (i / 96) % 3 {
                0 => 0u8,
                1 => TEXT[(i as usize) % TEXT.len()],
                _ => i.wrapping_mul(7) as u8,
            })
            .collect();
        push(format!("mixed-{n}"), mixed);
    }

    // --- CHD CD-subcode hunk sizes (2448 multiples) ---
    for mult in [1usize, 2, 8, 75] {
        let n = 2448 * mult;
        push(format!("cd-subcode-{n}"), repeated(TEXT, n));
        push(format!("cd-rand-{n}"), xorshift_bytes(0x5151 ^ n as u32, n));
    }

    // --- two-regime inputs: incompressible then compressible (block-type switch) ---
    {
        let mut v = xorshift_bytes(0xABCD, 40_000);
        v.extend(repeated(TEXT, 40_000));
        v.extend(vec![0u8; 40_000]);
        push("three-regime".into(), v);
    }

    // --- skewed literal distributions: force Huffman codes deeper than 15 bits, exercising the
    //     gen_bitlen bit-length overflow recompute (the "Calgary obj2/pic" path). ---
    {
        // Geometric: byte value = number of trailing 1-bits of a random word (P(k) ~ 2^-(k+1)).
        let mut x: u32 = 0x9E37_79B9;
        let geom: Vec<u8> = (0..250_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x.trailing_ones().min(255)) as u8
            })
            .collect();
        push("skewed-geom".into(), geom);
    }
    {
        // Fibonacci frequencies pushed through a shuffle so the skew lands on literals rather than
        // being eaten by matches: byte value k appears fib(k) times, emitted in xorshift order.
        let mut pool: Vec<u8> = Vec::new();
        let (mut a, mut b) = (1u32, 1u32);
        for k in 0..28u8 {
            for _ in 0..a {
                pool.push(k);
            }
            let n = a + b;
            a = b;
            b = n;
        }
        // Fisher–Yates with xorshift so adjacent bytes rarely repeat (minimizes LZ matches).
        let mut x: u32 = 0x1357_9BDF;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        for i in (1..pool.len()).rev() {
            let j = (next() as usize) % (i + 1);
            pool.swap(i, j);
        }
        push("skewed-fib".into(), pool);
    }

    xs
}

#[test]
fn oracle_is_zlib_131() {
    // SAFETY: the shim returns a static C string.
    let v = unsafe { CStr::from_ptr(zlib_bitexact_rs_cref_version()) };
    assert_eq!(v.to_str().unwrap(), "1.3.1", "oracle must be zlib 1.3.1");
}

/// Index of the first differing byte, for pinpointing divergence.
fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y).or_else(|| {
        if a.len() == b.len() {
            None
        } else {
            Some(a.len().min(b.len()))
        }
    })
}

#[test]
fn deflate_matches_c_oracle() {
    let mut failures = 0usize;
    for (name, input) in corpus() {
        let ours = zlib_bitexact_rs::deflate_raw(&input);
        let theirs = c_deflate(&input);
        if ours != theirs {
            failures += 1;
            let at = first_diff(&ours, &theirs).unwrap_or(0);
            eprintln!(
                "MISMATCH [{name}] {}-byte input: ours={} bytes, zlib={} bytes, first diff @ {at}",
                input.len(),
                ours.len(),
                theirs.len(),
            );
        }
    }
    assert_eq!(
        failures, 0,
        "{failures} corpus case(s) diverged from the oracle"
    );
}
