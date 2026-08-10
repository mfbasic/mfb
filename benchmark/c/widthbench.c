/* GROUP: width — the C oracle for width.mfb (plan-87 Theme 1, the first
 * benchmark of strings::displayWidth).
 *
 * displayWidth segments a string into UAX #29 extended grapheme clusters and
 * sums each cluster's terminal column width (0/1/2). Over the *controlled*
 * corpus these benchmarks use, that total is reproduced here by a small
 * per-scalar width table plus a one-scalar lookahead:
 *   - combining marks (U+0300..U+036F) and ZWJ (U+200D) are zero width;
 *   - the scalar immediately after a ZWJ is a sequence continuation (0 width);
 *   - East Asian Wide / emoji-presentation scalars are 2 columns, else 1.
 * dw_scan applies exactly that, yielding both the column total and the cluster
 * (grapheme) count. Verified per component against mfb: "abc"=3cols/3clusters,
 * U+65E5 U+672C U+8A9E (CJK)=6/3, "e"+U+0301=1/1, and the U+1F468 ZWJ U+1F469
 * ZWJ U+1F467 family=2/1.
 *
 * The corpus is written with all-ASCII source using octal byte escapes (\ooo),
 * so the exact UTF-8 bytes are unambiguous with no reliance on the source
 * encoding, no illegal sub-U+00A0 UCN, and no \x hex-greedy hazard.
 *
 * Expected checksums: ascii=8250000, mixed=480320, churn=360. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "widthbench.h"

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

static int is_zero_width(uint32_t cp) {
  return cp == 0x200D || (cp >= 0x0300 && cp <= 0x036F);
}

static int is_wide(uint32_t cp) {
  return (cp >= 0x4E00 && cp <= 0x9FFF) ||   /* CJK unified ideographs */
         (cp >= 0x1F300 && cp <= 0x1FAFF) || /* emoji / pictographs */
         (cp >= 0x2600 && cp <= 0x27BF);     /* misc symbols / dingbats */
}

/* Walk the UTF-8 string; accumulate the display-column total and the grapheme
 * (cluster) count using the same rules mfb's displayWidth/graphemesCount apply
 * to this corpus. */
static void dw_scan(const char *s, long *cols, long *clusters) {
  long c = 0, g = 0;
  int prev_zwj = 0;
  const unsigned char *p = (const unsigned char *)s;
  while (*p) {
    uint32_t cp;
    p += utf8_next(p, &cp);
    if (is_zero_width(cp)) { prev_zwj = (cp == 0x200D); continue; }
    if (prev_zwj) { prev_zwj = 0; continue; } /* ZWJ-sequence continuation */
    g++;
    c += is_wide(cp) ? 2 : 1;
    prev_zwj = 0;
  }
  *cols = c;
  *clusters = g;
}

static long display_width(const char *s) {
  long cols, clusters;
  dw_scan(s, &cols, &clusters);
  return cols;
}

/* ----- width ascii: all-narrow fast path over a fixed ~4 KB ASCII buffer -- */

static void test_width_ascii(void) {
  const char *frag = "The quick brown fox jumps over the lazy dog 0123456789 ";
  size_t fl = strlen(frag);
  char *base = malloc(fl * 75 + 1);
  for (int i = 0; i < 75; i++) memcpy(base + (size_t)i * fl, frag, fl);
  base[fl * 75] = '\0';

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long cols = 0;
    for (int rep = 0; rep < 2000; rep++) cols += display_width(base);
    checksum = cols;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "width_ascii = %ld\n", checksum);
  record("width", "ascii", t, RUN);
  free(t);
  free(base);
}

/* ----- width mixed: wide / zero-width / emoji / real EGC slow path -------- */

static void test_width_mixed(void) {
  /* abc | U+65E5 U+672C U+8A9E (日本語) | e + U+0301 (combining acute) |
   * U+1F468 ZWJ U+1F469 ZWJ U+1F467 (man ZWJ woman ZWJ girl). */
  const char *frag =
      "abc"
      "\346\227\245\346\234\254\350\252\236" /* U+65E5 U+672C U+8A9E */
      "e\314\201"                            /* e + U+0301 */
      "\360\237\221\250"                     /* U+1F468 */
      "\342\200\215"                         /* ZWJ U+200D */
      "\360\237\221\251"                     /* U+1F469 */
      "\342\200\215"                         /* ZWJ U+200D */
      "\360\237\221\247";                    /* U+1F467 */
  size_t fl = strlen(frag);
  char *base = malloc(fl * 40 + 1);
  for (int i = 0; i < 40; i++) memcpy(base + (size_t)i * fl, frag, fl);
  base[fl * 40] = '\0';

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long cols = 0;
    for (int rep = 0; rep < 1000; rep++) cols += display_width(base);
    long dummy, clusters;
    dw_scan(base, &dummy, &clusters);
    checksum = cols + clusters;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "width_mixed = %ld\n", checksum);
  record("width", "mixed", t, RUN);
  free(t);
  free(base);
}

/* ----- width churn: displayWidth over many fresh short strings ------------ */

static void test_width_churn(void) {
  const char *cjk = "\346\227\245\346\234\254"; /* U+65E5 U+672C — two wide ideographs */
  long long *t = alloc_times();
  long checksum = 0;
  char buf[64];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int pass = 0; pass < 5; pass++) {
      for (int i = 0; i < 8; i++) {
        snprintf(buf, sizeof buf, "row%d %s", i, cjk);
        acc += display_width(buf);
      }
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "width_churn = %ld\n", checksum);
  record("width", "churn", t, RUN);
  free(t);
}

void run_width_group(void) {
  test_width_ascii();
  test_width_mixed();
  test_width_churn();
}
