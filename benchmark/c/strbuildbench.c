/* GROUP: strbuild — the C oracle for strbuild.mfb (accumulate / tokenize /
 * clean hot paths) plus test_string_unibig (string group).
 *
 * Strings are actually materialized (realloc-grown concat, join, split, and a
 * replace/trim/strip/pad clean pipeline) so timing is fair. Checksums match mfb:
 *   concat=18890, join=18890, splitjoin=1778000, clean=240000
 * string_unibig mirrors the mfb workload with an approximate (stable) checksum. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "strbuildbench.h"

/* ----- small malloc'd string helpers ----------------------------------- */

static char *dup_n(const char *s, size_t n) {
  char *r = malloc(n + 1);
  memcpy(r, s, n);
  r[n] = '\0';
  return r;
}
static int in_set(char c, const char *set) { return c != '\0' && strchr(set, c) != NULL; }
static char *str_trim_chars(const char *s, const char *set) {
  const char *a = s;
  const char *b = s + strlen(s);
  while (*a && in_set(*a, set)) a++;
  while (b > a && in_set(b[-1], set)) b--;
  return dup_n(a, (size_t)(b - a));
}
static int str_count(const char *s, const char *sub) {
  int n = 0;
  size_t l = strlen(sub);
  const char *p = s;
  if (l == 0) return 0;
  while ((p = strstr(p, sub))) { n++; p += l; }
  return n;
}
static char *str_replace(const char *s, const char *from, const char *to) {
  size_t fl = strlen(from), tl = strlen(to), n = strlen(s);
  int cnt = str_count(s, from);
  char *r = malloc(n + (size_t)cnt * tl + 1);
  char *w = r;
  const char *q = s;
  while (1) {
    const char *m = fl ? strstr(q, from) : NULL;
    if (!m) { strcpy(w, q); break; }
    memcpy(w, q, (size_t)(m - q));
    w += m - q;
    memcpy(w, to, tl);
    w += tl;
    q = m + fl;
  }
  return r;
}
static char *strip_prefix(const char *s, const char *pre) {
  size_t pl = strlen(pre), sl = strlen(s);
  if (sl >= pl && memcmp(s, pre, pl) == 0) return dup_n(s + pl, sl - pl);
  return dup_n(s, sl);
}
static char *pad_left(const char *s, int width) {
  size_t l = strlen(s);
  if ((int)l >= width) return dup_n(s, l);
  char *r = malloc((size_t)width + 1);
  int pad = width - (int)l;
  memset(r, ' ', (size_t)pad);
  memcpy(r + pad, s, l);
  r[width] = '\0';
  return r;
}
static char **str_split(const char *s, const char *sep, int *out_n) {
  size_t sl = strlen(sep);
  int cap = 8, n = 0;
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
  *out_n = n;
  return parts;
}
static char *str_join(char **parts, int n, const char *sep) {
  size_t sl = strlen(sep), total = 0;
  for (int i = 0; i < n; i++) total += strlen(parts[i]);
  if (n > 0) total += sl * (size_t)(n - 1);
  char *r = malloc(total + 1);
  size_t pos = 0;
  for (int i = 0; i < n; i++) {
    if (i) { memcpy(r + pos, sep, sl); pos += sl; }
    size_t pl = strlen(parts[i]);
    memcpy(r + pos, parts[i], pl);
    pos += pl;
  }
  r[pos] = '\0';
  return r;
}

/* ----- concat: accumulate "i," via repeated realloc-grow --------------- */

static void test_strbuild_concat(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char num[16];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    char *s = NULL;
    size_t len = 0, cap = 0;
    for (int i = 0; i < 4000; i++) {
      int nl = snprintf(num, sizeof num, "%d,", i);
      if (len + (size_t)nl + 1 > cap) {
        cap = (len + (size_t)nl + 1) * 2;
        s = realloc(s, cap);
      }
      memcpy(s + len, num, (size_t)nl);
      len += (size_t)nl;
    }
    checksum = (long)len;
    t[r] = now_ns() - t0;
    free(s);
  }
  fprintf(stderr, "strbuild_concat = %ld\n", checksum);
  record("strbuild", "concat", t, RUN);
  free(t);
}

/* ----- join: collect tokens, join once, append trailing "," ------------ */

static void test_strbuild_join(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char num[16];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    char **parts = malloc(4000 * sizeof(char *));
    for (int i = 0; i < 4000; i++) {
      snprintf(num, sizeof num, "%d", i);
      parts[i] = dup_n(num, strlen(num));
    }
    char *joined = str_join(parts, 4000, ",");
    size_t jl = strlen(joined);
    char *s = malloc(jl + 2);
    memcpy(s, joined, jl);
    s[jl] = ',';
    s[jl + 1] = '\0';
    checksum = (long)strlen(s);
    t[r] = now_ns() - t0;
    for (int i = 0; i < 4000; i++) free(parts[i]);
    free(parts);
    free(joined);
    free(s);
  }
  fprintf(stderr, "strbuild_join = %ld\n", checksum);
  record("strbuild", "join", t, RUN);
  free(t);
}

/* ----- splitjoin: split a CSV-ish line then join back, in a loop -------- */

static void test_strbuild_splitjoin(void) {
  char field[16];
  char *fields[100];
  for (int i = 0; i < 100; i++) {
    snprintf(field, sizeof field, "field%d", i);
    fields[i] = dup_n(field, strlen(field));
  }
  char *line = str_join(fields, 100, ",");
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int p = 0; p < 2000; p++) {
      int nparts;
      char **parts = str_split(line, ",", &nparts);
      char *rejoined = str_join(parts, nparts, ",");
      acc += nparts + (long)strlen(rejoined);
      for (int k = 0; k < nparts; k++) free(parts[k]);
      free(parts);
      free(rejoined);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "strbuild_splitjoin = %ld\n", checksum);
  record("strbuild", "splitjoin", t, RUN);
  free(t);
  for (int i = 0; i < 100; i++) free(fields[i]);
  free(line);
}

/* ----- clean: trimChars / replace / stripPrefix / padLeft chain --------- */

static void test_strbuild_clean(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char raw[32];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 20000; i++) {
      snprintf(raw, sizeof raw, "  key_%d__  ", i);
      char *a = str_trim_chars(raw, " _");
      char *b = str_replace(a, "key", "K");
      char *c = strip_prefix(b, "K");
      char *d = pad_left(c, 12);
      acc += (long)strlen(d);
      free(a);
      free(b);
      free(c);
      free(d);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "strbuild_clean = %ld\n", checksum);
  record("strbuild", "clean", t, RUN);
  free(t);
}

void run_strbuild_group(void) {
  test_strbuild_concat();
  test_strbuild_join();
  test_strbuild_splitjoin();
  test_strbuild_clean();
}

/* ===================== string group: unibig ========================== *
 *
 * Mirror of string.mfb's test_string_unibig at the same tiny counts. The
 * grapheme / NFC lengths are APPROXIMATED (a grapheme = base scalar plus any
 * following U+0300..U+036F combining marks; normalizeNfc length ~ scalar
 * count) so the checksum is stable but not required to match mfb. */

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
static int is_combining(uint32_t cp) { return cp >= 0x300 && cp <= 0x36F; }
static int scalar_count(const char *s) {
  const unsigned char *p = (const unsigned char *)s;
  int n = 0;
  uint32_t cp;
  while (*p) { p += utf8_next(p, &cp); n++; }
  return n;
}
static int grapheme_count(const char *s) {
  const unsigned char *p = (const unsigned char *)s;
  int n = 0, open = 0;
  uint32_t cp;
  while (*p) {
    p += utf8_next(p, &cp);
    if (is_combining(cp) && open) continue;
    n++;
    open = 1;
  }
  return n;
}

void test_string_unibig(void) {
  /* Same fragment as scalarbench: "café 中文 rocket🚀 naïve Straße " with
   * DECOMPOSED naïve (nai + U+0308). Built from explicit universal-character
   * escapes; repeated 8 times. */
  const char *frag = u8"café 中文 rocket\U0001F680 naïve Straße ";
  size_t fl = strlen(frag);
  char *base = malloc(fl * 8 + 1);
  for (int i = 0; i < 8; i++) memcpy(base + (size_t)i * fl, frag, fl);
  base[fl * 8] = '\0';

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 5; i++) {
      acc += grapheme_count(base);   /* len(graphemes(base)) */
      acc += grapheme_count(base);   /* graphemesCount(base) */
      acc += 1;                      /* len(graphemeAt(base,0)) = "c" */
      acc += scalar_count(base);     /* len(normalizeNfc(base)) approx */
      acc += scalar_count(base);     /* len(caseFold(base)) approx */
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "string_unibig = %ld\n", checksum);
  record("string", "unibig", t, RUN);
  free(t);
  free(base);
}
