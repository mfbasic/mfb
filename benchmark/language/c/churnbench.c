/* GROUP: listchurn + mapchurn — the C oracle for listchurn.mfb / mapchurn.mfb.
 *
 * These reproduce the mfb build-by-append / prepend-shift / nested-copy and the
 * map grow-rehash / insert-delete / materialize hot paths with plain C data
 * structures (a growable long array for lists, a djb2 open-addressing map with
 * tombstone deletion for maps), doing comparable materialized work so timing is
 * fair. Only the checksums must match mfb:
 *   listchurn: append=199990000, prepend=1999000, nested=160042000
 *   mapchurn:  grow=12497500, churn=2128750, iterate=50153000 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "churnbench.h"

/* ===================== listchurn ===================================== */

static void test_listchurn_append(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long *nums = NULL;
    int len = 0, cap = 0;
    for (int i = 0; i < 20000; i++) {
      if (len == cap) {
        cap = cap ? cap * 2 : 16;
        nums = realloc(nums, (size_t)cap * sizeof(long));
      }
      nums[len++] = i;
    }
    long sumv = 0;
    for (int i = 0; i < len; i++) sumv += nums[i];
    checksum = sumv;
    t[r] = now_ns() - t0;
    free(nums);
  }
  fprintf(stderr, "listchurn_append = %ld\n", checksum);
  record("listchurn", "append", t, RUN);
  free(t);
}

static void test_listchurn_prepend(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long *nums = malloc(2000 * sizeof(long));
    int len = 0;
    for (int i = 0; i < 2000; i++) {
      memmove(nums + 1, nums, (size_t)len * sizeof(long)); /* O(n) front shift */
      nums[0] = i;
      len++;
    }
    long sumv = 0;
    for (int i = 0; i < len; i++) sumv += nums[i];
    checksum = sumv;
    t[r] = now_ns() - t0;
    free(nums);
  }
  fprintf(stderr, "listchurn_prepend = %ld\n", checksum);
  record("listchurn", "prepend", t, RUN);
  free(t);
}

static void test_listchurn_nested(void) {
  const int outer = 200, inner = 20, passes = 20;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int p = 0; p < passes; p++) {
      long **rows = malloc((size_t)outer * sizeof(long *));
      for (int i = 0; i < outer; i++) {
        long *row = malloc((size_t)inner * sizeof(long));
        for (int j = 0; j < inner; j++) row[j] = i * inner + j;
        rows[i] = row;
      }
      int fn = outer * inner;
      long *flat = malloc((size_t)fn * sizeof(long));
      int fi = 0;
      for (int i = 0; i < outer; i++)
        for (int j = 0; j < inner; j++) flat[fi++] = rows[i][j];
      acc += fn; /* len(flat) */
      long sf = 0;
      for (int i = 0; i < fn; i++) sf += flat[i];
      acc += sf;
      /* groupBy(flat, bucketKey=n%100, identity) — count distinct keys */
      char seen[100];
      memset(seen, 0, sizeof seen);
      int groups = 0;
      for (int i = 0; i < fn; i++) {
        int k = (int)(flat[i] % 100);
        if (!seen[k]) { seen[k] = 1; groups++; }
      }
      acc += groups; /* len(groups) */
      for (int i = 0; i < outer; i++) free(rows[i]);
      free(rows);
      free(flat);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "listchurn_nested = %ld\n", checksum);
  record("listchurn", "nested", t, RUN);
  free(t);
}

void run_listchurn_group(void) {
  test_listchurn_append();
  test_listchurn_prepend();
  test_listchurn_nested();
}

/* ===================== mapchurn ====================================== */

/* djb2 open-addressing map, String keys -> long values, tombstone deletion.
 * cap must be a power of two. state: 0 empty, 1 used, 2 tombstone. */
typedef struct { char *key; long val; int state; } MSlot;
typedef struct { MSlot *s; int cap; } CMap;

static unsigned long djb2(const char *s) {
  unsigned long h = 5381;
  int c;
  while ((c = *s++)) h = ((h << 5) + h) + (unsigned long)c;
  return h;
}

static CMap cmap_new(int cap) {
  CMap m;
  m.cap = cap;
  m.s = calloc((size_t)cap, sizeof(MSlot));
  return m;
}
static void cmap_free(CMap *m) {
  for (int i = 0; i < m->cap; i++)
    if (m->s[i].state == 1) free(m->s[i].key);
  free(m->s);
  m->s = NULL;
}
static void cmap_set(CMap *m, const char *k, long v) {
  unsigned long h = djb2(k) & (unsigned long)(m->cap - 1);
  long first_tomb = -1;
  while (m->s[h].state != 0) {
    if (m->s[h].state == 1 && strcmp(m->s[h].key, k) == 0) { m->s[h].val = v; return; }
    if (m->s[h].state == 2 && first_tomb < 0) first_tomb = (long)h;
    h = (h + 1) & (unsigned long)(m->cap - 1);
  }
  unsigned long slot = first_tomb >= 0 ? (unsigned long)first_tomb : h;
  m->s[slot].key = strdup(k);
  m->s[slot].state = 1;
  m->s[slot].val = v;
}
static int cmap_findidx(const CMap *m, const char *k) {
  unsigned long h = djb2(k) & (unsigned long)(m->cap - 1);
  while (m->s[h].state != 0) {
    if (m->s[h].state == 1 && strcmp(m->s[h].key, k) == 0) return (int)h;
    h = (h + 1) & (unsigned long)(m->cap - 1);
  }
  return -1;
}
static int cmap_has(const CMap *m, const char *k) { return cmap_findidx(m, k) >= 0; }
static long cmap_get(const CMap *m, const char *k) {
  int i = cmap_findidx(m, k);
  return i >= 0 ? m->s[i].val : 0;
}
static void cmap_remove(CMap *m, const char *k) {
  int i = cmap_findidx(m, k);
  if (i >= 0) { free(m->s[i].key); m->s[i].key = NULL; m->s[i].state = 2; }
}

static void test_mapchurn_grow(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char buf[16];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    CMap m = cmap_new(16384);
    for (int i = 0; i < 5000; i++) { snprintf(buf, sizeof buf, "%d", i); cmap_set(&m, buf, i); }
    long sumv = 0;
    for (int i = 0; i < 5000; i++) {
      snprintf(buf, sizeof buf, "%d", i);
      if (cmap_has(&m, buf)) sumv += cmap_get(&m, buf);
    }
    checksum = sumv;
    t[r] = now_ns() - t0;
    cmap_free(&m);
  }
  fprintf(stderr, "mapchurn_grow = %ld\n", checksum);
  record("mapchurn", "grow", t, RUN);
  free(t);
}

static void test_mapchurn_churn(void) {
  const int base = 500, cycles = 4000;
  long long *t = alloc_times();
  long checksum = 0;
  char buf[16];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    CMap m = cmap_new(16384);
    for (int i = 0; i < base; i++) { snprintf(buf, sizeof buf, "%d", i); cmap_set(&m, buf, i); }
    long removed = 0;
    for (int stp = 0; stp < cycles; stp++) {
      int newkey = base + stp;
      snprintf(buf, sizeof buf, "%d", newkey);
      cmap_set(&m, buf, newkey);
      snprintf(buf, sizeof buf, "%d", stp);
      if (cmap_has(&m, buf)) { cmap_remove(&m, buf); removed++; }
    }
    long sumv = 0;
    for (int i = 0; i < m.cap; i++)
      if (m.s[i].state == 1) sumv += m.s[i].val;
    checksum = sumv + removed;
    t[r] = now_ns() - t0;
    cmap_free(&m);
  }
  fprintf(stderr, "mapchurn_churn = %ld\n", checksum);
  record("mapchurn", "churn", t, RUN);
  free(t);
}

static void test_mapchurn_iterate(void) {
  const int n = 1000, passes = 100;
  long long *t = alloc_times();
  long checksum = 0;
  char buf[16];
  for (int r = 0; r < RUN; r++) {
    CMap m = cmap_new(4096);
    for (int i = 0; i < n; i++) { snprintf(buf, sizeof buf, "%d", i); cmap_set(&m, buf, i); }
    CMap other = cmap_new(64);
    for (int i = n; i < n + 10; i++) { snprintf(buf, sizeof buf, "%d", i); cmap_set(&other, buf, i); }
    long long t0 = now_ns();
    long acc = 0;
    for (int p = 0; p < passes; p++) {
      int nk = 0;
      for (int i = 0; i < m.cap; i++) if (m.s[i].state == 1) nk++;
      acc += nk; /* len(keys(m)) */
      long sv = 0;
      for (int i = 0; i < m.cap; i++) if (m.s[i].state == 1) sv += m.s[i].val;
      acc += sv; /* sum(values(m)) */
      CMap dbl = cmap_new(4096);
      for (int i = 0; i < m.cap; i++) if (m.s[i].state == 1) cmap_set(&dbl, m.s[i].key, m.s[i].val * 2);
      acc += cmap_get(&dbl, "10");
      CMap mg = cmap_new(4096);
      for (int i = 0; i < m.cap; i++) if (m.s[i].state == 1) cmap_set(&mg, m.s[i].key, m.s[i].val);
      for (int i = 0; i < other.cap; i++) if (other.s[i].state == 1) cmap_set(&mg, other.s[i].key, other.s[i].val);
      int mgk = 0;
      for (int i = 0; i < mg.cap; i++) if (mg.s[i].state == 1) mgk++;
      acc += mgk; /* len(keys(merge(m,other,TRUE))) */
      cmap_free(&dbl);
      cmap_free(&mg);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
    cmap_free(&m);
    cmap_free(&other);
  }
  fprintf(stderr, "mapchurn_iterate = %ld\n", checksum);
  record("mapchurn", "iterate", t, RUN);
  free(t);
}

void run_mapchurn_group(void) {
  test_mapchurn_grow();
  test_mapchurn_churn();
  test_mapchurn_iterate();
}
