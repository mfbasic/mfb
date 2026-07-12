/* GROUP: map (collections:: over maps)
 *
 * Moved into its own translation unit; shared timing/recording infra comes
 * from bench.h. Contains the two historical timing rows (set, lookup) plus the
 * consolidated coverage rows (int_ops, str_ops) that mirror map.mfb and drive
 * every Map-shaped collections:: member: set, get, getOr, hasKey, keys,
 * values, mapValues, merge, removeKey — over an Integer-valued and a
 * String-valued map. The coverage rows accumulate the same len()/value
 * arithmetic as the mfb version so the checksums match (38400 and 37850). */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "mapbench.h"

typedef struct { char *key; long val; int used; } SSlot;
static unsigned long djb2(const char *s) {
  unsigned long h = 5381; int c;
  while ((c = *s++)) h = ((h << 5) + h) + c;
  return h;
}

/* ----- historical timing rows ------------------------------------------ */

static void test_map_set(void) {
#define SMAP_CAP 4096
  long long *t = alloc_times();
  long checksum = 0;
  char buf[16];
  for (int r = 0; r < RUN; r++) {
    SSlot *table = calloc(SMAP_CAP, sizeof(SSlot));
    long long t0 = now_ns();
    for (int i = 0; i < 1000; i++) {
      snprintf(buf, sizeof buf, "%d", i);
      unsigned long h = djb2(buf) & (SMAP_CAP - 1);
      while (table[h].used) {
        if (strcmp(table[h].key, buf) == 0) break;
        h = (h + 1) & (SMAP_CAP - 1);
      }
      if (!table[h].used) { table[h].key = strdup(buf); table[h].used = 1; }
      table[h].val = i;
    }
    long sum = 0;
    for (int i = 0; i < 1000; i++) {
      snprintf(buf, sizeof buf, "%d", i);
      unsigned long h = djb2(buf) & (SMAP_CAP - 1);
      while (table[h].used) {
        if (strcmp(table[h].key, buf) == 0) { sum += table[h].val; break; }
        h = (h + 1) & (SMAP_CAP - 1);
      }
    }
    checksum = sum;
    t[r] = now_ns() - t0;
    for (int i = 0; i < SMAP_CAP; i++) if (table[i].used) free(table[i].key);
    free(table);
  }
  fprintf(stderr, "map_set = %ld\n", checksum);
  record("map", "set", t, RUN);
  free(t);
}

static void test_map_lookup(void) {
#define IMAP_CAP 32768
#define IMAP_N 20000
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long *keys = calloc(IMAP_CAP, sizeof(long));
    long *vals = calloc(IMAP_CAP, sizeof(long));
    char *used = calloc(IMAP_CAP, 1);
    long long t0 = now_ns();
    for (long i = 0; i < IMAP_N; i++) {
      size_t h = (size_t)(i * 1099511628211UL) & (IMAP_CAP - 1);
      while (used[h] && keys[h] != i) h = (h + 1) & (IMAP_CAP - 1);
      used[h] = 1; keys[h] = i; vals[h] = i;
    }
    long sum = 0;
    for (long i = 0; i < IMAP_N; i++) {
      size_t h = (size_t)(i * 1099511628211UL) & (IMAP_CAP - 1);
      while (used[h] && keys[h] != i) h = (h + 1) & (IMAP_CAP - 1);
      sum += vals[h];
    }
    checksum = sum;
    t[r] = now_ns() - t0;
    free(keys); free(vals); free(used);
  }
  fprintf(stderr, "map_lookup = %ld\n", checksum);
  record("map", "lookup", t, RUN);
  free(t);
}

/* ----- coverage: an open-addressing String-keyed map -------------------- */

#define MCAP 1024

/* Integer-valued map. */
typedef struct { char *keys[MCAP]; long vals[MCAP]; char used[MCAP]; } IMap;

static void imap_init(IMap *m) { memset(m, 0, sizeof(*m)); }
static void imap_free(IMap *m) {
  for (int i = 0; i < MCAP; i++) if (m->used[i]) free(m->keys[i]);
}
static void imap_set(IMap *m, const char *k, long v) {
  unsigned long h = djb2(k) & (MCAP - 1);
  while (m->used[h]) {
    if (strcmp(m->keys[h], k) == 0) { m->vals[h] = v; return; }
    h = (h + 1) & (MCAP - 1);
  }
  m->keys[h] = strdup(k); m->used[h] = 1; m->vals[h] = v;
}
static int imap_find(const IMap *m, const char *k) {
  unsigned long h = djb2(k) & (MCAP - 1);
  while (m->used[h]) {
    if (strcmp(m->keys[h], k) == 0) return (int)h;
    h = (h + 1) & (MCAP - 1);
  }
  return -1;
}
static int imap_has(const IMap *m, const char *k) { return imap_find(m, k) >= 0; }
static long imap_get(const IMap *m, const char *k) {
  int i = imap_find(m, k); return i >= 0 ? m->vals[i] : 0;
}
static long imap_getor(const IMap *m, const char *k, long def) {
  int i = imap_find(m, k); return i >= 0 ? m->vals[i] : def;
}
static int imap_count(const IMap *m) {
  int n = 0;
  for (int i = 0; i < MCAP; i++) if (m->used[i]) n++;
  return n;
}

static void test_map_int_ops(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char buf[16];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int pass = 0; pass < 50; pass++) {
      IMap m; imap_init(&m);
      for (int i = 0; i < 200; i++) { snprintf(buf, sizeof buf, "%d", i); imap_set(&m, buf, i); }
      IMap other; imap_init(&other);
      for (int i = 200; i <= 249; i++) { snprintf(buf, sizeof buf, "%d", i); imap_set(&other, buf, i); }
      /* merge(m, other, TRUE) */
      IMap merged; imap_init(&merged);
      for (int i = 0; i < MCAP; i++) if (m.used[i]) imap_set(&merged, m.keys[i], m.vals[i]);
      for (int i = 0; i < MCAP; i++) if (other.used[i]) imap_set(&merged, other.keys[i], other.vals[i]);
      /* mapValues(merged, v -> v + v) */
      IMap doubled; imap_init(&doubled);
      for (int i = 0; i < MCAP; i++) if (merged.used[i]) imap_set(&doubled, merged.keys[i], merged.vals[i] * 2);
      acc += imap_count(&doubled);   /* len(keys)   */
      acc += imap_count(&doubled);   /* len(values) */
      if (imap_has(&doubled, "10")) acc += imap_get(&doubled, "10");
      acc += imap_getor(&doubled, "missing", -1);
      /* removeKey(doubled, "0") */
      IMap pruned; imap_init(&pruned);
      for (int i = 0; i < MCAP; i++)
        if (doubled.used[i] && strcmp(doubled.keys[i], "0") != 0)
          imap_set(&pruned, doubled.keys[i], doubled.vals[i]);
      acc += imap_count(&pruned);
      imap_free(&m); imap_free(&other); imap_free(&merged);
      imap_free(&doubled); imap_free(&pruned);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "map_int_ops = %ld\n", checksum);
  record("map", "int_ops", t, RUN);
  free(t);
}

/* String-valued map. */
typedef struct { char *keys[MCAP]; char *vals[MCAP]; char used[MCAP]; } SMap;

static void smap_init(SMap *m) { memset(m, 0, sizeof(*m)); }
static void smap_free(SMap *m) {
  for (int i = 0; i < MCAP; i++) if (m->used[i]) { free(m->keys[i]); free(m->vals[i]); }
}
static void smap_set(SMap *m, const char *k, const char *v) {
  unsigned long h = djb2(k) & (MCAP - 1);
  while (m->used[h]) {
    if (strcmp(m->keys[h], k) == 0) { free(m->vals[h]); m->vals[h] = strdup(v); return; }
    h = (h + 1) & (MCAP - 1);
  }
  m->keys[h] = strdup(k); m->used[h] = 1; m->vals[h] = strdup(v);
}
static int smap_find(const SMap *m, const char *k) {
  unsigned long h = djb2(k) & (MCAP - 1);
  while (m->used[h]) {
    if (strcmp(m->keys[h], k) == 0) return (int)h;
    h = (h + 1) & (MCAP - 1);
  }
  return -1;
}
static int smap_has(const SMap *m, const char *k) { return smap_find(m, k) >= 0; }
static const char *smap_get(const SMap *m, const char *k) {
  int i = smap_find(m, k); return i >= 0 ? m->vals[i] : "";
}
static int smap_count(const SMap *m) {
  int n = 0;
  for (int i = 0; i < MCAP; i++) if (m->used[i]) n++;
  return n;
}

static void test_map_str_ops(void) {
  long long *t = alloc_times();
  long checksum = 0;
  char kbuf[16], vbuf[24];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int pass = 0; pass < 50; pass++) {
      SMap m; smap_init(&m);
      for (int i = 0; i < 200; i++) {
        snprintf(kbuf, sizeof kbuf, "%d", i);
        snprintf(vbuf, sizeof vbuf, "v%d", i);
        smap_set(&m, kbuf, vbuf);
      }
      SMap other; smap_init(&other);
      for (int i = 200; i <= 249; i++) {
        snprintf(kbuf, sizeof kbuf, "%d", i);
        snprintf(vbuf, sizeof vbuf, "v%d", i);
        smap_set(&other, kbuf, vbuf);
      }
      SMap merged; smap_init(&merged);
      for (int i = 0; i < MCAP; i++) if (m.used[i]) smap_set(&merged, m.keys[i], m.vals[i]);
      for (int i = 0; i < MCAP; i++) if (other.used[i]) smap_set(&merged, other.keys[i], other.vals[i]);
      /* mapValues(merged, v -> v & "!") */
      SMap tagged; smap_init(&tagged);
      for (int i = 0; i < MCAP; i++) if (merged.used[i]) {
        snprintf(vbuf, sizeof vbuf, "%s!", merged.vals[i]);
        smap_set(&tagged, merged.keys[i], vbuf);
      }
      acc += smap_count(&tagged);   /* len(keys)   */
      acc += smap_count(&tagged);   /* len(values) */
      if (smap_has(&tagged, "10")) acc += (long)strlen(smap_get(&tagged, "10"));
      acc += (long)strlen("none"); /* getOr(tagged, "missing", "none") */
      SMap pruned; smap_init(&pruned);
      for (int i = 0; i < MCAP; i++)
        if (tagged.used[i] && strcmp(tagged.keys[i], "0") != 0)
          smap_set(&pruned, tagged.keys[i], tagged.vals[i]);
      acc += smap_count(&pruned);
      smap_free(&m); smap_free(&other); smap_free(&merged);
      smap_free(&tagged); smap_free(&pruned);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "map_str_ops = %ld\n", checksum);
  record("map", "str_ops", t, RUN);
  free(t);
}

void run_map_group(void) {
  test_map_set();
  test_map_lookup();
  test_map_int_ops();
  test_map_str_ops();
}
