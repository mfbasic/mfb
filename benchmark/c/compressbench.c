/* GROUP: compress — the C oracle for compress.mfb (CRC-32 over the compress::
 * package). The peer calls system zlib's crc32() (linked with -lz), C's
 * standard means of doing the job; the CRC is a fixed function of the bytes,
 * so the checksum matches mfb and Python bit-for-bit.
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

void run_compress_group(void) { test_compress_crc32(); }
