/* GROUP: regexbench — the C oracle for regexbench.mfb, using POSIX <regex.h>
 * (regcomp/regexec) as parsebench.c does for the existing `parse regex` row.
 * All inputs are ASCII so the match counts / rewritten lengths equal mfb:
 *   compile=50, capture=539, alternation=150, replace=1199
 *
 * POSIX regex has no $N substitution, so capture/replace build the output
 * manually from regmatch_t offsets. REG_NOTBOL is set after the first match so
 * `^`-anchored constructs behave (the patterns here are unanchored either way).*/
#include <regex.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "regexbench.h"

/* Count non-overlapping matches of a compiled pattern over text. */
static long count_matches(const regex_t *re, const char *text) {
  long n = 0;
  const char *cur = text;
  regmatch_t m;
  int flags = 0;
  while (regexec(re, cur, 1, &m, flags) == 0) {
    n++;
    cur += m.rm_eo > 0 ? m.rm_eo : 1;
    flags = REG_NOTBOL;
  }
  return n;
}

/* A tiny growable byte buffer for building rewritten strings. */
typedef struct { char *b; size_t n, cap; } Buf;
static void buf_append(Buf *buf, const char *s, size_t len) {
  if (buf->n + len + 1 > buf->cap) {
    buf->cap = (buf->n + len + 1) * 2;
    buf->b = realloc(buf->b, buf->cap);
  }
  memcpy(buf->b + buf->n, s, len);
  buf->n += len;
  buf->b[buf->n] = '\0';
}

/* compile — one pattern matched over 25 separate lines (compile-once/many). */
static void test_regex_compile(void) {
  char lines[25][32];
  for (int i = 0; i < 25; i++) snprintf(lines[i], sizeof lines[i], "row%d val %d", i, i * 7);
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    regex_t re;
    regcomp(&re, "[0-9]+", REG_EXTENDED);
    long total = 0;
    for (int i = 0; i < 25; i++) total += count_matches(&re, lines[i]);
    regfree(&re);
    checksum = total;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "regex_compile = %ld\n", checksum);
  record("regexbench", "compile", t, RUN);
  free(t);
}

/* capture — replace each "(d+)-(d+)-(d+)" with $1$2$3 (dashes dropped). */
static void test_regex_capture(void) {
  Buf tb = {0};
  for (int i = 0; i < 70; i++) {
    char tok[32];
    snprintf(tok, sizeof tok, "%d-%d-%d", 2000 + i, i % 12, i % 28);
    if (i) buf_append(&tb, " ", 1);
    buf_append(&tb, tok, strlen(tok));
  }
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    regex_t re;
    regcomp(&re, "([0-9]+)-([0-9]+)-([0-9]+)", REG_EXTENDED);
    Buf out = {0};
    const char *cur = tb.b;
    regmatch_t m[4];
    int flags = 0;
    while (regexec(&re, cur, 4, m, flags) == 0) {
      buf_append(&out, cur, (size_t)m[0].rm_so);          /* text before match */
      for (int g = 1; g <= 3; g++)                         /* $1$2$3 */
        buf_append(&out, cur + m[g].rm_so, (size_t)(m[g].rm_eo - m[g].rm_so));
      cur += m[0].rm_eo > 0 ? m[0].rm_eo : 1;
      flags = REG_NOTBOL;
    }
    buf_append(&out, cur, strlen(cur));                    /* trailing text */
    regfree(&re);
    checksum = (long)out.n;
    t[r] = now_ns() - t0;
    free(out.b);
  }
  fprintf(stderr, "regex_capture = %ld\n", checksum);
  record("regexbench", "capture", t, RUN);
  free(t);
  free(tb.b);
}

/* alternation — count all matches of a |-heavy pattern over a repeated text. */
static void test_regex_alternation(void) {
  Buf tb = {0};
  const char *frag = "the cat and dog saw a bird near fish and owl";
  for (int i = 0; i < 30; i++) {
    if (i) buf_append(&tb, " ", 1);
    buf_append(&tb, frag, strlen(frag));
  }
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    regex_t re;
    regcomp(&re, "cat|dog|bird|fish|owl", REG_EXTENDED);
    checksum = count_matches(&re, tb.b);
    regfree(&re);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "regex_alternation = %ld\n", checksum);
  record("regexbench", "alternation", t, RUN);
  free(t);
  free(tb.b);
}

/* replace — mask every number-token with "#". */
static void test_regex_replace(void) {
  Buf tb = {0};
  for (int i = 0; i < 300; i++) {
    char tok[16];
    snprintf(tok, sizeof tok, "%d-x", i);
    if (i) buf_append(&tb, " ", 1);
    buf_append(&tb, tok, strlen(tok));
  }
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    regex_t re;
    regcomp(&re, "[0-9]+", REG_EXTENDED);
    Buf out = {0};
    const char *cur = tb.b;
    regmatch_t m;
    int flags = 0;
    while (regexec(&re, cur, 1, &m, flags) == 0) {
      buf_append(&out, cur, (size_t)m.rm_so);
      buf_append(&out, "#", 1);
      cur += m.rm_eo > 0 ? m.rm_eo : 1;
      flags = REG_NOTBOL;
    }
    buf_append(&out, cur, strlen(cur));
    regfree(&re);
    checksum = (long)out.n;
    t[r] = now_ns() - t0;
    free(out.b);
  }
  fprintf(stderr, "regex_replace = %ld\n", checksum);
  record("regexbench", "replace", t, RUN);
  free(t);
  free(tb.b);
}

void run_regexbench_group(void) {
  test_regex_compile();
  test_regex_capture();
  test_regex_alternation();
  test_regex_replace();
}
