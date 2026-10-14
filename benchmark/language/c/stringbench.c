/* GROUP: string (strings:: package + & concat)
 *
 * Moved into its own translation unit; shared timing/recording infra comes
 * from bench.h. Contains the historical `concat` row plus consolidated
 * coverage rows mirroring string.mfb: case (case/trim/normalize), search
 * (tests/search), slice (slice/reshape) and unicode (grapheme/byte views).
 *
 * The case/search/slice workloads use pure-ASCII strings, where mfb len()
 * (Unicode scalar count) equals the byte length, so the len()-based checksum
 * arithmetic matches plain C strlen math exactly. The unicode row's grapheme /
 * scalar counts are an APPROXIMATION of mfb's Unicode-table grapheme semantics
 * (see note at test_string_unicode); its checksum is stable but not required
 * to match mfb bit-for-bit. */
#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "stringbench.h"

/* ----- historical concat row ------------------------------------------- */

static void test_string_concat(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    char *s = NULL; int len = 0, cap = 0;
    for (int i = 0; i < 1000; i++) {
      if (len + 1 >= cap) { cap = cap ? cap * 2 : 2; s = realloc(s, cap); }
      s[len++] = 'x';
    }
    s[len] = '\0';
    checksum = len;
    t[r] = now_ns() - t0;
    free(s);
  }
  fprintf(stderr, "string_concat = %ld\n", checksum);
  record("string", "concat", t, RUN);
  free(t);
}

/* ----- small ASCII string helpers (all return malloc'd strings) --------- */

static char *dup_n(const char *s, size_t n) {
  char *r = malloc(n + 1); memcpy(r, s, n); r[n] = '\0'; return r;
}
static char *str_case(const char *s, int lower) {
  size_t n = strlen(s); char *r = malloc(n + 1);
  for (size_t i = 0; i < n; i++)
    r[i] = (char)(lower ? tolower((unsigned char)s[i]) : toupper((unsigned char)s[i]));
  r[n] = '\0'; return r;
}
static char *str_trim(const char *s, int left, int right) {
  const char *a = s; const char *b = s + strlen(s);
  if (left) while (*a && isspace((unsigned char)*a)) a++;
  if (right) while (b > a && isspace((unsigned char)b[-1])) b--;
  return dup_n(a, (size_t)(b - a));
}
static int in_set(char c, const char *set) { return c != '\0' && strchr(set, c) != NULL; }
static char *str_trim_chars(const char *s, const char *set) {
  const char *a = s; const char *b = s + strlen(s);
  while (*a && in_set(*a, set)) a++;
  while (b > a && in_set(b[-1], set)) b--;
  return dup_n(a, (size_t)(b - a));
}
static int str_count(const char *s, const char *sub) {
  int n = 0; size_t l = strlen(sub); const char *p = s;
  if (l == 0) return 0;
  while ((p = strstr(p, sub))) { n++; p += l; }
  return n;
}
static int starts_with(const char *s, const char *pre) {
  return strncmp(s, pre, strlen(pre)) == 0;
}
static int ends_with(const char *s, const char *suf) {
  size_t ls = strlen(s), lf = strlen(suf);
  return ls >= lf && memcmp(s + ls - lf, suf, lf) == 0;
}
static char *strip_prefix(const char *s, const char *pre) {
  if (starts_with(s, pre)) return dup_n(s + strlen(pre), strlen(s) - strlen(pre));
  return dup_n(s, strlen(s));
}
static char *strip_suffix(const char *s, const char *suf) {
  if (ends_with(s, suf)) return dup_n(s, strlen(s) - strlen(suf));
  return dup_n(s, strlen(s));
}
static char **str_split(const char *s, const char *sep, int *out_n) {
  size_t sl = strlen(sep); int cap = 8, n = 0;
  char **parts = malloc((size_t)cap * sizeof(char *));
  const char *p = s;
  while (1) {
    const char *q = sl ? strstr(p, sep) : NULL;
    size_t len = q ? (size_t)(q - p) : strlen(p);
    if (n == cap) { cap *= 2; parts = realloc(parts, (size_t)cap * sizeof(char *)); }
    parts[n++] = dup_n(p, len);
    if (!q) break;
    p = q + sl;
  }
  *out_n = n; return parts;
}
static char *str_join(char **parts, int n, const char *sep) {
  size_t sl = strlen(sep), total = 0;
  for (int i = 0; i < n; i++) total += strlen(parts[i]);
  if (n > 0) total += sl * (size_t)(n - 1);
  char *r = malloc(total + 1); size_t pos = 0;
  for (int i = 0; i < n; i++) {
    if (i) { memcpy(r + pos, sep, sl); pos += sl; }
    size_t pl = strlen(parts[i]); memcpy(r + pos, parts[i], pl); pos += pl;
  }
  r[pos] = '\0'; return r;
}
static char *str_replace(const char *s, const char *from, const char *to) {
  size_t fl = strlen(from), tl = strlen(to), n = strlen(s);
  int cnt = str_count(s, from);
  char *r = malloc(n + (size_t)cnt * tl + 1);
  char *w = r; const char *q = s;
  while (1) {
    const char *m = fl ? strstr(q, from) : NULL;
    if (!m) { strcpy(w, q); break; }
    memcpy(w, q, (size_t)(m - q)); w += m - q;
    memcpy(w, to, tl); w += tl;
    q = m + fl;
  }
  return r;
}
static char *str_repeat(const char *s, int times) {
  size_t l = strlen(s); char *r = malloc(l * (size_t)times + 1);
  for (int i = 0; i < times; i++) memcpy(r + (size_t)i * l, s, l);
  r[l * (size_t)times] = '\0'; return r;
}
static char *str_pad_left(const char *s, int width) {
  size_t l = strlen(s); if ((int)l >= width) return dup_n(s, l);
  char *r = malloc((size_t)width + 1); int pad = width - (int)l;
  memset(r, ' ', (size_t)pad); memcpy(r + pad, s, l); r[width] = '\0'; return r;
}
static char *str_pad_right(const char *s, int width, char pc) {
  size_t l = strlen(s); if ((int)l >= width) return dup_n(s, l);
  char *r = malloc((size_t)width + 1); memcpy(r, s, l);
  memset(r + l, pc, (size_t)(width - (int)l)); r[width] = '\0'; return r;
}
static char *str_left(const char *s, int n) {
  size_t l = strlen(s); size_t m = (size_t)n < l ? (size_t)n : l; return dup_n(s, m);
}
static char *str_right(const char *s, int n) {
  size_t l = strlen(s); size_t m = (size_t)n < l ? (size_t)n : l; return dup_n(s + l - m, m);
}
static char *str_mid(const char *s, int start, int length) {
  size_t l = strlen(s); int idx = start; /* zero-based, matches strings::mid */
  if (idx < 0) idx = 0; if ((size_t)idx > l) idx = (int)l;
  size_t avail = l - (size_t)idx;
  size_t take = (size_t)length < avail ? (size_t)length : avail;
  return dup_n(s + idx, take);
}

/* ----- coverage: case (case mapping, trimming, normalization) ----------- */

static void test_string_case(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char s[48];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 50000; i++) {
      snprintf(s, sizeof s, "  Hello World %d  ", i);
      char *a;
      a = str_case(s, 0); acc += (long)strlen(a); free(a);       /* upper */
      a = str_case(s, 1); acc += (long)strlen(a); free(a);       /* lower */
      a = str_case(s, 1); acc += (long)strlen(a); free(a);       /* caseFold */
      a = str_trim(s, 1, 1); acc += (long)strlen(a); free(a);    /* trim */
      a = str_trim(s, 1, 0); acc += (long)strlen(a); free(a);    /* trimStart */
      a = str_trim(s, 0, 1); acc += (long)strlen(a); free(a);    /* trimEnd */
      a = str_trim_chars(s, " Helo"); acc += (long)strlen(a); free(a);
      a = dup_n(s, strlen(s)); acc += (long)strlen(a); free(a);  /* normalizeNfc (ASCII no-op) */
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "string_case = %ld\n", checksum);
  record("string", "case", t, RUN);
  free(t);
}

/* ----- coverage: search (tests + search) -------------------------------- */

static void test_string_search(void) {
  const char *prefixes[2] = {"He", "Wo"};
  const char *suffixes[2] = {"ld", "xx"};
  long long *t = alloc_times();
  long checksum = 0;
  char s[32], suf[32];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 50000; i++) {
      snprintf(s, sizeof s, "Hello World %d", i);
      if (strstr(s, "World")) acc += 1;
      acc += str_count(s, "l");
      if (starts_with(s, "Hello")) acc += 1;
      snprintf(suf, sizeof suf, "World %d", i);
      if (ends_with(s, suf)) acc += 1;
      int swa = 0;
      for (int k = 0; k < 2; k++) if (starts_with(s, prefixes[k])) { swa = 1; break; }
      if (swa) acc += 1;
      int ewa = 0;
      for (int k = 0; k < 2; k++) if (ends_with(s, suffixes[k])) { ewa = 1; break; }
      if (ewa) acc += 1;
      const char *f = strstr(s, "World");
      acc += f ? (long)(f - s) : -1;
      char *sp;
      sp = strip_prefix(s, "Hello "); acc += (long)strlen(sp); free(sp);
      sp = strip_suffix(s, "!"); acc += (long)strlen(sp); free(sp);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "string_search = %ld\n", checksum);
  record("string", "search", t, RUN);
  free(t);
}

/* ----- coverage: slice (slicing + reshaping) ---------------------------- */

static void test_string_slice(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char s[32];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 50000; i++) {
      snprintf(s, sizeof s, "Hello World %d", i);
      char *a;
      a = str_left(s, 5); acc += (long)strlen(a); free(a);
      a = str_right(s, 3); acc += (long)strlen(a); free(a);
      a = str_mid(s, 2, 4); acc += (long)strlen(a); free(a);
      int nwords; char **words = str_split(s, " ", &nwords);
      acc += nwords;
      a = str_join(words, nwords, "-"); acc += (long)strlen(a); free(a);
      for (int k = 0; k < nwords; k++) free(words[k]);
      free(words);
      a = str_replace(s, "l", "L"); acc += (long)strlen(a); free(a);
      a = str_repeat("ab", 3); acc += (long)strlen(a); free(a);
      a = str_pad_left(s, 24); acc += (long)strlen(a); free(a);
      a = str_pad_right(s, 24, '.'); acc += (long)strlen(a); free(a);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "string_slice = %ld\n", checksum);
  record("string", "slice", t, RUN);
  free(t);
}

/* ----- coverage: unicode (grapheme segmentation + byte views) ----------- *
 *
 * NOTE: the grapheme and scalar counts below are an APPROXIMATION of mfb's
 * Unicode-table grapheme semantics. A grapheme cluster here is a base scalar
 * plus any following combining marks (U+0300..U+036F); everything else is its
 * own cluster. normalizeNfc's length is approximated by the scalar count
 * (true NFC would compose base+combining pairs into fewer scalars). The
 * checksum is therefore stable but NOT expected to match mfb bit-for-bit. */

static int utf8_next(const unsigned char *p, uint32_t *cp) {
  if (p[0] < 0x80) { *cp = p[0]; return 1; }
  if ((p[0] & 0xE0) == 0xC0) { *cp = ((p[0] & 0x1F) << 6) | (p[1] & 0x3F); return 2; }
  if ((p[0] & 0xF0) == 0xE0) {
    *cp = ((uint32_t)(p[0] & 0x0F) << 12) | ((p[1] & 0x3F) << 6) | (p[2] & 0x3F);
    return 3;
  }
  *cp = ((uint32_t)(p[0] & 0x07) << 18) | ((uint32_t)(p[1] & 0x3F) << 12) |
        ((p[2] & 0x3F) << 6) | (p[3] & 0x3F);
  return 4;
}
static int is_combining(uint32_t cp) { return cp >= 0x300 && cp <= 0x36F; }

static int scalar_count(const char *s) {
  const unsigned char *p = (const unsigned char *)s; int n = 0; uint32_t cp;
  while (*p) { p += utf8_next(p, &cp); n++; }
  return n;
}
static int grapheme_count(const char *s) {
  const unsigned char *p = (const unsigned char *)s; int n = 0, open = 0; uint32_t cp;
  while (*p) {
    p += utf8_next(p, &cp);
    if (is_combining(cp) && open) continue; /* attach to current cluster */
    n++; open = 1;
  }
  return n;
}

static void test_string_unicode(void) {
  /* mfb: "cafe\u{0301} rocket\u{1F680} nai\u{0308}ve schoen" in NFD.
   * Built here from explicit UTF-8 bytes (literal concatenation keeps each
   * \x escape unambiguous):
   *   "cafe" + U+0301(CC 81) + " rocket" + U+1F680(F0 9F 9A 80)
   *         + " nai" + U+0308(CC 88) + "ve schoen" */
  const char *u = "cafe" "\xcc\x81" " rocket" "\xf0\x9f\x9a\x80"
                  " nai" "\xcc\x88" "ve schoen";
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    /* Inner count matches the mfb coverage row (10). The mfb grapheme surface
     * triggers an arena mixed-transient-churn slowdown at high counts, so that
     * row is a small coverage smoke-test, not a throughput benchmark; keep the
     * C/Python rows at the same tiny count for a like-for-like table. */
    for (int i = 0; i < 10; i++) {
      acc += grapheme_count(u);            /* len(graphemes(u)) */
      acc += grapheme_count(u);            /* graphemesCount(u) */
      acc += 1;                            /* len(graphemeAt(u,0)) = "c" */
      acc += (long)strlen(u);              /* byteLen(u) */
      acc += (long)strlen(u);              /* len(toBytes(u)) */
      acc += scalar_count(u);              /* len(normalizeNfc(u)) approx */
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "string_unicode = %ld\n", checksum);
  record("string", "unicode", t, RUN);
  free(t);
}

void run_string_group(void) {
  test_string_concat();
  test_string_case();
  test_string_search();
  test_string_slice();
  test_string_unicode();
}
