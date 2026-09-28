/* GROUP: elem — the C peer for elem.mfb (P1 coverage): the common collection
 * ops over a record element (struct Item {id, name, score}), a double element,
 * and an int -> Item hash map. Mirrors what the mfb rows observably do:
 *   - filter/transform/sortBy MATERIALIZE a fresh Item array per call (struct
 *     copies; the immutable name string is shared by pointer, C's idiomatic
 *     copy of an immutable string), pinned with bench_opaque();
 *   - sortBy is a stable top-down merge sort on the score key (qsort is not
 *     stable, and the order-sensitive id hash proves tie order);
 *   - find compares every field (id, score, strcmp name) — record equality;
 *   - the map is open addressing (linear probe, power-of-two, grown from
 *     empty), rebuilt from empty inside the timed region like mfb's.
 * Scores are exact dyadics (multiples of 0.25), so every double sum is exact
 * and the (long long) truncations match mfb's toInt and Python's int().
 *
 * Expected checksums: rec_get=524258000, rec_set=681148,
 * rec_sortby=1463331720, rec_find=999000, rec_filter=101400,
 * rec_transform=240236, flt_sort=1163083625, flt_sum=9999000000,
 * flt_transform=4994700, map_rec_set=20217, map_rec_get=262129000. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "elembench.h"

typedef struct {
  long long id;
  char *name;
  double score;
} Item;

#define ELEM_N 1000

static Item make_item(int i) {
  char buf[32];
  snprintf(buf, sizeof buf, "item%d", i);
  Item it = {i, strdup(buf), (double)((i * 37) % 101) * 0.5};
  return it;
}

static Item *build_items(int n) {
  Item *xs = malloc(sizeof(Item) * n);
  for (int i = 0; i < n; i++) xs[i] = make_item(i);
  return xs;
}

static void free_items(Item *xs, int n) {
  for (int i = 0; i < n; i++) free(xs[i].name);
  free(xs);
}

static double *build_floats(int n) {
  double *xs = malloc(sizeof(double) * n);
  for (int i = 0; i < n; i++) xs[i] = (double)(((long long)i * 7919) % n) * 0.25;
  return xs;
}

static int item_eq(const Item *a, const Item *b) {
  return a->id == b->id && a->score == b->score && strcmp(a->name, b->name) == 0;
}

/* Stable merge sort by score (ties keep their input order). */
static void msort_rec(Item *a, Item *tmp, int lo, int hi) {
  if (hi - lo < 2) return;
  int mid = lo + (hi - lo) / 2;
  msort_rec(a, tmp, lo, mid);
  msort_rec(a, tmp, mid, hi);
  int i = lo, j = mid, k = lo;
  while (i < mid && j < hi) tmp[k++] = (a[j].score < a[i].score) ? a[j++] : a[i++];
  while (i < mid) tmp[k++] = a[i++];
  while (j < hi) tmp[k++] = a[j++];
  memcpy(a + lo, tmp + lo, sizeof(Item) * (hi - lo));
}

static int cmp_dbl(const void *a, const void *b) {
  double x = *(const double *)a, y = *(const double *)b;
  return x < y ? -1 : x > y ? 1 : 0;
}

/* ---- int -> Item open-addressing map ------------------------------------ */
typedef struct {
  long long *keys;
  Item *vals;
  unsigned char *used;
  int cap, n;
} IMap;

static void imap_init(IMap *m, int cap) {
  m->cap = cap;
  m->n = 0;
  m->keys = malloc(sizeof(long long) * cap);
  m->vals = malloc(sizeof(Item) * cap);
  m->used = calloc(cap, 1);
}
static void imap_free(IMap *m) { free(m->keys); free(m->vals); free(m->used); }
static unsigned imap_hash(long long k) { unsigned long long h = (unsigned long long)k * 0x9E3779B97F4A7C15ull; return (unsigned)(h >> 32); }
static void imap_set(IMap *m, long long k, Item v);
static void imap_grow(IMap *m) {
  IMap old = *m;
  imap_init(m, old.cap * 2);
  for (int i = 0; i < old.cap; i++)
    if (old.used[i]) imap_set(m, old.keys[i], old.vals[i]);
  imap_free(&old);
}
static void imap_set(IMap *m, long long k, Item v) {
  if ((m->n + 1) * 4 > m->cap * 3) imap_grow(m);
  unsigned mask = (unsigned)m->cap - 1, i = imap_hash(k) & mask;
  while (m->used[i]) {
    if (m->keys[i] == k) { m->vals[i] = v; return; }
    i = (i + 1) & mask;
  }
  m->used[i] = 1;
  m->keys[i] = k;
  m->vals[i] = v;
  m->n++;
}
static const Item *imap_get(const IMap *m, long long k) {
  unsigned mask = (unsigned)m->cap - 1, i = imap_hash(k) & mask;
  while (m->used[i]) {
    if (m->keys[i] == k) return &m->vals[i];
    i = (i + 1) & mask;
  }
  return NULL;
}

/* ---- row driver ---------------------------------------------------------- */
#define ROW(nm, body)                                                   \
  do {                                                                  \
    long long *t = alloc_times();                                       \
    long long checksum = 0;                                             \
    for (int r = 0; r < RUN; r++) {                                     \
      long long t0 = now_ns();                                          \
      body;                                                             \
      t[r] = now_ns() - t0;                                             \
    }                                                                   \
    fprintf(stderr, "test_elem_" nm " = %lld\n", checksum);             \
    record("elem", nm, t, RUN);                                         \
    free(t);                                                            \
  } while (0)

static void test_rec_get(void) {
  Item *items = build_items(ELEM_N);
  ROW("rec_get", {
    long long acc = 0;
    for (int k = 0; k < 1000; k++) {
      bench_opaque(items);
      for (int i = 0; i < ELEM_N; i++) {
        Item it = items[i];
        acc += it.id + (long long)it.score;
      }
    }
    checksum = acc;
  });
  free_items(items, ELEM_N);
}

static void test_rec_set(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    Item *items = build_items(ELEM_N); /* rebuilt untimed each run */
    long long t0 = now_ns();
    for (int pass = 0; pass < 100; pass++)
      for (int j = 0; j < ELEM_N; j++) {
        Item it = items[j];
        it.id += 1;
        it.score += 0.5;
        items[j] = it;
      }
    long long acc = 0;
    for (int j = 0; j < ELEM_N; j++) acc += items[j].id + (long long)items[j].score + (long long)strlen(items[j].name);
    checksum = acc;
    t[r] = now_ns() - t0;
    free_items(items, ELEM_N);
  }
  fprintf(stderr, "test_elem_rec_set = %lld\n", checksum);
  record("elem", "rec_set", t, RUN);
  free(t);
}

static void test_rec_sortby(void) {
  Item *items = build_items(ELEM_N);
  ROW("rec_sortby", {
    long long acc = 0;
    for (int k = 0; k < 20; k++) {
      Item *s = malloc(sizeof(Item) * ELEM_N);
      Item *tmp = malloc(sizeof(Item) * ELEM_N);
      memcpy(s, items, sizeof(Item) * ELEM_N);
      msort_rec(s, tmp, 0, ELEM_N);
      bench_opaque(s);
      long long h = 0;
      for (int i = 0; i < ELEM_N; i++) h = (h * 31 + s[i].id) % 1000000007;
      acc += h;
      free(tmp);
      free(s);
    }
    checksum = acc;
  });
  free_items(items, ELEM_N);
}

static void test_rec_find(void) {
  Item *items = build_items(ELEM_N);
  Item needle = make_item(999);
  ROW("rec_find", {
    long long acc = 0;
    for (int k = 0; k < 1000; k++) {
      bench_opaque(items);
      int idx = -1;
      for (int i = 0; i < ELEM_N; i++)
        if (item_eq(&items[i], &needle)) { idx = i; break; }
      acc += idx;
    }
    checksum = acc;
  });
  free(needle.name);
  free_items(items, ELEM_N);
}

static void test_rec_filter(void) {
  Item *items = build_items(ELEM_N);
  ROW("rec_filter", {
    long long acc = 0;
    for (int k = 0; k < 200; k++) {
      Item *high = malloc(sizeof(Item) * ELEM_N);
      int n = 0;
      for (int i = 0; i < ELEM_N; i++)
        if (items[i].score >= 25.0) high[n++] = items[i];
      bench_opaque(high);
      acc += n + high[0].id;
      free(high);
    }
    checksum = acc;
  });
  free_items(items, ELEM_N);
}

static void test_rec_transform(void) {
  Item *items = build_items(ELEM_N);
  ROW("rec_transform", {
    long long acc = 0;
    for (int k = 0; k < 200; k++) {
      Item *scaled = malloc(sizeof(Item) * ELEM_N);
      for (int i = 0; i < ELEM_N; i++) {
        scaled[i] = items[i];
        scaled[i].score = items[i].score * 2.0 + 0.25;
      }
      bench_opaque(scaled);
      acc += ELEM_N + (long long)(scaled[k].score * 4.0);
      free(scaled);
    }
    checksum = acc;
  });
  free_items(items, ELEM_N);
}

static void test_flt_sort(void) {
  const int n = 20000;
  double *xs = build_floats(n);
  ROW("flt_sort", {
    long long acc = 0;
    for (int k = 0; k < 5; k++) {
      double *s = malloc(sizeof(double) * n);
      memcpy(s, xs, sizeof(double) * n);
      qsort(s, n, sizeof(double), cmp_dbl); /* distinct keys: stability moot */
      bench_opaque(s);
      long long h = 0;
      for (int i = 0; i < n; i++) h = (h * 31 + (long long)(s[i] * 4.0)) % 1000000007;
      acc += h;
      free(s);
    }
    checksum = acc;
  });
  free(xs);
}

static double add_f(double acc, double x) { return acc + x; }

static void test_flt_sum(void) {
  const int n = 10000;
  double *xs = build_floats(n);
  ROW("flt_sum", {
    long long acc = 0;
    for (int k = 0; k < 200; k++) {
      bench_opaque(xs);
      double total = 0.0;
      for (int i = 0; i < n; i++) total = add_f(total, xs[i]);
      acc += (long long)(total * 4.0);
    }
    checksum = acc;
  });
  free(xs);
}

static void test_flt_transform(void) {
  const int n = 10000;
  double *xs = build_floats(n);
  ROW("flt_transform", {
    long long acc = 0;
    for (int k = 0; k < 200; k++) {
      double *ys = malloc(sizeof(double) * n);
      for (int i = 0; i < n; i++) ys[i] = xs[i] * 1.5 + 0.25;
      bench_opaque(ys);
      acc += n + (long long)(ys[k] * 8.0);
      free(ys);
    }
    checksum = acc;
  });
  free(xs);
}

static void test_map_rec_set(void) {
  Item *items = build_items(ELEM_N);
  ROW("map_rec_set", {
    long long acc = 0;
    for (int k = 0; k < 10; k++) {
      IMap m;
      imap_init(&m, 8);
      for (int i = 0; i < ELEM_N; i++) imap_set(&m, i, items[i]);
      for (int i = 0; i < ELEM_N; i++) {
        Item it = items[i];
        it.score += 1.0;
        imap_set(&m, i, it);
      }
      bench_opaque(m.vals);
      const Item *last = imap_get(&m, 999 - k);
      acc += m.n + last->id + (long long)last->score;
      imap_free(&m);
    }
    checksum = acc;
  });
  free_items(items, ELEM_N);
}

static void test_map_rec_get(void) {
  Item *items = build_items(ELEM_N);
  IMap m;
  imap_init(&m, 8);
  for (int i = 0; i < ELEM_N; i++) imap_set(&m, i, items[i]);
  ROW("map_rec_get", {
    long long acc = 0;
    for (int k = 0; k < 500; k++) {
      bench_opaque(m.vals);
      for (int i = 0; i < ELEM_N; i++) {
        const Item *it = imap_get(&m, i);
        acc += it->id + (long long)it->score;
      }
    }
    checksum = acc;
  });
  imap_free(&m);
  free_items(items, ELEM_N);
}

void run_elem_group(void) {
  test_rec_get();
  test_rec_set();
  test_rec_sortby();
  test_rec_find();
  test_rec_filter();
  test_rec_transform();
  test_flt_sort();
  test_flt_sum();
  test_flt_transform();
  test_map_rec_set();
  test_map_rec_get();
}
