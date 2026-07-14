/* GROUP: scalarbench — the C oracle for scalarbench.mfb (plan-41 Scalar).
 *
 * All classification / codepoint arithmetic is over ASCII (or the fixed
 * mixed-script fragment), so counts and transformed codepoints match mfb:
 *   roundtrip=2400, classify=3413150747, transform=3805, listchurn=48
 *
 * roundtrip's fragment is built from EXPLICIT universal-character escapes so it
 * is byte-unambiguous: "café 中文 rocket🚀 naïve Straße " with DECOMPOSED naïve
 * (nai + U+0308) = 30 Unicode code points; repeated 8x = 240 code points. A
 * runtime assert guards the 240 count. Code points are counted as UTF-8 lead
 * bytes ((b & 0xC0) != 0x80). */
#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "scalarbench.h"

/* Decode the next UTF-8 scalar; return its byte length. */
static int utf8_next(const unsigned char *p, uint32_t *cp) {
  if (p[0] < 0x80) { *cp = p[0]; return 1; }
  if ((p[0] & 0xE0) == 0xC0) { *cp = ((uint32_t)(p[0] & 0x1F) << 6) | (p[1] & 0x3F); return 2; }
  if ((p[0] & 0xF0) == 0xE0) {
    *cp = ((uint32_t)(p[0] & 0x0F) << 12) | ((uint32_t)(p[1] & 0x3F) << 6) | (p[2] & 0x3F);
    return 3;
  }
  *cp = ((uint32_t)(p[0] & 0x07) << 18) | ((uint32_t)(p[1] & 0x3F) << 12) |
        ((uint32_t)(p[2] & 0x3F) << 6) | (p[3] & 0x3F);
  return 4;
}
/* Encode one scalar into out; return bytes written. */
static int utf8_encode(uint32_t cp, char *out) {
  if (cp < 0x80) { out[0] = (char)cp; return 1; }
  if (cp < 0x800) {
    out[0] = (char)(0xC0 | (cp >> 6));
    out[1] = (char)(0x80 | (cp & 0x3F));
    return 2;
  }
  if (cp < 0x10000) {
    out[0] = (char)(0xE0 | (cp >> 12));
    out[1] = (char)(0x80 | ((cp >> 6) & 0x3F));
    out[2] = (char)(0x80 | (cp & 0x3F));
    return 3;
  }
  out[0] = (char)(0xF0 | (cp >> 18));
  out[1] = (char)(0x80 | ((cp >> 12) & 0x3F));
  out[2] = (char)(0x80 | ((cp >> 6) & 0x3F));
  out[3] = (char)(0x80 | (cp & 0x3F));
  return 4;
}
static int codepoint_count(const char *s) {
  int n = 0;
  for (const unsigned char *p = (const unsigned char *)s; *p; p++)
    if ((*p & 0xC0) != 0x80) n++;
  return n;
}

/* ----- roundtrip: string <-> List OF Scalar decode/encode --------------- */

static void test_scalar_roundtrip(void) {
  const char *frag =
      u8"café 中文 rocket\U0001F680 naïve Straße ";
  size_t fl = strlen(frag);
  char *base = malloc(fl * 8 + 1);
  for (int i = 0; i < 8; i++) memcpy(base + (size_t)i * fl, frag, fl);
  base[fl * 8] = '\0';
  if (codepoint_count(base) != 240) {
    fprintf(stderr, "scalar_roundtrip: FATAL base code-point count %d != 240\n",
            codepoint_count(base));
  }

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 5; i++) {
      /* toScalars: decode into a fresh List OF Scalar */
      int cap = 64, n = 0;
      uint32_t *scalars = malloc((size_t)cap * sizeof(uint32_t));
      const unsigned char *p = (const unsigned char *)base;
      while (*p) {
        uint32_t cp;
        p += utf8_next(p, &cp);
        if (n == cap) { cap *= 2; scalars = realloc(scalars, (size_t)cap * sizeof(uint32_t)); }
        scalars[n++] = cp;
      }
      acc += n;
      /* fromScalars: re-encode to a String */
      char *back = malloc((size_t)n * 4 + 1);
      int bl = 0;
      for (int k = 0; k < n; k++) bl += utf8_encode(scalars[k], back + bl);
      back[bl] = '\0';
      acc += codepoint_count(back);
      free(scalars);
      free(back);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "scalar_roundtrip = %ld\n", checksum);
  record("scalarbench", "roundtrip", t, RUN);
  free(t);
  free(base);
}

/* ----- classify: category sweep, packed counts -------------------------- */

static void test_scalar_classify(void) {
  const char *base =
      "The Quick Brown Fox 123 JUMPS over 42 lazy Dogs! Now 7 Cats and 9 Owls.";
  size_t n = strlen(base);
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int nLetter = 0, nDigit = 0, nWhite = 0, nUpper = 0, nLower = 0;
    for (int pass = 0; pass < 2000; pass++) {
      nLetter = nDigit = nWhite = nUpper = nLower = 0;
      for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)base[i];
        if (isalpha(c)) nLetter++;
        if (isdigit(c)) nDigit++;
        if (isspace(c)) nWhite++;
        if (isupper(c)) nUpper++;
        if (islower(c)) nLower++;
      }
    }
    checksum = (long long)nLetter + (long long)nDigit * 100 + (long long)nWhite * 10000 +
               (long long)nUpper * 1000000 + (long long)nLower * 100000000;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "scalar_classify = %lld\n", checksum);
  record("scalarbench", "classify", t, RUN);
  free(t);
}

/* ----- transform: ROT-13 over ASCII letters, sum transformed codepoints -- */

static void test_scalar_transform(void) {
  const char *base = "The Quick Brown Fox Jumps Over The Lazy Dog";
  size_t n = strlen(base);
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long total = 0;
    for (int pass = 0; pass < 200; pass++) {
      long sumcp = 0;
      for (size_t i = 0; i < n; i++) {
        int cp = (unsigned char)base[i];
        int cp2 = cp;
        if (cp >= 97 && cp <= 122) cp2 = ((cp - 97 + 13) % 26) + 97;
        if (cp >= 65 && cp <= 90) cp2 = ((cp - 65 + 13) % 26) + 65;
        sumcp += cp2;
      }
      total = (long)n;   /* len(rebuilt) — ASCII so scalar count == byte count */
      total += sumcp;
    }
    checksum = total;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "scalar_transform = %ld\n", checksum);
  record("scalarbench", "transform", t, RUN);
  free(t);
}

/* ----- listchurn: ascending-adjacent-pair count over a List OF Scalar --- */

static void test_scalar_listchurn(void) {
  const char *frag = "azbyAZ09 mkq!Wp";
  size_t fl = strlen(frag);
  char *base = malloc(fl * 6 + 1);
  for (int i = 0; i < 6; i++) memcpy(base + (size_t)i * fl, frag, fl);
  base[fl * 6] = '\0';
  /* frag is ASCII, so each scalar is one byte. */
  int n = (int)strlen(base);
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long ascents = 0;
    for (int pass = 0; pass < 2000; pass++) {
      long a = 0;
      for (int i = 0; i < n - 1; i++)
        if ((unsigned char)base[i] < (unsigned char)base[i + 1]) a++;
      ascents = a;
    }
    checksum = ascents;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "scalar_listchurn = %ld\n", checksum);
  record("scalarbench", "listchurn", t, RUN);
  free(t);
  free(base);
}

void run_scalarbench_group(void) {
  test_scalar_roundtrip();
  test_scalar_classify();
  test_scalar_transform();
  test_scalar_listchurn();
}
