//! Huffman trees — port of stock zlib 1.3.1 `trees.c`.
//!
//! `build_tree`/`gen_bitlen`/`gen_codes`/`pqdownheap` (tree construction), `scan_tree`/`send_tree`/
//! `build_bl_tree`/`send_all_trees` (the bit-length tree RLE), `compress_block` (emit symbols), and
//! `_tr_flush_block` — including the **stored vs static vs dynamic** block-type cost comparison
//! (`opt_len` vs `static_len` vs the stored-block size) that decides each block's encoding.
//!
//! The static trees and the length/distance code maps are produced at first use by a faithful port
//! of `tr_static_init` (rather than transcribing `trees.h`), so they cannot drift from the C
//! tables. C reference: `cref/vendor/zlib/trees.c`. These cost decisions are bit-exactness-critical
//! — choosing static where zlib chose dynamic (or vice versa) diverges the whole stream. Output
//! goes through `bitwriter.rs` (LSB-first).

use std::sync::OnceLock;

use crate::deflate::DeflateState;

// ===========================================================================
// Constants (deflate.h / zutil.h / trees.c)
// ===========================================================================

pub(crate) const LENGTH_CODES: usize = 29; // number of length codes, not counting END_BLOCK
pub(crate) const LITERALS: usize = 256; // number of literal bytes 0..255
pub(crate) const L_CODES: usize = LITERALS + 1 + LENGTH_CODES; // 286
pub(crate) const D_CODES: usize = 30; // number of distance codes
pub(crate) const BL_CODES: usize = 19; // codes used to transfer the bit lengths
pub(crate) const HEAP_SIZE: usize = 2 * L_CODES + 1; // 573
pub(crate) const MAX_BITS: usize = 15; // all codes must not exceed MAX_BITS bits
pub(crate) const MAX_BL_BITS: usize = 7; // bit length codes must not exceed this

pub(crate) const MIN_MATCH: usize = 3;
pub(crate) const MAX_MATCH: usize = 258;

const END_BLOCK: usize = 256; // end of block literal code
const REP_3_6: usize = 16; // repeat previous bit length 3-6 times
const REPZ_3_10: usize = 17; // repeat a zero length 3-10 times
const REPZ_11_138: usize = 18; // repeat a zero length 11-138 times

const STORED_BLOCK: i32 = 0;
const STATIC_TREES: i32 = 1;
const DYN_TREES: i32 = 2;

const DIST_CODE_LEN: usize = 512;
const SMALLEST: usize = 1; // heap index of least frequent node

// extra bits for each length / distance / bit-length code (`static` so refs are 'static)
static EXTRA_LBITS: [i32; LENGTH_CODES] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
static EXTRA_DBITS: [i32; D_CODES] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
static EXTRA_BLBITS: [i32; BL_CODES] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 7];
/// The lengths of the bit length codes are sent in order of decreasing probability.
const BL_ORDER: [usize; BL_CODES] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

// ===========================================================================
// ct_data — one tree element. The C `union { freq; code; }` / `union { dad; len; }`.
// ===========================================================================

/// A single Huffman tree element. `fc` aliases `Freq` (during build) and `Code` (after);
/// `dl` aliases `Dad` (during build) and `Len` (after). Matches zlib's `ct_data`.
#[derive(Clone, Copy, Default)]
pub(crate) struct CtData {
    /// `freq` (frequency, during tree build) / `code` (bit string, after `gen_codes`).
    pub(crate) fc: u16,
    /// `dad` (parent, during tree build) / `len` (code length, after `gen_bitlen`).
    pub(crate) dl: u16,
}

// ===========================================================================
// Precomputed static data (port of tr_static_init)
// ===========================================================================

/// The immutable tables zlib builds once in `tr_static_init`.
pub(crate) struct StaticData {
    pub(crate) static_ltree: [CtData; L_CODES + 2], // 288
    pub(crate) static_dtree: [CtData; D_CODES],     // 30
    pub(crate) dist_code: [u8; DIST_CODE_LEN],      // 512
    pub(crate) length_code: [u8; MAX_MATCH - MIN_MATCH + 1], // 256
    pub(crate) base_length: [i32; LENGTH_CODES],
    pub(crate) base_dist: [i32; D_CODES],
}

static STATIC_DATA: OnceLock<StaticData> = OnceLock::new();

pub(crate) fn static_data() -> &'static StaticData {
    STATIC_DATA.get_or_init(tr_static_init)
}

/// Reverse the first `len` bits of `code` (zlib `bi_reverse`). IN: 1 <= len <= 15.
fn bi_reverse(mut code: u32, mut len: i32) -> u32 {
    let mut res: u32 = 0;
    loop {
        res |= code & 1;
        code >>= 1;
        res <<= 1;
        len -= 1;
        if len <= 0 {
            break;
        }
    }
    res >> 1
}

/// Generate the codes for a tree given the per-length counts `bl_count` (zlib `gen_codes`).
fn gen_codes(tree: &mut [CtData], max_code: i32, bl_count: &[u16]) {
    let mut next_code = [0u16; MAX_BITS + 1];
    let mut code: u32 = 0;
    for bits in 1..=MAX_BITS {
        code = (code + bl_count[bits - 1] as u32) << 1;
        next_code[bits] = code as u16;
    }
    for n in 0..=max_code {
        let len = tree[n as usize].dl as usize; // Len
        if len == 0 {
            continue;
        }
        tree[n as usize].fc = bi_reverse(next_code[len] as u32, len as i32) as u16; // Code
        next_code[len] += 1;
    }
}

/// Port of `tr_static_init`: build the static literal/distance trees and the length/distance maps.
fn tr_static_init() -> StaticData {
    let mut length_code = [0u8; MAX_MATCH - MIN_MATCH + 1];
    let mut base_length = [0i32; LENGTH_CODES];
    let mut dist_code = [0u8; DIST_CODE_LEN];
    let mut base_dist = [0i32; D_CODES];
    let mut static_ltree = [CtData::default(); L_CODES + 2];
    let mut static_dtree = [CtData::default(); D_CODES];

    // Initialize the mapping length (0..255) -> length code (0..28).
    let mut length = 0i32;
    for code in 0..(LENGTH_CODES - 1) {
        base_length[code] = length;
        for _ in 0..(1 << EXTRA_LBITS[code]) {
            length_code[length as usize] = code as u8;
            length += 1;
        }
    }
    // length == 256 here. Length 255 (match length 258) gets the best (single-code) encoding.
    length_code[(length - 1) as usize] = (LENGTH_CODES - 1) as u8;

    // Initialize the mapping dist (0..32K) -> dist code (0..29).
    let mut dist = 0i32;
    for code in 0..16 {
        base_dist[code] = dist;
        for _ in 0..(1 << EXTRA_DBITS[code]) {
            dist_code[dist as usize] = code as u8;
            dist += 1;
        }
    }
    dist >>= 7; // from now on, all distances are divided by 128
    for code in 16..D_CODES {
        base_dist[code] = dist << 7;
        for _ in 0..(1 << (EXTRA_DBITS[code] - 7)) {
            dist_code[(256 + dist) as usize] = code as u8;
            dist += 1;
        }
    }

    // Construct the codes of the static literal tree.
    let mut bl_count = [0u16; MAX_BITS + 1];
    let mut n = 0usize;
    while n <= 143 {
        static_ltree[n].dl = 8;
        bl_count[8] += 1;
        n += 1;
    }
    while n <= 255 {
        static_ltree[n].dl = 9;
        bl_count[9] += 1;
        n += 1;
    }
    while n <= 279 {
        static_ltree[n].dl = 7;
        bl_count[7] += 1;
        n += 1;
    }
    while n <= 287 {
        static_ltree[n].dl = 8;
        bl_count[8] += 1;
        n += 1;
    }
    // Codes 286 and 287 exist only to build a canonical tree (longest code all ones).
    gen_codes(&mut static_ltree, (L_CODES + 1) as i32, &bl_count);

    // The static distance tree is trivial: all codes 5 bits.
    for (n, e) in static_dtree.iter_mut().enumerate() {
        e.dl = 5;
        e.fc = bi_reverse(n as u32, 5) as u16;
    }

    StaticData {
        static_ltree,
        static_dtree,
        dist_code,
        length_code,
        base_length,
        base_dist,
    }
}

/// `d_code(dist)` — map a distance (already `dist - 1`) to its distance code.
#[inline]
fn d_code(sd: &StaticData, dist: usize) -> usize {
    if dist < 256 {
        sd.dist_code[dist] as usize
    } else {
        sd.dist_code[256 + (dist >> 7)] as usize
    }
}

// ===========================================================================
// Tree descriptors (the static_tree_desc fields, resolved per tree kind)
// ===========================================================================

#[derive(Clone, Copy)]
pub(crate) enum TreeKind {
    Literal,
    Distance,
    BitLength,
}

struct Desc {
    stree: Option<&'static [CtData]>,
    extra: &'static [i32],
    extra_base: usize,
    elems: usize,
    max_length: i32,
}

fn desc_for(kind: TreeKind) -> Desc {
    let sd = static_data();
    match kind {
        TreeKind::Literal => Desc {
            stree: Some(&sd.static_ltree[..]),
            extra: &EXTRA_LBITS,
            extra_base: LITERALS + 1,
            elems: L_CODES,
            max_length: MAX_BITS as i32,
        },
        TreeKind::Distance => Desc {
            stree: Some(&sd.static_dtree[..]),
            extra: &EXTRA_DBITS,
            extra_base: 0,
            elems: D_CODES,
            max_length: MAX_BITS as i32,
        },
        TreeKind::BitLength => Desc {
            stree: None,
            extra: &EXTRA_BLBITS,
            extra_base: 0,
            elems: BL_CODES,
            max_length: MAX_BL_BITS as i32,
        },
    }
}

/// `smaller(tree, n, m, depth)` — heap ordering with depth as tie breaker.
#[inline]
fn smaller(tree: &[CtData], n: i32, m: i32, depth: &[u8]) -> bool {
    let (n, m) = (n as usize, m as usize);
    tree[n].fc < tree[m].fc || (tree[n].fc == tree[m].fc && depth[n] <= depth[m])
}

/// Scan a literal or distance `tree` to accumulate bit-length-code frequencies into `bltree`
/// (zlib `scan_tree`). Also sets the `0xffff` guard at `tree[max_code + 1]` for `send_tree`.
fn scan_tree(tree: &mut [CtData], bltree: &mut [CtData], max_code: i32) {
    let mut prevlen: i32 = -1;
    let mut nextlen = tree[0].dl as i32;
    let mut count = 0i32;
    let mut max_count = 7i32;
    let mut min_count = 4i32;
    if nextlen == 0 {
        max_count = 138;
        min_count = 3;
    }
    tree[(max_code + 1) as usize].dl = 0xffff; // guard
    for n in 0..=max_code {
        let curlen = nextlen;
        nextlen = tree[(n + 1) as usize].dl as i32;
        count += 1;
        if count < max_count && curlen == nextlen {
            continue;
        } else if count < min_count {
            bltree[curlen as usize].fc += count as u16;
        } else if curlen != 0 {
            if curlen != prevlen {
                bltree[curlen as usize].fc += 1;
            }
            bltree[REP_3_6].fc += 1;
        } else if count <= 10 {
            bltree[REPZ_3_10].fc += 1;
        } else {
            bltree[REPZ_11_138].fc += 1;
        }
        count = 0;
        prevlen = curlen;
        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        } else if curlen == nextlen {
            max_count = 6;
            min_count = 3;
        } else {
            max_count = 7;
            min_count = 4;
        }
    }
}

impl DeflateState<'_> {
    // -----------------------------------------------------------------------
    // init_block / tr_init
    // -----------------------------------------------------------------------

    /// `init_block` — zero the dynamic trees for a new block.
    pub(crate) fn init_block(&mut self) {
        for n in 0..L_CODES {
            self.dyn_ltree[n].fc = 0;
        }
        for n in 0..D_CODES {
            self.dyn_dtree[n].fc = 0;
        }
        for n in 0..BL_CODES {
            self.bl_tree[n].fc = 0;
        }
        self.dyn_ltree[END_BLOCK].fc = 1;
        self.opt_len = 0;
        self.static_len = 0;
        self.sym_next = 0;
    }

    /// `_tr_init` — initialize the tree state for a new stream.
    pub(crate) fn tr_init(&mut self) {
        // Static tables are produced lazily by static_data(); descriptors resolved per-call.
        self.bw.bi_buf = 0;
        self.bw.bi_valid = 0;
        self.init_block();
    }

    // -----------------------------------------------------------------------
    // _tr_tally — save match info + tally frequency counts; return true if block full
    // -----------------------------------------------------------------------

    /// `_tr_tally(dist, lc)` (and the inline `_tr_tally_lit`/`_tr_tally_dist`). `dist == 0` means
    /// `lc` is a literal byte; otherwise `lc` is `match_length - MIN_MATCH` and `dist` the distance.
    pub(crate) fn tr_tally(&mut self, dist: usize, lc: usize) -> bool {
        self.sym_buf[self.sym_next] = (dist & 0xff) as u8;
        self.sym_buf[self.sym_next + 1] = (dist >> 8) as u8;
        self.sym_buf[self.sym_next + 2] = lc as u8;
        self.sym_next += 3;
        if dist == 0 {
            self.dyn_ltree[lc].fc += 1; // Freq
        } else {
            let d = dist - 1; // match distance - 1
            let sd = static_data();
            self.dyn_ltree[sd.length_code[lc] as usize + LITERALS + 1].fc += 1;
            self.dyn_dtree[d_code(sd, d)].fc += 1;
        }
        self.sym_next == self.sym_end
    }

    // -----------------------------------------------------------------------
    // build_tree + helpers
    // -----------------------------------------------------------------------

    /// `pqdownheap(tree, k)` — restore the heap property moving down from `k`.
    fn pqdownheap(&mut self, tree: &[CtData], k0: usize) {
        let v = self.heap[k0];
        let mut k = k0;
        let mut j = k << 1; // left son
        while j <= self.heap_len {
            if j < self.heap_len && smaller(tree, self.heap[j + 1], self.heap[j], &self.depth) {
                j += 1;
            }
            if smaller(tree, v, self.heap[j], &self.depth) {
                break;
            }
            self.heap[k] = self.heap[j];
            k = j;
            j <<= 1;
        }
        self.heap[k] = v;
    }

    /// `gen_bitlen(tree)` — compute optimal bit lengths and update `opt_len`/`static_len`.
    fn gen_bitlen(&mut self, tree: &mut [CtData], kind: TreeKind, max_code: i32) {
        let d = desc_for(kind);
        let stree = d.stree;
        let extra = d.extra;
        let base = d.extra_base;
        let max_length = d.max_length;
        let mut overflow = 0i32;

        for bits in 0..=MAX_BITS {
            self.bl_count[bits] = 0;
        }

        tree[self.heap[self.heap_max] as usize].dl = 0; // root of the heap

        for h in (self.heap_max + 1)..HEAP_SIZE {
            let n = self.heap[h] as usize;
            let mut bits = tree[tree[n].dl as usize].dl as i32 + 1; // tree[Dad].Len + 1
            if bits > max_length {
                bits = max_length;
                overflow += 1;
            }
            tree[n].dl = bits as u16; // Len (overwrites Dad)

            if (n as i32) > max_code {
                continue; // not a leaf node
            }

            self.bl_count[bits as usize] += 1;
            let mut xbits = 0i32;
            if n >= base {
                xbits = extra[n - base];
            }
            let f = tree[n].fc as u32; // Freq
            self.opt_len = self
                .opt_len
                .wrapping_add(f.wrapping_mul((bits + xbits) as u32));
            if let Some(st) = stree {
                self.static_len = self
                    .static_len
                    .wrapping_add(f.wrapping_mul((st[n].dl as i32 + xbits) as u32));
            }
        }
        if overflow == 0 {
            return;
        }

        // Find the first bit length which could increase.
        loop {
            let mut bits = max_length - 1;
            while self.bl_count[bits as usize] == 0 {
                bits -= 1;
            }
            self.bl_count[bits as usize] -= 1; // move one leaf down the tree
            self.bl_count[(bits + 1) as usize] += 2; // move one overflow item as its brother
            self.bl_count[max_length as usize] -= 1;
            overflow -= 2;
            if overflow <= 0 {
                break;
            }
        }

        // Recompute all bit lengths, scanning in increasing frequency.
        let mut h = HEAP_SIZE;
        for bits in (1..=max_length).rev() {
            let mut n = self.bl_count[bits as usize];
            while n != 0 {
                h -= 1;
                let m = self.heap[h] as usize;
                if (m as i32) > max_code {
                    continue;
                }
                if tree[m].dl as i32 != bits {
                    self.opt_len = self.opt_len.wrapping_add(
                        (bits as u32)
                            .wrapping_sub(tree[m].dl as u32)
                            .wrapping_mul(tree[m].fc as u32),
                    );
                    tree[m].dl = bits as u16;
                }
                n -= 1;
            }
        }
    }

    /// `build_tree(tree)` — build one Huffman tree, set codes/lengths, return `max_code`.
    fn build_tree(&mut self, tree: &mut [CtData], kind: TreeKind) -> i32 {
        let d = desc_for(kind);
        let stree = d.stree;
        let elems = d.elems;
        let mut max_code: i32 = -1;

        self.heap_len = 0;
        self.heap_max = HEAP_SIZE;

        for (n, t) in tree.iter_mut().enumerate().take(elems) {
            if t.fc != 0 {
                self.heap_len += 1;
                self.heap[self.heap_len] = n as i32;
                max_code = n as i32;
                self.depth[n] = 0;
            } else {
                t.dl = 0;
            }
        }

        // Force at least two codes of non-zero frequency.
        while self.heap_len < 2 {
            let node = if max_code < 2 {
                max_code += 1;
                max_code
            } else {
                0
            };
            self.heap_len += 1;
            self.heap[self.heap_len] = node;
            tree[node as usize].fc = 1;
            self.depth[node as usize] = 0;
            self.opt_len = self.opt_len.wrapping_sub(1);
            if let Some(st) = stree {
                self.static_len = self.static_len.wrapping_sub(st[node as usize].dl as u32);
            }
        }

        // Establish sub-heaps of increasing lengths.
        for n in (1..=self.heap_len / 2).rev() {
            self.pqdownheap(tree, n);
        }

        // Combine the two least frequent nodes repeatedly.
        let mut node = elems;
        loop {
            // pqremove
            let n = self.heap[SMALLEST];
            self.heap[SMALLEST] = self.heap[self.heap_len];
            self.heap_len -= 1;
            self.pqdownheap(tree, SMALLEST);
            let m = self.heap[SMALLEST];

            self.heap_max -= 1;
            self.heap[self.heap_max] = n; // keep the nodes sorted by frequency
            self.heap_max -= 1;
            self.heap[self.heap_max] = m;

            tree[node].fc = tree[n as usize].fc + tree[m as usize].fc;
            self.depth[node] = self.depth[n as usize].max(self.depth[m as usize]) + 1;
            tree[n as usize].dl = node as u16; // Dad
            tree[m as usize].dl = node as u16;

            self.heap[SMALLEST] = node as i32;
            node += 1;
            self.pqdownheap(tree, SMALLEST);

            if self.heap_len < 2 {
                break;
            }
        }

        self.heap_max -= 1;
        self.heap[self.heap_max] = self.heap[SMALLEST];

        self.gen_bitlen(tree, kind, max_code);
        gen_codes(tree, max_code, &self.bl_count);
        max_code
    }

    /// `build_bl_tree` — build the bit-length tree, return `max_blindex`.
    fn build_bl_tree(
        &mut self,
        ltree: &mut [CtData],
        dtree: &mut [CtData],
        bltree: &mut [CtData],
    ) -> i32 {
        scan_tree(ltree, bltree, self.l_max_code);
        scan_tree(dtree, bltree, self.d_max_code);

        self.build_tree(bltree, TreeKind::BitLength);

        let mut max_blindex = (BL_CODES - 1) as i32;
        while max_blindex >= 3 {
            if bltree[BL_ORDER[max_blindex as usize]].dl != 0 {
                break;
            }
            max_blindex -= 1;
        }
        // Include the bit length tree and counts in opt_len.
        self.opt_len = self
            .opt_len
            .wrapping_add(3 * (max_blindex as u32 + 1) + 5 + 5 + 4);
        max_blindex
    }

    // -----------------------------------------------------------------------
    // Emitting: send_code / send_tree / send_all_trees / compress_block / stored
    // -----------------------------------------------------------------------

    /// `send_code(c, tree)` — send a code from a tree (Code + Len bits).
    #[inline]
    fn send_code(&mut self, c: usize, tree: &[CtData]) {
        self.bw.send_bits(tree[c].fc as i32, tree[c].dl as i32);
    }

    /// `send_tree(tree)` — send a literal/distance tree in compressed form using `bltree`.
    fn send_tree(&mut self, tree: &[CtData], bltree: &[CtData], max_code: i32) {
        let mut prevlen: i32 = -1;
        let mut nextlen = tree[0].dl as i32;
        let mut count = 0i32;
        let mut max_count = 7i32;
        let mut min_count = 4i32;
        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        }
        for n in 0..=max_code {
            let curlen = nextlen;
            nextlen = tree[(n + 1) as usize].dl as i32;
            count += 1;
            if count < max_count && curlen == nextlen {
                continue;
            } else if count < min_count {
                loop {
                    self.send_code(curlen as usize, bltree);
                    count -= 1;
                    if count == 0 {
                        break;
                    }
                }
            } else if curlen != 0 {
                if curlen != prevlen {
                    self.send_code(curlen as usize, bltree);
                    count -= 1;
                }
                self.send_code(REP_3_6, bltree);
                self.bw.send_bits(count - 3, 2);
            } else if count <= 10 {
                self.send_code(REPZ_3_10, bltree);
                self.bw.send_bits(count - 3, 3);
            } else {
                self.send_code(REPZ_11_138, bltree);
                self.bw.send_bits(count - 11, 7);
            }
            count = 0;
            prevlen = curlen;
            if nextlen == 0 {
                max_count = 138;
                min_count = 3;
            } else if curlen == nextlen {
                max_count = 6;
                min_count = 3;
            } else {
                max_count = 7;
                min_count = 4;
            }
        }
    }

    /// `send_all_trees` — send the counts, bit-length-code lengths, and the two trees.
    fn send_all_trees(
        &mut self,
        lcodes: i32,
        dcodes: i32,
        blcodes: i32,
        ltree: &[CtData],
        dtree: &[CtData],
        bltree: &[CtData],
    ) {
        self.bw.send_bits(lcodes - 257, 5);
        self.bw.send_bits(dcodes - 1, 5);
        self.bw.send_bits(blcodes - 4, 4);
        for rank in 0..blcodes {
            self.bw
                .send_bits(bltree[BL_ORDER[rank as usize]].dl as i32, 3);
        }
        self.send_tree(ltree, bltree, lcodes - 1);
        self.send_tree(dtree, bltree, dcodes - 1);
    }

    /// `compress_block(ltree, dtree)` — emit the block's symbols from `sym_buf`.
    fn compress_block(&mut self, ltree: &[CtData], dtree: &[CtData]) {
        let sd = static_data();
        let mut sx = 0usize;
        if self.sym_next != 0 {
            loop {
                let dist = (self.sym_buf[sx] as usize) | ((self.sym_buf[sx + 1] as usize) << 8);
                let lc = self.sym_buf[sx + 2] as usize;
                sx += 3;
                if dist == 0 {
                    self.send_code(lc, ltree); // literal byte
                } else {
                    let code = sd.length_code[lc] as usize;
                    self.send_code(code + LITERALS + 1, ltree); // length code
                    let extra = EXTRA_LBITS[code];
                    if extra != 0 {
                        let lc2 = lc - sd.base_length[code] as usize;
                        self.bw.send_bits(lc2 as i32, extra);
                    }
                    let mut d = dist - 1; // match distance - 1
                    let dc = d_code(sd, d);
                    self.send_code(dc, dtree); // distance code
                    let extra = EXTRA_DBITS[dc];
                    if extra != 0 {
                        d -= sd.base_dist[dc] as usize;
                        self.bw.send_bits(d as i32, extra);
                    }
                }
                if sx >= self.sym_next {
                    break;
                }
            }
        }
        self.send_code(END_BLOCK, ltree);
    }

    /// `_tr_stored_block` — emit a stored (uncompressed) block.
    fn tr_stored_block(&mut self, buf: Option<usize>, stored_len: usize, last: bool) {
        self.bw.send_bits((STORED_BLOCK << 1) + last as i32, 3); // block type
        self.bw.bi_windup(); // align on byte boundary
        self.bw.put_short(stored_len as u16);
        self.bw.put_short(!(stored_len as u16));
        if let Some(off) = buf {
            // zmemcpy window[off..off+stored_len] -> output (disjoint fields: bw.out vs window)
            self.bw
                .out
                .extend_from_slice(&self.window[off..off + stored_len]);
        }
    }

    /// `_tr_flush_block` — pick stored/static/dynamic and write the block.
    pub(crate) fn tr_flush_block(&mut self, buf: Option<usize>, stored_len: usize, last: bool) {
        let mut opt_lenb: u32;
        let static_lenb: u32;
        let mut max_blindex = 0i32;

        // Lend the three dynamic trees out so the emit/build helpers never alias `self`.
        let mut ltree = std::mem::take(&mut self.dyn_ltree);
        let mut dtree = std::mem::take(&mut self.dyn_dtree);
        let mut bltree = std::mem::take(&mut self.bl_tree);

        if self.level > 0 {
            // (detect_data_type omitted: it only sets strm->data_type, no output effect.)
            self.l_max_code = self.build_tree(&mut ltree, TreeKind::Literal);
            self.d_max_code = self.build_tree(&mut dtree, TreeKind::Distance);
            max_blindex = self.build_bl_tree(&mut ltree, &mut dtree, &mut bltree);

            opt_lenb = (self.opt_len.wrapping_add(3 + 7)) >> 3;
            static_lenb = (self.static_len.wrapping_add(3 + 7)) >> 3;

            if static_lenb <= opt_lenb {
                opt_lenb = static_lenb;
            }
        } else {
            opt_lenb = (stored_len + 5) as u32;
            static_lenb = opt_lenb;
        }

        if (stored_len + 4) as u32 <= opt_lenb && buf.is_some() {
            self.tr_stored_block(buf, stored_len, last);
        } else if static_lenb == opt_lenb {
            self.bw.send_bits((STATIC_TREES << 1) + last as i32, 3);
            let sd = static_data();
            self.compress_block(&sd.static_ltree, &sd.static_dtree);
        } else {
            self.bw.send_bits((DYN_TREES << 1) + last as i32, 3);
            self.send_all_trees(
                self.l_max_code + 1,
                self.d_max_code + 1,
                max_blindex + 1,
                &ltree,
                &dtree,
                &bltree,
            );
            self.compress_block(&ltree, &dtree);
        }

        // Restore the trees before init_block (which writes them).
        self.dyn_ltree = ltree;
        self.dyn_dtree = dtree;
        self.bl_tree = bltree;

        self.init_block();

        if last {
            self.bw.bi_windup();
        }
    }
}
