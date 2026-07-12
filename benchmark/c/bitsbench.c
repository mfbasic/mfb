/* GROUP: bits (bits:: package coverage)
 *
 * Moved into its own translation unit; shared timing/recording infra comes
 * from bench.h. Mirrors bits.mfb: every bits:: member operates on raw 64-bit
 * Integer bit patterns, so a single row drives band/bor/bxor/bnot, sl/sr/sra,
 * rl32/rr32/rl64/rr64, clz/ctz/popCount and bswap16/32/64. The accumulator is
 * folded with XOR (not +) so a full-width bit pattern never trips overflow.
 *
 * All arithmetic is done on uint64_t and reported as a signed 64-bit Integer,
 * matching mfb's i64 Integer. clz(0)/ctz(0) are special-cased to 64 (mfb's
 * defined result); the 32-bit ops act on the low 32 bits and zero-extend. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "bitsbench.h"

static inline uint64_t rotl64(uint64_t v, unsigned s) {
  s &= 63u;
  return s ? (v << s) | (v >> (64 - s)) : v;
}
static inline uint64_t rotr64(uint64_t v, unsigned s) {
  s &= 63u;
  return s ? (v >> s) | (v << (64 - s)) : v;
}
static inline uint32_t rotl32(uint32_t v, unsigned s) {
  s &= 31u;
  return s ? (v << s) | (v >> (32 - s)) : v;
}
static inline uint32_t rotr32(uint32_t v, unsigned s) {
  s &= 31u;
  return s ? (v >> s) | (v << (32 - s)) : v;
}
static inline int clz64(uint64_t v) { return v ? __builtin_clzll(v) : 64; }
static inline int ctz64(uint64_t v) { return v ? __builtin_ctzll(v) : 64; }

static void test_bits_ops(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    uint64_t acc = 0;
    uint64_t h = 2166136261ULL;
    for (int i = 0; i < 200000; i++) {
      uint64_t x = (uint64_t)i;
      h ^= (x | (x << 3));                 /* bxor(h, bor(x, sl(x,3))) */
      h &= 1152921504606846975ULL;         /* band, 2^60 - 1          */
      h = rotl64(h, 7);                     /* rl64(h,7)               */
      h = rotr64(h, 3);                     /* rr64(h,3)               */
      h ^= (h >> 11);                       /* sr  (logical)           */
      h ^= (uint64_t)((int64_t)h >> 2);     /* sra (arithmetic)        */
      h ^= ~x;                              /* bnot(x)                 */
      acc ^= h;
      acc ^= (uint64_t)rotl32((uint32_t)h, 5);
      acc ^= (uint64_t)rotr32((uint32_t)h, 9);
      acc ^= (uint64_t)__builtin_bswap16((uint16_t)x);
      acc ^= (uint64_t)__builtin_bswap32((uint32_t)x);
      acc ^= __builtin_bswap64(h);
      acc ^= (uint64_t)__builtin_popcountll(h);
      acc ^= (uint64_t)clz64(x);
      acc ^= (uint64_t)ctz64(x);
    }
    checksum = (long long)acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "bits_ops = %lld\n", checksum);
  record("bits", "ops", t, RUN);
  free(t);
}

void run_bits_group(void) {
  test_bits_ops();
}
