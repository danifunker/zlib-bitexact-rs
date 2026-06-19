/* cref/shim.c -- minimal FFI shim over vendored stock zlib 1.3.1, used by the
 * differential tests to compare zlib-bitexact-rs against the C reference. Built
 * only under the `cref` Cargo feature (see build.rs). C89-style declarations for
 * MSVC portability.
 *
 * Reproduces the exact deflate configuration MAME's CHD codec uses
 * (chdcodec.cpp:910): raw DEFLATE (windowBits -15, no zlib header/trailer), level
 * Z_BEST_COMPRESSION (9), memLevel 8, Z_DEFAULT_STRATEGY, one deflate(Z_FINISH). */

#include <stdlib.h>
#include <string.h>

#include "zlib.h"

/* Deflate src[0..src_len] into dst with the CHD configuration. *dst_len is the
 * output capacity on input, the produced length on output. Returns the zlib code
 * (Z_OK == 0 on the final flush means stream end was reached). */
int zlib_bitexact_rs_cref_deflate_raw(const unsigned char *src, size_t src_len,
                                      unsigned char *dst, size_t *dst_len) {
    z_stream strm;
    int ret;

    memset(&strm, 0, sizeof(strm));
    strm.zalloc = Z_NULL;
    strm.zfree = Z_NULL;
    strm.opaque = Z_NULL;

    /* -MAX_WBITS => raw deflate (no 2-byte header, no adler32 trailer). */
    ret = deflateInit2(&strm, Z_BEST_COMPRESSION, Z_DEFLATED,
                       -15 /* -MAX_WBITS */, 8 /* memLevel */,
                       Z_DEFAULT_STRATEGY);
    if (ret != Z_OK) {
        return ret;
    }

    strm.next_in = (Bytef *)src;
    strm.avail_in = (uInt)src_len;
    strm.next_out = (Bytef *)dst;
    strm.avail_out = (uInt)(*dst_len);

    ret = deflate(&strm, Z_FINISH);
    *dst_len = (size_t)strm.total_out;

    deflateEnd(&strm);
    /* Z_STREAM_END (1) means the whole input was compressed and flushed. */
    return ret;
}

/* Report the zlib version string the oracle was built against, so a test can
 * assert it is "1.3.1". */
const char *zlib_bitexact_rs_cref_version(void) {
    return zlibVersion();
}
