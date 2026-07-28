/* GROUP: set (collections:: over `Set OF T`, plan-63)
 *
 * The C oracle for benchmark/mfb/src/setops.mfb. A `Set OF Integer` is modelled
 * as an open-addressing integer hash set (the same probe idiom the map peer
 * uses in mapbench.c), so `add`/`contains` are O(1)-average and the set-algebra
 * members are built by iterating members and probing the other set. Only set
 * semantics — dedup, membership, and the algebra results — determine the
 * checksums, so this reproduces the mfb runtime's 20000 (`build`) and 6006
 * (`ops`) exactly. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "setopsbench.h"

typedef struct {
  long *keys;
  unsigned char *used;
  int cap; /* power of two */
  int len;
} IntSet;

static size_t iset_slot(long k, int cap) {
  return (size_t)((unsigned long)k * 1099511628211UL) & (size_t)(cap - 1);
}

static IntSet *iset_new(int cap) {
  IntSet *s = malloc(sizeof(IntSet));
  s->cap = cap;
  s->len = 0;
  s->keys = malloc((size_t)cap * sizeof(long));
  s->used = calloc((size_t)cap, 1);
  return s;
}

static void iset_free(IntSet *s) {
  free(s->keys);
  free(s->used);
  free(s);
}

static void iset_insert_raw(IntSet *s, long k) {
  size_t h = iset_slot(k, s->cap);
  while (s->used[h]) {
    if (s->keys[h] == k) return;
    h = (h + 1) & (size_t)(s->cap - 1);
  }
  s->used[h] = 1;
  s->keys[h] = k;
  s->len++;
}

static void iset_grow(IntSet *s) {
  int oldcap = s->cap;
  long *oldkeys = s->keys;
  unsigned char *oldused = s->used;
  s->cap = oldcap * 2;
  s->keys = malloc((size_t)s->cap * sizeof(long));
  s->used = calloc((size_t)s->cap, 1);
  s->len = 0;
  for (int i = 0; i < oldcap; i++)
    if (oldused[i]) iset_insert_raw(s, oldkeys[i]);
  free(oldkeys);
  free(oldused);
}

/* add — idempotent insert, matching collections::add on a Set. */
static void iset_add(IntSet *s, long k) {
  if ((s->len + 1) * 4 >= s->cap * 3) iset_grow(s);
  iset_insert_raw(s, k);
}

static int iset_contains(const IntSet *s, long k) {
  size_t h = iset_slot(k, s->cap);
  while (s->used[h]) {
    if (s->keys[h] == k) return 1;
    h = (h + 1) & (size_t)(s->cap - 1);
  }
  return 0;
}

/* remove — return a fresh set holding every element of s except k. */
static IntSet *iset_remove(const IntSet *s, long k) {
  IntSet *out = iset_new(s->cap);
  for (int i = 0; i < s->cap; i++)
    if (s->used[i] && s->keys[i] != k) iset_add(out, s->keys[i]);
  return out;
}

static IntSet *iset_union(const IntSet *a, const IntSet *b) {
  IntSet *out = iset_new(a->cap > b->cap ? a->cap : b->cap);
  for (int i = 0; i < a->cap; i++) if (a->used[i]) iset_add(out, a->keys[i]);
  for (int i = 0; i < b->cap; i++) if (b->used[i]) iset_add(out, b->keys[i]);
  return out;
}

static IntSet *iset_intersection(const IntSet *a, const IntSet *b) {
  IntSet *out = iset_new(a->cap);
  for (int i = 0; i < a->cap; i++)
    if (a->used[i] && iset_contains(b, a->keys[i])) iset_add(out, a->keys[i]);
  return out;
}

/* difference — a \ b. */
static IntSet *iset_difference(const IntSet *a, const IntSet *b) {
  IntSet *out = iset_new(a->cap);
  for (int i = 0; i < a->cap; i++)
    if (a->used[i] && !iset_contains(b, a->keys[i])) iset_add(out, a->keys[i]);
  return out;
}

/* symmetricDifference — (a \ b) union (b \ a). */
static IntSet *iset_symdiff(const IntSet *a, const IntSet *b) {
  IntSet *out = iset_new(a->cap > b->cap ? a->cap : b->cap);
  for (int i = 0; i < a->cap; i++)
    if (a->used[i] && !iset_contains(b, a->keys[i])) iset_add(out, a->keys[i]);
  for (int i = 0; i < b->cap; i++)
    if (b->used[i] && !iset_contains(a, b->keys[i])) iset_add(out, b->keys[i]);
  return out;
}

static int iset_is_subset(const IntSet *a, const IntSet *b) {
  for (int i = 0; i < a->cap; i++)
    if (a->used[i] && !iset_contains(b, a->keys[i])) return 0;
  return 1;
}

static int iset_is_disjoint(const IntSet *a, const IntSet *b) {
  for (int i = 0; i < a->cap; i++)
    if (a->used[i] && iset_contains(b, a->keys[i])) return 0;
  return 1;
}

/* toList + toSet round-trip: collect members then rebuild a fresh set. */
static IntSet *iset_tolist_toset(const IntSet *s) {
  long *list = malloc((size_t)s->len * sizeof(long));
  int n = 0;
  for (int i = 0; i < s->cap; i++)
    if (s->used[i]) list[n++] = s->keys[i];
  IntSet *out = iset_new(s->cap);
  for (int i = 0; i < n; i++) iset_add(out, list[i]);
  free(list);
  return out;
}

/* build — grow a set by repeated add (half the inserts are duplicates so the
 * idempotent-hit path is exercised), then sum a membership sweep. */
static void test_set_build(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    IntSet *s = iset_new(1024);
    for (int i = 0; i < 20000; i++) iset_add(s, i / 2);
    long hits = 0;
    for (int i = 0; i < 20000; i++)
      if (iset_contains(s, i)) hits++;
    checksum = s->len + hits;
    t[r] = now_ns() - t0;
    iset_free(s);
  }
  fprintf(stderr, "set_build = %ld\n", checksum);
  record("set", "build", t, RUN);
  free(t);
}

/* ops — one coverage row over the entire Set surface on two moderate sets. */
static void test_set_ops(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    IntSet *a = iset_new(2048);
    IntSet *b = iset_new(2048);
    for (int i = 0; i < 1000; i++) {
      iset_add(a, i);
      iset_add(b, i + 500);
    }
    IntSet *u = iset_union(a, b);
    IntSet *inter = iset_intersection(a, b);
    IntSet *diff = iset_difference(a, b);
    IntSet *sym = iset_symdiff(a, b);
    IntSet *without_one = iset_remove(a, 0);
    IntSet *from_list = iset_tolist_toset(u);
    long flags = 0;
    if (iset_is_subset(inter, a)) flags += 1;
    if (iset_is_subset(a, u)) flags += 2; /* isSuperset(u, a) == isSubset(a, u) */
    if (iset_is_disjoint(diff, b)) flags += 4;
    checksum = u->len + inter->len + diff->len + sym->len + without_one->len +
               from_list->len + flags;
    t[r] = now_ns() - t0;
    iset_free(a);
    iset_free(b);
    iset_free(u);
    iset_free(inter);
    iset_free(diff);
    iset_free(sym);
    iset_free(without_one);
    iset_free(from_list);
  }
  fprintf(stderr, "set_ops = %ld\n", checksum);
  record("set", "ops", t, RUN);
  free(t);
}

void run_setops_group(void) {
  test_set_build();
  test_set_ops();
}
