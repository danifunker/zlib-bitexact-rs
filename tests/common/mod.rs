//! Shared, deterministic **golden corpus** for the byte-exact regression test.
//!
//! Compact on purpose: each case's expected raw-DEFLATE output is committed under
//! `tests/vectors/<name>.bin` (captured once from the stock zlib 1.3.1 C oracle — see
//! `docs/verifying-against-zlib.md`). The set is curated to exercise every code path with a small
//! on-disk footprint, not to be exhaustive; the full live differential corpus is reconstructed
//! against the C oracle per that doc.
#![allow(dead_code)]

/// Deterministic xorshift32 byte stream (incompressible -> stored blocks).
pub fn xorshift_bytes(seed: u32, n: usize) -> Vec<u8> {
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

pub const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog. ";

/// Repeat `src` until exactly `len` bytes.
pub fn repeated(src: &[u8], len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len);
    while v.len() < len {
        let take = (len - v.len()).min(src.len());
        v.extend_from_slice(&src[..take]);
    }
    v
}

/// Curated corpus covering every block-type / code path with a small golden footprint:
/// stored / static / dynamic, single- vs multi-block (>16383 symbols), window sliding (>64 KiB),
/// MIN_MATCH/MAX_MATCH boundaries, the deep-Huffman (bit-length overflow) path, and real CHD hunk
/// sizes (4096; 2448 CD-subcode; 19584). Each `<name>` maps to `tests/vectors/<name>.bin`.
pub fn golden_corpus() -> Vec<(&'static str, Vec<u8>)> {
    let mut xs: Vec<(&'static str, Vec<u8>)> = Vec::new();

    xs.push(("empty", Vec::new()));
    xs.push(("one-byte", vec![0x5a]));
    xs.push(("three-same", vec![7, 7, 7])); // MIN_MATCH run boundary
    xs.push(("zeros-258", vec![0u8; 258])); // MAX_MATCH boundary
    xs.push(("zeros-4096", vec![0u8; 4096])); // CHD hunk, very compressible
    xs.push(("zeros-19584", vec![0u8; 19584])); // CHD hunk
    xs.push(("ff-65536", vec![0xffu8; 65536])); // run > 64 KiB -> window slide
    xs.push((
        "ramp-4096",
        (0..4096u32).map(|i| (i & 0xff) as u8).collect(),
    )); // distance-256 matches
    xs.push(("text-8192", repeated(TEXT, 8192))); // matches -> dynamic trees
    xs.push((
        "mixed-4096",
        (0..4096u32)
            .map(|i| match (i / 96) % 3 {
                0 => 0u8,
                1 => TEXT[(i as usize) % TEXT.len()],
                _ => i.wrapping_mul(7) as u8,
            })
            .collect(),
    )); // mixed compressibility, CHD hunk
    xs.push(("cd-subcode-2448", repeated(TEXT, 2448))); // CD subcode hunk size
    xs.push(("rand-4096", xorshift_bytes(0x1234_5678, 4096))); // incompressible -> stored, CHD hunk
    xs.push(("rand-20000", xorshift_bytes(0x9E37_79B9, 20000))); // stored + multi-block (>16383 symbols)
    xs.push(("skewed-geom-40000", {
        // Geometric literal skew (byte = trailing-1-bit count, P(k) ~ 2^-(k+1)) drives Huffman
        // codes past 15 bits, exercising gen_bitlen's bit-length-overflow recompute.
        let mut x: u32 = 0x9E37_79B9;
        (0..40000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x.trailing_ones().min(255) as u8
            })
            .collect()
    }));

    xs
}
