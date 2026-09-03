/* GROUP: set matrix — Fixed/Dynamic element.
 * C peer for mfb's setops.mfb plain groups (see benchmark/mfb/gen_set.py for the
 * sizes). C/Python carry only the plain element axis — Record/State are an mfb
 * value-semantics (bug-430) story. Checksums are count based (order-independent),
 * so they match mfb without matching set iteration order.
 *
 *   set (Fixed)    Set of Integer   (INT sizes)
 *   set (Dynamic)  Set of String    (STR sizes) */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "setmatrix.h"

#define SCAP 4096
#define SMASK (SCAP - 1)

static inline unsigned h64(uint64_t x) {
  x *= 0x9E3779B97F4A7C15ULL;
  return (unsigned)(x >> 40) & SMASK;
}
static inline unsigned hstr(const char *s) {
  uint64_t h = 1469598103934665603ULL;
  while (*s) { h ^= (unsigned char)*s++; h *= 1099511628211ULL; }
  return (unsigned)(h >> 40) & SMASK;
}

/* used: 0 empty, 1 live, 2 tombstone. Ops build fresh sets and never interleave
 * add-after-delete, so tombstones need no reuse-on-add handling. */

/* ---- Integer set ----------------------------------------------------- */
typedef struct { long long k[SCAP]; char used[SCAP]; int n; } ISet;
static ISet *iset_new(void) { ISet *s = malloc(sizeof(ISet)); memset(s->used, 0, SCAP); s->n = 0; return s; }
static int iset_has(ISet *s, long long k) {
  unsigned i = h64((uint64_t)k);
  while (s->used[i]) { if (s->used[i] == 1 && s->k[i] == k) return 1; i = (i + 1) & SMASK; }
  return 0;
}
static void iset_add(ISet *s, long long k) {
  unsigned i = h64((uint64_t)k);
  while (s->used[i] == 1) { if (s->k[i] == k) return; i = (i + 1) & SMASK; }
  s->used[i] = 1; s->k[i] = k; s->n++;
}
static void iset_del(ISet *s, long long k) {
  unsigned i = h64((uint64_t)k);
  while (s->used[i]) { if (s->used[i] == 1 && s->k[i] == k) { s->used[i] = 2; s->n--; return; } i = (i + 1) & SMASK; }
}

/* ---- String set ------------------------------------------------------ */
typedef struct { char *k[SCAP]; char used[SCAP]; int n; } SSet;
static SSet *sset_new(void) { SSet *s = malloc(sizeof(SSet)); memset(s->used, 0, SCAP); s->n = 0; return s; }
static int sset_has(SSet *s, const char *k) {
  unsigned i = hstr(k);
  while (s->used[i]) { if (s->used[i] == 1 && strcmp(s->k[i], k) == 0) return 1; i = (i + 1) & SMASK; }
  return 0;
}
static void sset_add(SSet *s, const char *k) {
  unsigned i = hstr(k);
  while (s->used[i] == 1) { if (strcmp(s->k[i], k) == 0) return; i = (i + 1) & SMASK; }
  s->used[i] = 1; s->k[i] = strdup(k); s->n++;
}
static void sset_del(SSet *s, const char *k) {
  unsigned i = hstr(k);
  while (s->used[i]) { if (s->used[i] == 1 && strcmp(s->k[i], k) == 0) { free(s->k[i]); s->used[i] = 2; s->n--; return; } i = (i + 1) & SMASK; }
}
static void sset_free(SSet *s) { for (int i = 0; i < SCAP; i++) if (s->used[i] == 1) free(s->k[i]); free(s); }

typedef struct { int add_n, rem_n, ro_n, alg_n, alg_sh, k_contains, k_tolist, k_alg, k_pred; } Sz;
static const Sz SZ_INT = {300, 300, 1000, 300, 150, 100, 200, 20, 300};
static const Sz SZ_STR = {400, 400, 200, 100, 50, 50, 100, 15, 60};

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

static void run_iset(const char *group, const char *pfx) {
  const Sz S = SZ_INT;
  ROW("add", { ISet *s = iset_new(); for (int i = 0; i < S.add_n; i++) iset_add(s, i); checksum = s->n; free(s); });
  ROW("remove", { ISet *s = iset_new(); for (int i = 0; i < S.rem_n; i++) iset_add(s, i); long long c = 0; for (int i = 0; i < S.rem_n; i++) { iset_del(s, i); c++; } checksum = c; free(s); });
  { ISet *b = iset_new(); for (int i = 0; i < S.ro_n; i++) iset_add(b, i);
    ROW("contains", { long long a = 0; for (int k = 0; k < S.k_contains; k++) for (int i = 0; i < S.ro_n; i++) a += iset_has(b, i) ? 1 : 0; checksum = a; });
    /* toList materializes the member list, like mfb's collections::toList. */
    ROW("toList", { long long a = 0; for (int k = 0; k < S.k_tolist; k++) { long long *xs = malloc(sizeof(long long) * b->n); int n2 = 0; for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1) xs[n2++] = b->k[s2]; a += n2; bench_opaque(xs); free(xs); } checksum = a; });
    free(b); }
  { ISet *b = iset_new(); for (int i = 0; i < S.alg_n; i++) iset_add(b, i);
    ISet *o = iset_new(); for (int i = 0; i < S.alg_n; i++) iset_add(o, i + S.alg_sh);
    ROW("toSet", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { ISet *r = iset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1) iset_add(r, b->k[s2]); a += r->n; free(r); } checksum = a; });
    ROW("union", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { ISet *r = iset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1) iset_add(r, b->k[s2]); for (int s2 = 0; s2 < SCAP; s2++) if (o->used[s2] == 1) iset_add(r, o->k[s2]); a += r->n; free(r); } checksum = a; });
    ROW("intersection", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { ISet *r = iset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1 && iset_has(o, b->k[s2])) iset_add(r, b->k[s2]); a += r->n; free(r); } checksum = a; });
    ROW("difference", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { ISet *r = iset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1 && !iset_has(o, b->k[s2])) iset_add(r, b->k[s2]); a += r->n; free(r); } checksum = a; });
    ROW("symmetricDifference", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { ISet *r = iset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1 && !iset_has(o, b->k[s2])) iset_add(r, b->k[s2]); for (int s2 = 0; s2 < SCAP; s2++) if (o->used[s2] == 1 && !iset_has(b, o->k[s2])) iset_add(r, o->k[s2]); a += r->n; free(r); } checksum = a; });
    ROW("isSubset", { long long a = 0; for (int k = 0; k < S.k_pred; k++) { int sub = 1; for (int s2 = 0; s2 < SCAP && sub; s2++) if (b->used[s2] == 1 && !iset_has(o, b->k[s2])) sub = 0; a += sub; } checksum = a; });
    ROW("isSuperset", { long long a = 0; for (int k = 0; k < S.k_pred; k++) { int sup = 1; for (int s2 = 0; s2 < SCAP && sup; s2++) if (o->used[s2] == 1 && !iset_has(b, o->k[s2])) sup = 0; a += sup; } checksum = a; });
    ROW("isDisjoint", { long long a = 0; for (int k = 0; k < S.k_pred; k++) { int dis = 1; for (int s2 = 0; s2 < SCAP && dis; s2++) if (b->used[s2] == 1 && iset_has(o, b->k[s2])) dis = 0; a += dis; } checksum = a; });
    free(b); free(o); }
}

static void run_sset(const char *group, const char *pfx) {
  const Sz S = SZ_STR;
  char buf[24];
  ROW("add", { SSet *s = sset_new(); for (int i = 0; i < S.add_n; i++) { snprintf(buf, sizeof buf, "s%d", i); sset_add(s, buf); } checksum = s->n; sset_free(s); });
  ROW("remove", { SSet *s = sset_new(); for (int i = 0; i < S.rem_n; i++) { snprintf(buf, sizeof buf, "s%d", i); sset_add(s, buf); } long long c = 0; for (int i = 0; i < S.rem_n; i++) { snprintf(buf, sizeof buf, "s%d", i); sset_del(s, buf); c++; } checksum = c; sset_free(s); });
  { SSet *b = sset_new(); for (int i = 0; i < S.ro_n; i++) { snprintf(buf, sizeof buf, "s%d", i); sset_add(b, buf); }
    ROW("contains", { long long a = 0; for (int k = 0; k < S.k_contains; k++) for (int i = 0; i < S.ro_n; i++) { snprintf(buf, sizeof buf, "s%d", i); a += sset_has(b, buf) ? 1 : 0; } checksum = a; });
    /* toList materializes the member list (copying string bytes, like mfb). */
    ROW("toList", { long long a = 0; for (int k = 0; k < S.k_tolist; k++) { char **xs = malloc(sizeof(char *) * b->n); int n2 = 0; for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1) xs[n2++] = strdup(b->k[s2]); a += n2; bench_opaque(xs); for (int j = 0; j < n2; j++) free(xs[j]); free(xs); } checksum = a; });
    sset_free(b); }
  { SSet *b = sset_new(); for (int i = 0; i < S.alg_n; i++) { snprintf(buf, sizeof buf, "s%d", i); sset_add(b, buf); }
    SSet *o = sset_new(); for (int i = 0; i < S.alg_n; i++) { snprintf(buf, sizeof buf, "s%d", i + S.alg_sh); sset_add(o, buf); }
    ROW("toSet", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { SSet *r = sset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1) sset_add(r, b->k[s2]); a += r->n; sset_free(r); } checksum = a; });
    ROW("union", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { SSet *r = sset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1) sset_add(r, b->k[s2]); for (int s2 = 0; s2 < SCAP; s2++) if (o->used[s2] == 1) sset_add(r, o->k[s2]); a += r->n; sset_free(r); } checksum = a; });
    ROW("intersection", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { SSet *r = sset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1 && sset_has(o, b->k[s2])) sset_add(r, b->k[s2]); a += r->n; sset_free(r); } checksum = a; });
    ROW("difference", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { SSet *r = sset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1 && !sset_has(o, b->k[s2])) sset_add(r, b->k[s2]); a += r->n; sset_free(r); } checksum = a; });
    ROW("symmetricDifference", { long long a = 0; for (int k = 0; k < S.k_alg; k++) { SSet *r = sset_new(); for (int s2 = 0; s2 < SCAP; s2++) if (b->used[s2] == 1 && !sset_has(o, b->k[s2])) sset_add(r, b->k[s2]); for (int s2 = 0; s2 < SCAP; s2++) if (o->used[s2] == 1 && !sset_has(b, o->k[s2])) sset_add(r, o->k[s2]); a += r->n; sset_free(r); } checksum = a; });
    ROW("isSubset", { long long a = 0; for (int k = 0; k < S.k_pred; k++) { int sub = 1; for (int s2 = 0; s2 < SCAP && sub; s2++) if (b->used[s2] == 1 && !sset_has(o, b->k[s2])) sub = 0; a += sub; } checksum = a; });
    ROW("isSuperset", { long long a = 0; for (int k = 0; k < S.k_pred; k++) { int sup = 1; for (int s2 = 0; s2 < SCAP && sup; s2++) if (o->used[s2] == 1 && !sset_has(b, o->k[s2])) sup = 0; a += sup; } checksum = a; });
    ROW("isDisjoint", { long long a = 0; for (int k = 0; k < S.k_pred; k++) { int dis = 1; for (int s2 = 0; s2 < SCAP && dis; s2++) if (b->used[s2] == 1 && sset_has(o, b->k[s2])) dis = 0; a += dis; } checksum = a; });
    sset_free(b); sset_free(o); }
}

void run_setmatrix_group(void) {
  run_iset("set (Fixed)", "sf");
  run_sset("set (Dynamic)", "sd");
}
