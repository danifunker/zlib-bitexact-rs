//! The deflate match finder — port of stock zlib 1.3.1 `longest_match` (`deflate.c`).
//!
//! Hash chains (`head[]` / `prev[]`), the `max_chain_length` walk, the `nice_match` early-out, the
//! `good_match` chain-length halving, and the "prefer the closer match of equal length" tie-break.
//! Plus `INSERT_STRING` / `UPDATE_HASH` (with `hash_shift = (hash_bits + MIN_MATCH - 1) / MIN_MATCH`;
//! `hash_bits = 15` at memLevel 8).
//!
//! This is the non-asm, non-`UNALIGNED_OK` `#else` path (the portable scalar path),
//! rewritten with window indices instead of pointers. The exact sequence of matches it returns
//! determines every output byte. C reference: stock zlib 1.3.1 `deflate.c` `longest_match`.

use crate::deflate::DeflateState;
use crate::trees::{MAX_MATCH, MIN_MATCH};

impl DeflateState<'_> {
    /// `UPDATE_HASH(s, ins_h, c)` — roll the running hash with the next byte.
    #[inline]
    pub(crate) fn update_hash(&mut self, c: u8) {
        self.ins_h = ((self.ins_h << self.hash_shift) ^ (c as u32)) & self.hash_mask;
    }

    /// `INSERT_STRING(s, str, match_head)` — insert `str` into the hash chain, returning the
    /// previous head of that chain.
    #[inline]
    pub(crate) fn insert_string(&mut self, str: usize) -> usize {
        self.update_hash(self.window[str + MIN_MATCH - 1]);
        let h = self.ins_h as usize;
        let match_head = self.head[h];
        self.prev[str & self.w_mask] = match_head;
        self.head[h] = str as u16;
        match_head as usize
    }

    /// `longest_match(s, cur_match)` — set `match_start` to the longest match starting at `strstart`
    /// and return its length. Matches shorter than or equal to `prev_length` are discarded.
    pub(crate) fn longest_match(&mut self, cur_match0: usize) -> usize {
        let mut chain_length = self.max_chain_length; // max hash chain length
        let strstart = self.strstart;
        let mut best_len = self.prev_length; // best match length so far
        let mut nice_match = self.nice_match; // stop if match long enough
        let max_dist = self.max_dist();
        // `strstart > MAX_DIST ? strstart - MAX_DIST : NIL` — equivalent to a saturating sub (NIL == 0).
        let limit = strstart.saturating_sub(max_dist);
        let w_mask = self.w_mask;
        let strend = strstart + MAX_MATCH;
        let mut cur_match = cur_match0;

        let mut scan_end1 = self.window[strstart + best_len - 1];
        let mut scan_end = self.window[strstart + best_len];

        // Do not waste too much time if we already have a good match.
        if self.prev_length >= self.good_match {
            chain_length >>= 2;
        }
        // Do not look for matches beyond the end of the input (keeps deflate deterministic).
        if nice_match > self.lookahead {
            nice_match = self.lookahead;
        }

        loop {
            let m = cur_match;

            // Skip to the next chain entry if the match cannot increase, or the first two bytes
            // differ. (zlib also re-checks best_len - 1; the heuristic is kept verbatim.)
            if self.window[m + best_len] != scan_end
                || self.window[m + best_len - 1] != scan_end1
                || self.window[m] != self.window[strstart]
                || self.window[m + 1] != self.window[strstart + 1]
            {
                // continue: fall through to the chain advance below
            } else {
                // The first two bytes match. C does `scan += 2, match++` (match was already
                // advanced once by the `*++match` check), then compares 8 bytes at a time,
                // checking `scan < strend` only every 8th comparison.
                let mut scan_i = strstart + 2;
                let mut match_i = m + 2;
                'extend: loop {
                    let mut matched = true;
                    for _ in 0..8 {
                        scan_i += 1;
                        match_i += 1;
                        if self.window[scan_i] != self.window[match_i] {
                            matched = false;
                            break;
                        }
                    }
                    if !matched || scan_i >= strend {
                        break 'extend;
                    }
                }

                let len = MAX_MATCH - (strend - scan_i);

                if len > best_len {
                    self.match_start = cur_match;
                    best_len = len;
                    if len >= nice_match {
                        break;
                    }
                    scan_end1 = self.window[strstart + best_len - 1];
                    scan_end = self.window[strstart + best_len];
                }
            }

            // while ((cur_match = prev[cur_match & wmask]) > limit && --chain_length != 0)
            cur_match = self.prev[cur_match & w_mask] as usize;
            if cur_match <= limit {
                break;
            }
            chain_length -= 1;
            if chain_length == 0 {
                break;
            }
        }

        if best_len <= self.lookahead {
            best_len
        } else {
            self.lookahead
        }
    }
}
