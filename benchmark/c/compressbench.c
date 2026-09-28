/* GROUP: compress — the C oracle for compress.mfb (the compress:: package).
 * The peer calls system zlib (linked with -lz), C's standard means of doing
 * the job: crc32(), and deflate()/inflate() with raw (-15) and gzip (31)
 * window bits at level 6.
 *
 * crc32 is a fixed function of the bytes. The deflate/inflate/gzip rows never
 * checksum a compressed length (zlib's bytes legitimately differ from mfb's
 * encoder): per repetition they add len + crc32 of the round-tripped
 * (decompressed) data, which matches mfb and Python bit-for-bit. `inflate`
 * decodes zlib's own level-6 stream of the same text.
 *
 * Input: the same deterministic ~128 KB word-salad text as compress.mfb /
 * compressbench.py, built once, untimed. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <zlib.h>

#include "bench.h"
#include "compressbench.h"

#define COMPRESS_TARGET 131072
#define CRC_REPS 64
#define DEFLATE_REPS 4
#define INFLATE_REPS 16
#define GZIP_REPS 4
#define RAW_BITS (-15)
#define GZIP_BITS 31

/* LCG x = (x*1103515245 + 12345) mod 2^31 picks word (x/65536) mod 16; the
 * separator is '\n' when (x/1024) mod 13 == 0, else ' '. Stops once at least
 * `target` bytes are produced. */
static unsigned char *compress_input(size_t target, size_t *out_len) {
  static const char *words[16] = {
      "the",     "quick",    "brown",   "fox",     "jumps",   "over",
      "lazy",    "dog",      "compress", "deflate", "window",  "huffman",
      "literal", "length",   "distance", "block"};
  size_t cap = target + 16, n = 0;
  unsigned char *buf = malloc(cap);
  long long x = 12345;
  while (n < target) {
    x = (x * 1103515245LL + 12345) % 2147483648LL;
    const char *w = words[(x / 65536) % 16];
    size_t wl = strlen(w);
    memcpy(buf + n, w, wl);
    n += wl;
    buf[n++] = ((x / 1024) % 13 == 0) ? '\n' : ' ';
  }
  *out_len = n;
  return buf;
}

/* ----- compress crc32: 64x crc32 of the input ---------------------------- */

static void test_compress_crc32(void) {
  size_t n;
  unsigned char *x = compress_input(COMPRESS_TARGET, &n);
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    for (int rep = 0; rep < CRC_REPS; rep++) {
      bench_opaque(x);
      acc += (long long)crc32(0L, x, (uInt)n);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_compress_crc32 = %lld\n", checksum);
  record("compress", "crc32", t, RUN);
  free(t);
  free(x);
}

/* Compress x[0..n) with zlib at level 6 into a malloc'd buffer; `bits` picks
 * the wrapper (RAW_BITS raw DEFLATE, GZIP_BITS gzip). */
static unsigned char *z_encode(const unsigned char *x, size_t n, int bits, size_t *out_len) {
  z_stream s;
  memset(&s, 0, sizeof s);
  if (deflateInit2(&s, 6, Z_DEFLATED, bits, 8, Z_DEFAULT_STRATEGY) != Z_OK) abort();
  uLong cap = deflateBound(&s, (uLong)n) + 32;
  unsigned char *out = malloc(cap);
  s.next_in = (Bytef *)x;
  s.avail_in = (uInt)n;
  s.next_out = out;
  s.avail_out = (uInt)cap;
  if (deflate(&s, Z_FINISH) != Z_STREAM_END) abort();
  *out_len = s.total_out;
  deflateEnd(&s);
  return out;
}

/* Decompress d[0..n) (wrapper per `bits`) into a malloc'd buffer, growing it
 * as needed. */
static unsigned char *z_decode(const unsigned char *d, size_t n, int bits, size_t *out_len) {
  z_stream s;
  memset(&s, 0, sizeof s);
  if (inflateInit2(&s, bits) != Z_OK) abort();
  size_t cap = n * 4 + 1024;
  unsigned char *out = malloc(cap);
  s.next_in = (Bytef *)d;
  s.avail_in = (uInt)n;
  for (;;) {
    s.next_out = out + s.total_out;
    s.avail_out = (uInt)(cap - s.total_out);
    int rc = inflate(&s, Z_NO_FLUSH);
    if (rc == Z_STREAM_END) break;
    if (rc != Z_OK && rc != Z_BUF_ERROR) abort();
    if (s.avail_out == 0) {
      cap *= 2;
      out = realloc(out, cap);
    } else if (rc == Z_BUF_ERROR) {
      abort(); /* truncated input */
    }
  }
  *out_len = s.total_out;
  inflateEnd(&s);
  return out;
}

/* Round-trip checksum term: decompressed length + its CRC-32. */
static long long round_trip(const unsigned char *y, size_t n) {
  return (long long)n + (long long)crc32(0L, y, (uInt)n);
}

/* ----- compress deflate: 4x raw DEFLATE (level 6) of the input ----------- */

static void test_compress_deflate(void) {
  size_t n;
  unsigned char *x = compress_input(COMPRESS_TARGET, &n);
  long long *t = alloc_times();
  long long checksum = 0;
  unsigned char *outs[DEFLATE_REPS];
  size_t lens[DEFLATE_REPS];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    for (int rep = 0; rep < DEFLATE_REPS; rep++) {
      bench_opaque(x);
      outs[rep] = z_encode(x, n, RAW_BITS, &lens[rep]);
    }
    t[r] = now_ns() - t0;
    long long acc = 0;
    for (int rep = 0; rep < DEFLATE_REPS; rep++) {
      size_t yl;
      unsigned char *y = z_decode(outs[rep], lens[rep], RAW_BITS, &yl);
      acc += round_trip(y, yl);
      free(y);
      free(outs[rep]);
    }
    checksum = acc;
  }
  fprintf(stderr, "test_compress_deflate = %lld\n", checksum);
  record("compress", "deflate", t, RUN);
  free(t);
  free(x);
}

/* ----- compress inflate: 16x raw inflate of the deflated input ----------- */

static void test_compress_inflate(void) {
  size_t n, dl;
  unsigned char *x = compress_input(COMPRESS_TARGET, &n);
  unsigned char *d = z_encode(x, n, RAW_BITS, &dl);
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    for (int rep = 0; rep < INFLATE_REPS; rep++) {
      bench_opaque(d);
      size_t yl;
      unsigned char *y = z_decode(d, dl, RAW_BITS, &yl);
      acc += round_trip(y, yl);
      free(y);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_compress_inflate = %lld\n", checksum);
  record("compress", "inflate", t, RUN);
  free(t);
  free(d);
  free(x);
}

/* ----- compress gzip: 4x gzip encode + decode round trip (level 6) ------- */

static void test_compress_gzip(void) {
  size_t n;
  unsigned char *x = compress_input(COMPRESS_TARGET, &n);
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    for (int rep = 0; rep < GZIP_REPS; rep++) {
      bench_opaque(x);
      size_t gl, yl;
      unsigned char *g = z_encode(x, n, GZIP_BITS, &gl);
      unsigned char *y = z_decode(g, gl, GZIP_BITS, &yl);
      acc += round_trip(y, yl);
      free(y);
      free(g);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_compress_gzip = %lld\n", checksum);
  record("compress", "gzip", t, RUN);
  free(t);
  free(x);
}

void run_compress_group(void) {
  test_compress_crc32();
  test_compress_deflate();
  test_compress_inflate();
  test_compress_gzip();
}
