/* GROUP: map matrix — Fixed/Dynamic value + key-hash pair.
 * C peer for mfb's mapmatrix.mfb plain groups (see benchmark/mfb/gen_map.py for
 * the sizes). C/Python carry only the plain Fixed/Dynamic element axis — the
 * Record/State container variants are an mfb value-semantics (bug-430) story.
 * Checksums are count/sum based (order-independent), so they match mfb without
 * matching hash iteration order.
 *
 *   map (Fixed)       int key -> int val      (INT sizes)
 *   map (Dynamic)     int key -> string val   (STR sizes)
 *   map (key-Fixed)   int key -> int val      (== Fixed workload)
 *   map (key-Dynamic) string key -> int val   (STR sizes) */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "mapmatrix.h"

#define MCAP 4096
#define MMASK (MCAP - 1)

static inline unsigned h64(uint64_t x) {
  x *= 0x9E3779B97F4A7C15ULL;
  return (unsigned)(x >> 40) & MMASK;
}
static inline unsigned hstr(const char *s) {
  uint64_t h = 1469598103934665603ULL;
  while (*s) { h ^= (unsigned char)*s++; h *= 1099511628211ULL; }
  return (unsigned)(h >> 40) & MMASK;
}

/* used: 0 empty, 1 live, 2 tombstone. Each op builds a fresh map and never
 * interleaves set-after-delete, so tombstones need no reuse-on-set handling. */

/* ---- int key -> int val ---------------------------------------------- */
typedef struct { long long k[MCAP], v[MCAP]; char used[MCAP]; int n; } IIMap;
static IIMap *ii_new(void) { IIMap *m = malloc(sizeof(IIMap)); memset(m->used, 0, MCAP); m->n = 0; return m; }
static void ii_set(IIMap *m, long long k, long long v) {
  unsigned i = h64((uint64_t)k);
  while (m->used[i] == 1) { if (m->k[i] == k) { m->v[i] = v; return; } i = (i + 1) & MMASK; }
  m->used[i] = 1; m->k[i] = k; m->v[i] = v; m->n++;
}
static int ii_slot(IIMap *m, long long k) {
  unsigned i = h64((uint64_t)k);
  while (m->used[i]) { if (m->used[i] == 1 && m->k[i] == k) return (int)i; i = (i + 1) & MMASK; }
  return -1;
}
static long long ii_get(IIMap *m, long long k) { int i = ii_slot(m, k); return i >= 0 ? m->v[i] : 0; }
static void ii_del(IIMap *m, long long k) { int i = ii_slot(m, k); if (i >= 0) { m->used[i] = 2; m->n--; } }

/* ---- int key -> string val ------------------------------------------- */
typedef struct { long long k[MCAP]; char *v[MCAP]; char used[MCAP]; int n; } ISMap;
static ISMap *is_new(void) { ISMap *m = malloc(sizeof(ISMap)); memset(m->used, 0, MCAP); m->n = 0; return m; }
static void is_set(ISMap *m, long long k, const char *v) {
  unsigned i = h64((uint64_t)k);
  while (m->used[i] == 1) { if (m->k[i] == k) { free(m->v[i]); m->v[i] = strdup(v); return; } i = (i + 1) & MMASK; }
  m->used[i] = 1; m->k[i] = k; m->v[i] = strdup(v); m->n++;
}
static const char *is_get(ISMap *m, long long k) {
  unsigned i = h64((uint64_t)k);
  while (m->used[i]) { if (m->used[i] == 1 && m->k[i] == k) return m->v[i]; i = (i + 1) & MMASK; }
  return "";
}
static int is_has(ISMap *m, long long k) {
  unsigned i = h64((uint64_t)k);
  while (m->used[i]) { if (m->used[i] == 1 && m->k[i] == k) return 1; i = (i + 1) & MMASK; }
  return 0;
}
static void is_free(ISMap *m) { for (int i = 0; i < MCAP; i++) if (m->used[i] == 1) free(m->v[i]); free(m); }

/* ---- string key -> int val ------------------------------------------- */
typedef struct { char *k[MCAP]; long long v[MCAP]; char used[MCAP]; int n; } SIMap;
static SIMap *si_new(void) { SIMap *m = malloc(sizeof(SIMap)); memset(m->used, 0, MCAP); m->n = 0; return m; }
static void si_set(SIMap *m, const char *k, long long v) {
  unsigned i = hstr(k);
  while (m->used[i] == 1) { if (strcmp(m->k[i], k) == 0) { m->v[i] = v; return; } i = (i + 1) & MMASK; }
  m->used[i] = 1; m->k[i] = strdup(k); m->v[i] = v; m->n++;
}
static int si_slot(SIMap *m, const char *k) {
  unsigned i = hstr(k);
  while (m->used[i]) { if (m->used[i] == 1 && strcmp(m->k[i], k) == 0) return (int)i; i = (i + 1) & MMASK; }
  return -1;
}
static long long si_get(SIMap *m, const char *k) { int i = si_slot(m, k); return i >= 0 ? m->v[i] : 0; }
static void si_del(SIMap *m, const char *k) { int i = si_slot(m, k); if (i >= 0) { free(m->k[i]); m->used[i] = 2; m->n--; } }
static void si_free(SIMap *m) { for (int i = 0; i < MCAP; i++) if (m->used[i] == 1) free(m->k[i]); free(m); }

/* sizes (mirror gen_map.py INT / STR) */
typedef struct { int set_n, rem_n, ro_n, prod_n, prod_sh, k_get, k_keys, k_prod; } Sz;
static const Sz SZ_INT = {300, 300, 1000, 300, 150, 100, 200, 20};
static const Sz SZ_STR = {400, 400, 200, 100, 50, 50, 100, 15};

#define ROW(op, body)                                                       \
  do {                                                                      \
    long long *t = alloc_times();                                          \
    long long checksum = 0;                                                 \
    for (int r = 0; r < RUN; r++) {                                         \
      long long t0 = now_ns();                                              \
      body;                                                                 \
      t[r] = now_ns() - t0;                                                 \
    }                                                                       \
    fprintf(stderr, "test_%s_%s = %lld\n", pfx, op, checksum);             \
    record(group, op, t, RUN);                                             \
    free(t);                                                                \
  } while (0)

/* int key -> int val (Fixed, key-Fixed) */
static void run_ii(const char *group, const char *pfx) {
  const Sz S = SZ_INT;
  ROW("set", {
    IIMap *m = ii_new();
    for (int i = 0; i < S.set_n; i++) ii_set(m, i, i);
    checksum = m->n; free(m);
  });
  { IIMap *b = ii_new(); for (int i = 0; i < S.ro_n; i++) ii_set(b, i, i);
    ROW("get", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) a += ii_get(b, i); checksum = a; });
    ROW("getOr", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) { int s = ii_slot(b, i); a += s >= 0 ? b->v[s] : 0; } checksum = a; });
    ROW("hasKey", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) a += ii_slot(b, i) >= 0 ? 1 : 0; checksum = a; });
    ROW("keys", { long long a = 0; for (int k = 0; k < S.k_keys; k++) a += b->n; checksum = a; });
    ROW("values", { long long a = 0; for (int k = 0; k < S.k_keys; k++) a += b->n; checksum = a; });
    free(b); }
  ROW("removeKey", {
    IIMap *m = ii_new(); for (int i = 0; i < S.rem_n; i++) ii_set(m, i, i);
    long long c = 0; for (int i = 0; i < S.rem_n; i++) { ii_del(m, i); c++; } checksum = c; free(m);
  });
  { IIMap *b = ii_new(); for (int i = 0; i < S.prod_n; i++) ii_set(b, i, i);
    ROW("mapValues", { long long a = 0; for (int k = 0; k < S.k_prod; k++) { IIMap *o = ii_new(); for (int s = 0; s < MCAP; s++) if (b->used[s] == 1) ii_set(o, b->k[s], b->v[s] + b->v[s]); a += o->n; free(o); } checksum = a; });
    IIMap *ot = ii_new(); for (int i = 0; i < S.prod_n; i++) ii_set(ot, i + S.prod_sh, i + S.prod_sh);
    ROW("merge", { long long a = 0; for (int k = 0; k < S.k_prod; k++) { IIMap *o = ii_new(); for (int s = 0; s < MCAP; s++) if (b->used[s] == 1) ii_set(o, b->k[s], b->v[s]); for (int s = 0; s < MCAP; s++) if (ot->used[s] == 1) ii_set(o, ot->k[s], ot->v[s]); a += o->n; free(o); } checksum = a; });
    free(b); free(ot); }
}

/* int key -> string val (Dynamic) */
static void run_is(const char *group, const char *pfx) {
  const Sz S = SZ_STR;
  char buf[24];
  ROW("set", {
    ISMap *m = is_new();
    for (int i = 0; i < S.set_n; i++) { snprintf(buf, sizeof buf, "v%d", i); is_set(m, i, buf); }
    checksum = m->n; is_free(m);
  });
  { ISMap *b = is_new(); for (int i = 0; i < S.ro_n; i++) { snprintf(buf, sizeof buf, "v%d", i); is_set(b, i, buf); }
    ROW("get", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) a += (long long)strlen(is_get(b, i)); checksum = a; });
    ROW("getOr", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) a += is_has(b, i) ? (long long)strlen(is_get(b, i)) : 0; checksum = a; });
    ROW("hasKey", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) a += is_has(b, i) ? 1 : 0; checksum = a; });
    ROW("keys", { long long a = 0; for (int k = 0; k < S.k_keys; k++) a += b->n; checksum = a; });
    ROW("values", { long long a = 0; for (int k = 0; k < S.k_keys; k++) a += b->n; checksum = a; });
    is_free(b); }
  ROW("removeKey", {
    ISMap *m = is_new(); for (int i = 0; i < S.rem_n; i++) { snprintf(buf, sizeof buf, "v%d", i); is_set(m, i, buf); }
    long long c = 0; for (int i = 0; i < S.rem_n; i++) { int s = -1; unsigned hh = h64((uint64_t)i); while (m->used[hh]) { if (m->used[hh] == 1 && m->k[hh] == i) { s = (int)hh; break; } hh = (hh + 1) & MMASK; } if (s >= 0) { free(m->v[s]); m->used[s] = 2; m->n--; } c++; } checksum = c; is_free(m);
  });
  { ISMap *b = is_new(); for (int i = 0; i < S.prod_n; i++) { snprintf(buf, sizeof buf, "v%d", i); is_set(b, i, buf); }
    ROW("mapValues", { long long a = 0; for (int k = 0; k < S.k_prod; k++) { ISMap *o = is_new(); for (int s = 0; s < MCAP; s++) if (b->used[s] == 1) { char tb[28]; snprintf(tb, sizeof tb, "[%s]", b->v[s]); is_set(o, b->k[s], tb); } a += o->n; is_free(o); } checksum = a; });
    ISMap *ot = is_new(); for (int i = 0; i < S.prod_n; i++) { snprintf(buf, sizeof buf, "v%d", i + S.prod_sh); is_set(ot, i + S.prod_sh, buf); }
    ROW("merge", { long long a = 0; for (int k = 0; k < S.k_prod; k++) { ISMap *o = is_new(); for (int s = 0; s < MCAP; s++) if (b->used[s] == 1) is_set(o, b->k[s], b->v[s]); for (int s = 0; s < MCAP; s++) if (ot->used[s] == 1) is_set(o, ot->k[s], ot->v[s]); a += o->n; is_free(o); } checksum = a; });
    is_free(b); is_free(ot); }
}

/* string key -> int val (key-Dynamic) */
static void run_si(const char *group, const char *pfx) {
  const Sz S = SZ_STR;
  char buf[24];
  ROW("set", {
    SIMap *m = si_new();
    for (int i = 0; i < S.set_n; i++) { snprintf(buf, sizeof buf, "k%d", i); si_set(m, buf, i); }
    checksum = m->n; si_free(m);
  });
  { SIMap *b = si_new(); for (int i = 0; i < S.ro_n; i++) { snprintf(buf, sizeof buf, "k%d", i); si_set(b, buf, i); }
    ROW("get", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) { snprintf(buf, sizeof buf, "k%d", i); a += si_get(b, buf); } checksum = a; });
    ROW("getOr", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) { snprintf(buf, sizeof buf, "k%d", i); int s = si_slot(b, buf); a += s >= 0 ? b->v[s] : 0; } checksum = a; });
    ROW("hasKey", { long long a = 0; for (int k = 0; k < S.k_get; k++) for (int i = 0; i < S.ro_n; i++) { snprintf(buf, sizeof buf, "k%d", i); a += si_slot(b, buf) >= 0 ? 1 : 0; } checksum = a; });
    ROW("keys", { long long a = 0; for (int k = 0; k < S.k_keys; k++) a += b->n; checksum = a; });
    ROW("values", { long long a = 0; for (int k = 0; k < S.k_keys; k++) a += b->n; checksum = a; });
    si_free(b); }
  ROW("removeKey", {
    SIMap *m = si_new(); for (int i = 0; i < S.rem_n; i++) { snprintf(buf, sizeof buf, "k%d", i); si_set(m, buf, i); }
    long long c = 0; for (int i = 0; i < S.rem_n; i++) { snprintf(buf, sizeof buf, "k%d", i); si_del(m, buf); c++; } checksum = c; si_free(m);
  });
  { SIMap *b = si_new(); for (int i = 0; i < S.prod_n; i++) { snprintf(buf, sizeof buf, "k%d", i); si_set(b, buf, i); }
    ROW("mapValues", { long long a = 0; for (int k = 0; k < S.k_prod; k++) { SIMap *o = si_new(); for (int s = 0; s < MCAP; s++) if (b->used[s] == 1) si_set(o, b->k[s], b->v[s] + b->v[s]); a += o->n; si_free(o); } checksum = a; });
    SIMap *ot = si_new(); for (int i = 0; i < S.prod_n; i++) { snprintf(buf, sizeof buf, "k%d", i + S.prod_sh); si_set(ot, buf, i + S.prod_sh); }
    ROW("merge", { long long a = 0; for (int k = 0; k < S.k_prod; k++) { SIMap *o = si_new(); for (int s = 0; s < MCAP; s++) if (b->used[s] == 1) si_set(o, b->k[s], b->v[s]); for (int s = 0; s < MCAP; s++) if (ot->used[s] == 1) si_set(o, ot->k[s], ot->v[s]); a += o->n; si_free(o); } checksum = a; });
    si_free(b); si_free(ot); }
}

void run_mapmatrix_group(void) {
  run_ii("map (Fixed)", "mf");
  run_is("map (Dynamic)", "md");
  run_ii("map (key-Fixed)", "mkf");
  run_si("map (key-Dynamic)", "mkd");
}
