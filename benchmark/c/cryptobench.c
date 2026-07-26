/* GROUP: crypto (crypto:: package coverage) — plan-65 Theme 1.
 *
 * Mirrors benchmark/mfb/src/crypto.mfb: SHA-256/512, HMAC-SHA-256, PBKDF2-HMAC-
 * SHA-256, constant-time compare, and a fresh-message hash churn. Like the
 * encoding group hand-rolls RFC 4648 base64/hex, this file hand-rolls the FIPS
 * 180-4 / RFC 2104 / RFC 8018 cores so the C column needs no vendored dependency;
 * each checksum folds the digest/tag bytes and matches the mfb `crypto::` software
 * core and Python hashlib bit-for-bit (the cross-language checksum is the proof of
 * correctness). The Ed25519 row has no in-suite C peer, so it records a `--`
 * placeholder to keep the three tables row-aligned. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "cryptobench.h"

/* ----- SHA-256 (FIPS 180-4) -------------------------------------------- */

static const uint32_t K256[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2};

static uint32_t ror32(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }

/* SHA-256 of `len` bytes into out[32]. */
static void sha256(const uint8_t *data, size_t len, uint8_t out[32]) {
  uint32_t h[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                   0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
  size_t total = len + 1 + 8;              /* data + 0x80 + 64-bit length */
  size_t padded = ((total + 63) / 64) * 64;
  uint8_t *msg = calloc(padded, 1);
  memcpy(msg, data, len);
  msg[len] = 0x80;
  uint64_t bits = (uint64_t)len * 8;
  for (int i = 0; i < 8; i++) msg[padded - 1 - i] = (uint8_t)(bits >> (8 * i));

  for (size_t off = 0; off < padded; off += 64) {
    uint32_t w[64];
    for (int i = 0; i < 16; i++)
      w[i] = ((uint32_t)msg[off + 4 * i] << 24) | ((uint32_t)msg[off + 4 * i + 1] << 16) |
             ((uint32_t)msg[off + 4 * i + 2] << 8) | (uint32_t)msg[off + 4 * i + 3];
    for (int i = 16; i < 64; i++) {
      uint32_t s0 = ror32(w[i - 15], 7) ^ ror32(w[i - 15], 18) ^ (w[i - 15] >> 3);
      uint32_t s1 = ror32(w[i - 2], 17) ^ ror32(w[i - 2], 19) ^ (w[i - 2] >> 10);
      w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
    for (int i = 0; i < 64; i++) {
      uint32_t S1 = ror32(e, 6) ^ ror32(e, 11) ^ ror32(e, 25);
      uint32_t ch = (e & f) ^ ((~e) & g);
      uint32_t t1 = hh + S1 + ch + K256[i] + w[i];
      uint32_t S0 = ror32(a, 2) ^ ror32(a, 13) ^ ror32(a, 22);
      uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
      uint32_t t2 = S0 + maj;
      hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
    }
    h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
  }
  free(msg);
  for (int i = 0; i < 8; i++) {
    out[4 * i] = (uint8_t)(h[i] >> 24);
    out[4 * i + 1] = (uint8_t)(h[i] >> 16);
    out[4 * i + 2] = (uint8_t)(h[i] >> 8);
    out[4 * i + 3] = (uint8_t)h[i];
  }
}

/* ----- SHA-512 (FIPS 180-4) -------------------------------------------- */

static const uint64_t K512[80] = {
    0x428a2f98d728ae22ULL, 0x7137449123ef65cdULL, 0xb5c0fbcfec4d3b2fULL, 0xe9b5dba58189dbbcULL,
    0x3956c25bf348b538ULL, 0x59f111f1b605d019ULL, 0x923f82a4af194f9bULL, 0xab1c5ed5da6d8118ULL,
    0xd807aa98a3030242ULL, 0x12835b0145706fbeULL, 0x243185be4ee4b28cULL, 0x550c7dc3d5ffb4e2ULL,
    0x72be5d74f27b896fULL, 0x80deb1fe3b1696b1ULL, 0x9bdc06a725c71235ULL, 0xc19bf174cf692694ULL,
    0xe49b69c19ef14ad2ULL, 0xefbe4786384f25e3ULL, 0x0fc19dc68b8cd5b5ULL, 0x240ca1cc77ac9c65ULL,
    0x2de92c6f592b0275ULL, 0x4a7484aa6ea6e483ULL, 0x5cb0a9dcbd41fbd4ULL, 0x76f988da831153b5ULL,
    0x983e5152ee66dfabULL, 0xa831c66d2db43210ULL, 0xb00327c898fb213fULL, 0xbf597fc7beef0ee4ULL,
    0xc6e00bf33da88fc2ULL, 0xd5a79147930aa725ULL, 0x06ca6351e003826fULL, 0x142929670a0e6e70ULL,
    0x27b70a8546d22ffcULL, 0x2e1b21385c26c926ULL, 0x4d2c6dfc5ac42aedULL, 0x53380d139d95b3dfULL,
    0x650a73548baf63deULL, 0x766a0abb3c77b2a8ULL, 0x81c2c92e47edaee6ULL, 0x92722c851482353bULL,
    0xa2bfe8a14cf10364ULL, 0xa81a664bbc423001ULL, 0xc24b8b70d0f89791ULL, 0xc76c51a30654be30ULL,
    0xd192e819d6ef5218ULL, 0xd69906245565a910ULL, 0xf40e35855771202aULL, 0x106aa07032bbd1b8ULL,
    0x19a4c116b8d2d0c8ULL, 0x1e376c085141ab53ULL, 0x2748774cdf8eeb99ULL, 0x34b0bcb5e19b48a8ULL,
    0x391c0cb3c5c95a63ULL, 0x4ed8aa4ae3418acbULL, 0x5b9cca4f7763e373ULL, 0x682e6ff3d6b2b8a3ULL,
    0x748f82ee5defb2fcULL, 0x78a5636f43172f60ULL, 0x84c87814a1f0ab72ULL, 0x8cc702081a6439ecULL,
    0x90befffa23631e28ULL, 0xa4506cebde82bde9ULL, 0xbef9a3f7b2c67915ULL, 0xc67178f2e372532bULL,
    0xca273eceea26619cULL, 0xd186b8c721c0c207ULL, 0xeada7dd6cde0eb1eULL, 0xf57d4f7fee6ed178ULL,
    0x06f067aa72176fbaULL, 0x0a637dc5a2c898a6ULL, 0x113f9804bef90daeULL, 0x1b710b35131c471bULL,
    0x28db77f523047d84ULL, 0x32caab7b40c72493ULL, 0x3c9ebe0a15c9bebcULL, 0x431d67c49c100d4cULL,
    0x4cc5d4becb3e42b6ULL, 0x597f299cfc657e2aULL, 0x5fcb6fab3ad6faecULL, 0x6c44198c4a475817ULL};

static uint64_t ror64(uint64_t x, int n) { return (x >> n) | (x << (64 - n)); }

/* SHA-512 of `len` bytes into out[64]. */
static void sha512(const uint8_t *data, size_t len, uint8_t out[64]) {
  uint64_t h[8] = {0x6a09e667f3bcc908ULL, 0xbb67ae8584caa73bULL, 0x3c6ef372fe94f82bULL,
                   0xa54ff53a5f1d36f1ULL, 0x510e527fade682d1ULL, 0x9b05688c2b3e6c1fULL,
                   0x1f83d9abfb41bd6bULL, 0x5be0cd19137e2179ULL};
  size_t total = len + 1 + 16;             /* data + 0x80 + 128-bit length */
  size_t padded = ((total + 127) / 128) * 128;
  uint8_t *msg = calloc(padded, 1);
  memcpy(msg, data, len);
  msg[len] = 0x80;
  uint64_t bits = (uint64_t)len * 8;       /* high 64 bits of the length are 0 here */
  for (int i = 0; i < 8; i++) msg[padded - 1 - i] = (uint8_t)(bits >> (8 * i));

  for (size_t off = 0; off < padded; off += 128) {
    uint64_t w[80];
    for (int i = 0; i < 16; i++) {
      w[i] = 0;
      for (int b = 0; b < 8; b++) w[i] = (w[i] << 8) | msg[off + 8 * i + b];
    }
    for (int i = 16; i < 80; i++) {
      uint64_t s0 = ror64(w[i - 15], 1) ^ ror64(w[i - 15], 8) ^ (w[i - 15] >> 7);
      uint64_t s1 = ror64(w[i - 2], 19) ^ ror64(w[i - 2], 61) ^ (w[i - 2] >> 6);
      w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    uint64_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
    for (int i = 0; i < 80; i++) {
      uint64_t S1 = ror64(e, 14) ^ ror64(e, 18) ^ ror64(e, 41);
      uint64_t ch = (e & f) ^ ((~e) & g);
      uint64_t t1 = hh + S1 + ch + K512[i] + w[i];
      uint64_t S0 = ror64(a, 28) ^ ror64(a, 34) ^ ror64(a, 39);
      uint64_t maj = (a & b) ^ (a & c) ^ (b & c);
      uint64_t t2 = S0 + maj;
      hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
    }
    h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
  }
  free(msg);
  for (int i = 0; i < 8; i++)
    for (int b = 0; b < 8; b++) out[8 * i + b] = (uint8_t)(h[i] >> (56 - 8 * b));
}

/* ----- HMAC-SHA-256 (RFC 2104) and PBKDF2-HMAC-SHA-256 (RFC 8018) ------- */

static void hmac_sha256(const uint8_t *key, size_t keylen, const uint8_t *msg,
                        size_t msglen, uint8_t out[32]) {
  uint8_t k[64] = {0};
  if (keylen > 64)
    sha256(key, keylen, k); /* long key hashed to 32 bytes, zero-padded to 64 */
  else
    memcpy(k, key, keylen);
  uint8_t ipad[64], opad[64];
  for (int i = 0; i < 64; i++) { ipad[i] = k[i] ^ 0x36; opad[i] = k[i] ^ 0x5c; }

  uint8_t inner[32];
  uint8_t *ibuf = malloc(64 + msglen);
  memcpy(ibuf, ipad, 64);
  memcpy(ibuf + 64, msg, msglen);
  sha256(ibuf, 64 + msglen, inner);
  free(ibuf);

  uint8_t obuf[96];
  memcpy(obuf, opad, 64);
  memcpy(obuf + 64, inner, 32);
  sha256(obuf, 96, out);
}

static void pbkdf2_sha256(const uint8_t *pw, size_t pwlen, const uint8_t *salt,
                          size_t saltlen, int iters, uint8_t *dk, size_t dklen) {
  uint32_t blocks = (uint32_t)((dklen + 31) / 32);
  uint8_t *saltb = malloc(saltlen + 4);
  memcpy(saltb, salt, saltlen);
  size_t done = 0;
  for (uint32_t i = 1; i <= blocks; i++) {
    saltb[saltlen] = (uint8_t)(i >> 24);
    saltb[saltlen + 1] = (uint8_t)(i >> 16);
    saltb[saltlen + 2] = (uint8_t)(i >> 8);
    saltb[saltlen + 3] = (uint8_t)i;
    uint8_t u[32], tblock[32];
    hmac_sha256(pw, pwlen, saltb, saltlen + 4, u);
    memcpy(tblock, u, 32);
    for (int it = 1; it < iters; it++) {
      hmac_sha256(pw, pwlen, u, 32, u);
      for (int b = 0; b < 32; b++) tblock[b] ^= u[b];
    }
    size_t take = dklen - done < 32 ? dklen - done : 32;
    memcpy(dk + done, tblock, take);
    done += take;
  }
  free(saltb);
}

/* ----- workloads -------------------------------------------------------- */

static void make_buf(uint8_t *buf, int n) {
  for (int i = 0; i < n; i++) buf[i] = (uint8_t)((i * 37 + 11) % 256);
}

static long sum_bytes(const uint8_t *d, int n) {
  long s = 0;
  for (int i = 0; i < n; i++) s += d[i];
  return s;
}

static void test_crypto_sha256(void) {
  uint8_t buf[1024], out[32];
  make_buf(buf, 1024);
  int reps = 64;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) { sha256(buf, 1024, out); acc += sum_bytes(out, 32); }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "crypto_sha256 = %ld\n", checksum);
  record("crypto", "sha256", t, RUN);
  free(t);
}

/* Arena-gated in mfb (plan-64-A: SHA-512's >2KB transients hit the O(n^2) free-
 * list walk); the C mirror keeps the same tiny reps so the table lines up. */
static void test_crypto_sha512(void) {
  uint8_t buf[1024], out[64];
  make_buf(buf, 1024);
  int reps = 2; /* TODO(plan-64-A): raise to 64 */
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) { sha512(buf, 1024, out); acc += sum_bytes(out, 64); }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "crypto_sha512 = %ld\n", checksum);
  record("crypto", "sha512", t, RUN);
  free(t);
}

/* Arena-gated in mfb (plan-64-A: the per-MAC transient volume drives a cumulative
 * quick-bin climb); the C mirror keeps the same tiny reps so the table lines up. */
static void test_crypto_hmac(void) {
  uint8_t buf[1024], key[32], out[32];
  make_buf(buf, 1024);
  make_buf(key, 32);
  int reps = 8; /* TODO(plan-64-A): raise to 64 */
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) { hmac_sha256(key, 32, buf, 1024, out); acc += sum_bytes(out, 32); }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "crypto_hmac = %ld\n", checksum);
  record("crypto", "hmac", t, RUN);
  free(t);
}

/* Arena-gated in mfb (plan-64-A: a 4096-iteration derive is ~8192 SHA-256 ops
 * whose transient churn triggers the flush amplifier); the C mirror keeps the same
 * low work factor so the table lines up. */
static void test_crypto_pbkdf2(void) {
  uint8_t pw[16], salt[16], dk[32];
  make_buf(pw, 16);
  make_buf(salt, 16);
  int iters = 64; /* TODO(plan-64-A): raise to 4096 */
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    pbkdf2_sha256(pw, 16, salt, 16, iters, dk, 32);
    checksum = sum_bytes(dk, 32);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "crypto_pbkdf2 = %ld\n", checksum);
  record("crypto", "pbkdf2", t, RUN);
  free(t);
}

/* Constant-time byte compare: OR-fold every position's XOR, never early-return. */
static int ct_equal(const uint8_t *a, const uint8_t *b, int n) {
  uint8_t diff = 0;
  for (int i = 0; i < n; i++) diff |= (uint8_t)(a[i] ^ b[i]);
  return diff == 0;
}

static void test_crypto_cte(void) {
  uint8_t a[32], b[32];
  make_buf(a, 32);
  make_buf(b, 32);
  int reps = 8192;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long cnt = 0;
    for (int rep = 0; rep < reps; rep++)
      if (ct_equal(a, b, 32)) cnt++;
    checksum = cnt;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "crypto_cte = %ld\n", checksum);
  record("crypto", "cte", t, RUN);
  free(t);
}

/* Hash-churn: hash many fresh 40-byte messages. Arena-gated in mfb (plan-64-A);
 * the C mirror keeps the same tiny count so the table lines up. */
static void test_crypto_churn(void) {
  int msgs = 16; /* TODO(plan-64-A): raise to 4096 */
  uint8_t out[32];
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < msgs; i++) {
      uint8_t m[40];
      for (int j = 0; j < 40; j++) m[j] = (uint8_t)((i * 13 + j * 37 + 11) % 256);
      sha256(m, 40, out);
      acc += sum_bytes(out, 32);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "crypto_churn = %ld\n", checksum);
  record("crypto", "churn", t, RUN);
  free(t);
}

void run_crypto_group(void) {
  test_crypto_sha256();
  test_crypto_sha512();
  test_crypto_hmac();
  test_crypto_pbkdf2();
  test_crypto_cte();
  test_crypto_churn();
  /* ed25519 is mfb+python only (deterministic RFC-8032 sign+verify); C has no
   * in-suite peer, so record a `--` placeholder to keep the tables row-aligned. */
  record("crypto", "ed25519", NULL, 0);
}
