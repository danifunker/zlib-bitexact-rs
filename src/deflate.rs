//! The deflate engine — port of stock zlib 1.3.1 `deflate.c`.
//!
//! Covers `deflate_state`, the level-9 `configuration_table` entry
//! (`good_length=32, max_lazy=258, nice_length=258, max_chain=4096, func=deflate_slow`),
//! `fill_window`, `deflate_slow` (lazy matching), and the `Z_FINISH` one-shot drive.
//!
//! For the CHD configuration only: level 9, raw (`windowBits = -15`), `memLevel = 8`
//! (`hash_bits = 15`, `hash_size = 32768`, `w_size = 32768`), `Z_DEFAULT_STRATEGY`, a single
//! `deflate(Z_FINISH)` over the whole input. No gzip/zlib wrapper, no dictionaries, no flush modes
//! other than finish.
//!
//! Because the oracle is always called with the whole input available and enough output space, the
//! return-and-resume machinery (`need_more`, `flush_pending`, `pending_buf`) collapses: output is
//! appended straight to a `Vec<u8>` and a block flush never has to bail out early. The matching and
//! window bookkeeping are ported faithfully so the produced bytes are identical.
//!
//! C reference: `cref/vendor/zlib/deflate.c` — `deflate_slow`, `fill_window`, `lm_init`,
//! `configuration_table`, the `deflate()` `Z_FINISH` path. See CLAUDE.md for the hazards.

use crate::bitwriter::BitWriter;
use crate::trees::{BL_CODES, CtData, D_CODES, HEAP_SIZE, MAX_BITS, MAX_MATCH, MIN_MATCH};

// --- The one configuration that matters (memLevel 8, windowBits 15, level 9). ---
const W_BITS: usize = 15;
const W_SIZE: usize = 1 << W_BITS; // 32768
const W_MASK: usize = W_SIZE - 1; // 32767
const HASH_BITS: usize = 15; // memLevel + 7
const HASH_SIZE: usize = 1 << HASH_BITS; // 32768
const HASH_MASK: u32 = (HASH_SIZE - 1) as u32; // 32767
const HASH_SHIFT: u32 = HASH_BITS.div_ceil(MIN_MATCH) as u32; // (hash_bits + MIN_MATCH-1)/MIN_MATCH = 5
const LIT_BUFSIZE: usize = 1 << (8 + 6); // 1 << (memLevel + 6) = 16384
const WINDOW_SIZE: usize = 2 * W_SIZE; // 65536

const MIN_LOOKAHEAD: usize = MAX_MATCH + MIN_MATCH + 1; // 262
const WIN_INIT: usize = MAX_MATCH; // 258
const TOO_FAR: usize = 4096;

// configuration_table[9] = {good=32, lazy=258, nice=258, chain=4096, deflate_slow}
const GOOD_MATCH: usize = 32;
const MAX_LAZY_MATCH: usize = 258;
const NICE_MATCH: usize = 258;
const MAX_CHAIN_LENGTH: usize = 4096;
const LEVEL: i32 = 9;

/// The deflate compression state for the one fixed CHD configuration.
pub(crate) struct DeflateState<'a> {
    // ---- input (read_buf source) ----
    pub(crate) input: &'a [u8],
    pub(crate) next_in: usize,
    pub(crate) avail_in: usize,

    // ---- output / bit writer ----
    pub(crate) bw: BitWriter,

    // ---- sliding window & match state ----
    pub(crate) window: Vec<u8>, // window_size bytes
    pub(crate) window_size: usize,
    pub(crate) w_size: usize,
    pub(crate) w_mask: usize,
    pub(crate) prev: Vec<u16>, // w_size entries
    pub(crate) head: Vec<u16>, // hash_size entries
    pub(crate) ins_h: u32,
    pub(crate) hash_mask: u32,
    pub(crate) hash_shift: u32,
    pub(crate) block_start: isize,
    pub(crate) match_length: usize,
    pub(crate) prev_match: usize,
    pub(crate) match_available: bool,
    pub(crate) strstart: usize,
    pub(crate) match_start: usize,
    pub(crate) lookahead: usize,
    pub(crate) prev_length: usize,
    pub(crate) max_chain_length: usize,
    pub(crate) max_lazy_match: usize,
    pub(crate) good_match: usize,
    pub(crate) nice_match: usize,
    pub(crate) level: i32,
    pub(crate) insert: usize,
    pub(crate) high_water: usize,

    // ---- Huffman trees (lent out via mem::take during a block flush) ----
    pub(crate) dyn_ltree: Vec<CtData>, // HEAP_SIZE
    pub(crate) dyn_dtree: Vec<CtData>, // 2*D_CODES+1
    pub(crate) bl_tree: Vec<CtData>,   // 2*BL_CODES+1
    pub(crate) l_max_code: i32,
    pub(crate) d_max_code: i32,
    pub(crate) bl_count: [u16; MAX_BITS + 1],
    pub(crate) heap: [i32; HEAP_SIZE],
    pub(crate) heap_len: usize,
    pub(crate) heap_max: usize,
    pub(crate) depth: [u8; HEAP_SIZE],

    // ---- symbol buffer ----
    pub(crate) sym_buf: Vec<u8>, // lit_bufsize * 3 bytes
    pub(crate) sym_next: usize,
    pub(crate) sym_end: usize,
    pub(crate) opt_len: u32, // ulg (32-bit on the MSVC oracle) — wrap to match
    pub(crate) static_len: u32,
    // (zlib's `matches` field is omitted: it is only read by deflate_stored/deflateParams, neither
    // of which this encode-only, level-9 crate ports, so tracking it would be dead code.)
}

impl<'a> DeflateState<'a> {
    /// Allocate and initialize the state (`deflateInit2_` + `lm_init` for the fixed config).
    pub(crate) fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            next_in: 0,
            avail_in: input.len(),

            bw: BitWriter::new(),

            window: vec![0u8; WINDOW_SIZE],
            window_size: WINDOW_SIZE,
            w_size: W_SIZE,
            w_mask: W_MASK,
            prev: vec![0u16; W_SIZE],
            head: vec![0u16; HASH_SIZE], // CLEAR_HASH: all NIL
            ins_h: 0,
            hash_mask: HASH_MASK,
            hash_shift: HASH_SHIFT,
            block_start: 0,
            match_length: MIN_MATCH - 1,
            prev_match: 0,
            match_available: false,
            strstart: 0,
            match_start: 0,
            lookahead: 0,
            prev_length: MIN_MATCH - 1,
            max_chain_length: MAX_CHAIN_LENGTH,
            max_lazy_match: MAX_LAZY_MATCH,
            good_match: GOOD_MATCH,
            nice_match: NICE_MATCH,
            level: LEVEL,
            insert: 0,
            high_water: 0,

            dyn_ltree: vec![CtData::default(); HEAP_SIZE],
            dyn_dtree: vec![CtData::default(); 2 * D_CODES + 1],
            bl_tree: vec![CtData::default(); 2 * BL_CODES + 1],
            l_max_code: 0,
            d_max_code: 0,
            bl_count: [0; MAX_BITS + 1],
            heap: [0; HEAP_SIZE],
            heap_len: 0,
            heap_max: 0,
            depth: [0; HEAP_SIZE],

            sym_buf: vec![0u8; LIT_BUFSIZE * 3],
            sym_next: 0,
            sym_end: (LIT_BUFSIZE - 1) * 3,
            opt_len: 0,
            static_len: 0,
        }
    }

    /// `MAX_DIST(s)` — the largest allowed match distance.
    #[inline]
    pub(crate) fn max_dist(&self) -> usize {
        self.w_size - MIN_LOOKAHEAD
    }

    /// `read_buf` — copy up to `size` bytes of input into the window at `buf_off`. wrap == 0, so no
    /// adler/crc is computed.
    fn read_buf(&mut self, buf_off: usize, size: usize) -> usize {
        let mut len = self.avail_in;
        if len > size {
            len = size;
        }
        if len == 0 {
            return 0;
        }
        self.avail_in -= len;
        let input = self.input; // &[u8] is Copy: detach from the self borrow
        self.window[buf_off..buf_off + len]
            .copy_from_slice(&input[self.next_in..self.next_in + len]);
        self.next_in += len;
        len
    }

    /// `slide_hash` — shift the hash table down by `w_size` when the window slides.
    fn slide_hash(&mut self) {
        let wsize = self.w_size as u16;
        // `m >= wsize ? m - wsize : NIL` — saturating_sub yields exactly that (NIL == 0).
        for h in self.head.iter_mut() {
            *h = (*h).saturating_sub(wsize);
        }
        for p in self.prev.iter_mut() {
            *p = (*p).saturating_sub(wsize);
        }
    }

    /// `fill_window` — refill the lookahead, sliding the window and hash when needed, and keep the
    /// `high_water`-zeroed guard region past the data so `longest_match` reads defined bytes.
    fn fill_window(&mut self) {
        let wsize = self.w_size;
        loop {
            let mut more = self.window_size - self.lookahead - self.strstart;

            // If the window is almost full, move the upper half to the lower one.
            if self.strstart >= wsize + self.max_dist() {
                self.window.copy_within(wsize..wsize + (wsize - more), 0);
                self.match_start -= wsize;
                self.strstart -= wsize;
                self.block_start -= wsize as isize;
                if self.insert > self.strstart {
                    self.insert = self.strstart;
                }
                self.slide_hash();
                more += wsize;
            }
            if self.avail_in == 0 {
                break;
            }

            let n = self.read_buf(self.strstart + self.lookahead, more);
            self.lookahead += n;

            // Initialize the hash value now that we have some input.
            if self.lookahead + self.insert >= MIN_MATCH {
                let mut str = self.strstart - self.insert;
                self.ins_h = self.window[str] as u32;
                self.update_hash(self.window[str + 1]);
                while self.insert != 0 {
                    self.update_hash(self.window[str + MIN_MATCH - 1]);
                    self.prev[str & self.w_mask] = self.head[self.ins_h as usize];
                    self.head[self.ins_h as usize] = str as u16;
                    str += 1;
                    self.insert -= 1;
                    if self.lookahead + self.insert < MIN_MATCH {
                        break;
                    }
                }
            }

            if !(self.lookahead < MIN_LOOKAHEAD && self.avail_in != 0) {
                break;
            }
        }

        // Zero the WIN_INIT bytes past the data (high water mark bookkeeping).
        if self.high_water < self.window_size {
            let curr = self.strstart + self.lookahead;
            if self.high_water < curr {
                let mut init = self.window_size - curr;
                if init > WIN_INIT {
                    init = WIN_INIT;
                }
                for b in &mut self.window[curr..curr + init] {
                    *b = 0;
                }
                self.high_water = curr + init;
            } else if self.high_water < curr + WIN_INIT {
                let mut init = curr + WIN_INIT - self.high_water;
                if init > self.window_size - self.high_water {
                    init = self.window_size - self.high_water;
                }
                let hw = self.high_water;
                for b in &mut self.window[hw..hw + init] {
                    *b = 0;
                }
                self.high_water += init;
            }
        }
    }

    /// `FLUSH_BLOCK_ONLY` — flush the current block (output is unbounded, so this never bails out).
    fn flush_block(&mut self, last: bool) {
        let (buf, stored_len) = if self.block_start >= 0 {
            let bs = self.block_start as usize;
            (Some(bs), self.strstart - bs)
        } else {
            (None, (self.strstart as isize - self.block_start) as usize)
        };
        self.tr_flush_block(buf, stored_len, last);
        self.block_start = self.strstart as isize;
    }

    /// `deflate_slow` — lazy matching over the whole input with `flush == Z_FINISH`.
    fn deflate_slow(&mut self) {
        loop {
            // Ensure MIN_LOOKAHEAD bytes are available, except at end of input.
            if self.lookahead < MIN_LOOKAHEAD {
                self.fill_window();
                // flush != Z_NO_FLUSH, so we never return need_more here.
                if self.lookahead == 0 {
                    break;
                }
            }

            // Insert the string at strstart and find the head of its hash chain.
            let mut hash_head = 0usize; // NIL
            if self.lookahead >= MIN_MATCH {
                hash_head = self.insert_string(self.strstart);
            }

            // Find the longest match, discarding those <= prev_length.
            self.prev_length = self.match_length;
            self.prev_match = self.match_start;
            self.match_length = MIN_MATCH - 1;

            if hash_head != 0
                && self.prev_length < self.max_lazy_match
                && self.strstart - hash_head <= self.max_dist()
            {
                self.match_length = self.longest_match(hash_head);
                // Z_DEFAULT_STRATEGY: drop a length-3 match that is too far (TOO_FAR).
                if self.match_length == MIN_MATCH && self.strstart - self.match_start > TOO_FAR {
                    self.match_length = MIN_MATCH - 1;
                }
            }

            // If there was a match at the previous step and the current is not better, output it.
            if self.prev_length >= MIN_MATCH && self.match_length <= self.prev_length {
                let max_insert = self.strstart + self.lookahead - MIN_MATCH;

                let bflush = self.tr_tally(
                    self.strstart - 1 - self.prev_match,
                    self.prev_length - MIN_MATCH,
                );

                self.lookahead -= self.prev_length - 1;
                self.prev_length -= 2;
                loop {
                    self.strstart += 1;
                    if self.strstart <= max_insert {
                        hash_head = self.insert_string(self.strstart);
                    }
                    self.prev_length -= 1;
                    if self.prev_length == 0 {
                        break;
                    }
                }
                let _ = hash_head; // last insert head is unused, as in C
                self.match_available = false;
                self.match_length = MIN_MATCH - 1;
                self.strstart += 1;

                if bflush {
                    self.flush_block(false);
                }
            } else if self.match_available {
                // Output the single literal deferred from the previous position.
                let bflush = self.tr_tally(0, self.window[self.strstart - 1] as usize);
                if bflush {
                    self.flush_block(false);
                }
                self.strstart += 1;
                self.lookahead -= 1;
            } else {
                // No previous match to compare with; defer one step.
                self.match_available = true;
                self.strstart += 1;
                self.lookahead -= 1;
            }
        }

        if self.match_available {
            self.tr_tally(0, self.window[self.strstart - 1] as usize);
            self.match_available = false;
        }
        self.insert = if self.strstart < MIN_MATCH - 1 {
            self.strstart
        } else {
            MIN_MATCH - 1
        };
        // flush == Z_FINISH
        self.flush_block(true);
    }
}

/// Compress `input` at the fixed level-9 raw configuration (`deflate(Z_FINISH)` one-shot).
pub(crate) fn deflate_raw_level9(input: &[u8]) -> Vec<u8> {
    let mut s = DeflateState::new(input);
    s.tr_init();
    s.deflate_slow();
    s.bw.out
}
