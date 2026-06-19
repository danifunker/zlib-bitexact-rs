//! deflate's bit output — port of stock zlib 1.3.1 `send_bits` / `bi_buf` / `bi_windup`.
//!
//! ⚠️ deflate packs bits **least-significant-first** (the opposite of MAME's MSB-first
//! `bitstream_out`). This is a faithful port of zlib's accumulator:
//!
//! ```c
//! // Buf_size = 16
//! if (s->bi_valid > Buf_size - length) {
//!     s->bi_buf |= (ush)val << s->bi_valid;       // low bits into the accumulator
//!     put_short(s, s->bi_buf);                     // emit 2 bytes, little-endian
//!     s->bi_buf = (ush)val >> (Buf_size - s->bi_valid);
//!     s->bi_valid += length - Buf_size;
//! } else {
//!     s->bi_buf |= (ush)val << s->bi_valid;
//!     s->bi_valid += length;
//! }
//! // bi_windup: flush the remaining bits, padding the final byte with zeros.
//! ```
//!
//! `put_short` writes the low byte then the high byte (little-endian). C reference:
//! `cref/vendor/zlib/trees.c` (`send_bits`, `bi_windup`) and
//! `cref/vendor/zlib/deflate.h` (`put_byte`, `put_short`).

/// Size of bit buffer in `bi_buf` (`Buf_size` in zlib).
const BUF_SIZE: i32 = 16;

/// LSB-first bit accumulator writing straight to the output vector.
///
/// In stock zlib the bytes go to `pending_buf` and are later copied to `next_out` by
/// `flush_pending`. Because this crate runs one-shot with effectively unbounded output,
/// `flush_pending` is a pure in-order pass-through, so we append directly to `out` — the
/// resulting byte stream is identical.
pub(crate) struct BitWriter {
    /// The raw DEFLATE output bytes produced so far.
    pub(crate) out: Vec<u8>,
    /// Output bit buffer; bits are inserted starting at the least significant bit (`bi_buf`).
    pub(crate) bi_buf: u16,
    /// Number of valid bits in `bi_buf`; all bits above the last valid bit are zero (`bi_valid`).
    pub(crate) bi_valid: i32,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self {
            out: Vec::new(),
            bi_buf: 0,
            bi_valid: 0,
        }
    }

    /// `put_byte(s, c)` — append one byte to the pending output.
    #[inline]
    pub(crate) fn put_byte(&mut self, b: u8) {
        self.out.push(b);
    }

    /// `put_short(s, w)` — append a 16-bit value LSB first (little-endian).
    #[inline]
    pub(crate) fn put_short(&mut self, w: u16) {
        self.out.push((w & 0xff) as u8);
        self.out.push((w >> 8) as u8);
    }

    /// `send_bits(s, value, length)` — send `length` low bits of `value`, LSB first.
    ///
    /// IN assertion (as in zlib): `length <= 16` and `value` fits in `length` bits.
    #[inline]
    pub(crate) fn send_bits(&mut self, value: i32, length: i32) {
        // (ush)val: zlib first truncates to 16 bits, then shifts in `int` arithmetic and
        // truncates the `|=` back to 16 bits. Masking to 16 bits up front matches that exactly.
        let v = (value as u32) & 0xffff;
        if self.bi_valid > BUF_SIZE - length {
            self.bi_buf |= (v << (self.bi_valid as u32)) as u16;
            self.put_short(self.bi_buf);
            self.bi_buf = (v >> ((BUF_SIZE - self.bi_valid) as u32)) as u16;
            self.bi_valid += length - BUF_SIZE;
        } else {
            self.bi_buf |= (v << (self.bi_valid as u32)) as u16;
            self.bi_valid += length;
        }
    }

    /// `bi_windup(s)` — flush the bit buffer and align output on a byte boundary, padding the
    /// final byte with zeros.
    pub(crate) fn bi_windup(&mut self) {
        if self.bi_valid > 8 {
            self.put_short(self.bi_buf);
        } else if self.bi_valid > 0 {
            self.put_byte(self.bi_buf as u8);
        }
        self.bi_buf = 0;
        self.bi_valid = 0;
    }
}
