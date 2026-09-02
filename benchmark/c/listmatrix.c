/* GROUP: list matrix — Fixed/Dynamic element.
 * C peer for mfb's list.mfb plain groups (see benchmark/mfb/gen_list.py). Same
 * sizes for both element types; only the element differs (int vs "s"+i). C/Python
 * carry only the plain element axis — Record/State are an mfb value-semantics
 * (bug-430) story. Replaces the old `list` + `liststr` groups. Most checksums are
 * count/sum based; the sort adaptivity rows use the same polynomial hash as mfb. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "listmatrix.h"

/* ---- growable lists -------------------------------------------------- */
typedef struct { long long *a; int n, cap; } IL;
static IL *il_new(int cap) { if (cap < 1) cap = 1; IL *l = malloc(sizeof(IL)); l->a = malloc(sizeof(long long) * cap); l->n = 0; l->cap = cap; return l; }
static void il_push(IL *l, long long v) { if (l->n == l->cap) { l->cap *= 2; l->a = realloc(l->a, sizeof(long long) * l->cap); } l->a[l->n++] = v; }
static void il_ins(IL *l, int idx, long long v) { if (l->n == l->cap) { l->cap *= 2; l->a = realloc(l->a, sizeof(long long) * l->cap); } memmove(&l->a[idx + 1], &l->a[idx], sizeof(long long) * (l->n - idx)); l->a[idx] = v; l->n++; }
static void il_free(IL *l) { free(l->a); free(l); }

typedef struct { char **a; int n, cap; } SL;
static SL *sl_new(int cap) { if (cap < 1) cap = 1; SL *l = malloc(sizeof(SL)); l->a = malloc(sizeof(char *) * cap); l->n = 0; l->cap = cap; return l; }
static void sl_push(SL *l, const char *s) { if (l->n == l->cap) { l->cap *= 2; l->a = realloc(l->a, sizeof(char *) * l->cap); } l->a[l->n++] = strdup(s); }
static void sl_ins(SL *l, int idx, const char *s) { if (l->n == l->cap) { l->cap *= 2; l->a = realloc(l->a, sizeof(char *) * l->cap); } memmove(&l->a[idx + 1], &l->a[idx], sizeof(char *) * (l->n - idx)); l->a[idx] = strdup(s); l->n++; }
static void sl_free(SL *l) { for (int i = 0; i < l->n; i++) free(l->a[i]); free(l->a); free(l); }

static int cmp_ll(const void *a, const void *b) { long long x = *(const long long *)a, y = *(const long long *)b; return x < y ? -1 : x > y ? 1 : 0; }
static int cmp_str(const void *a, const void *b) { return strcmp(*(char *const *)a, *(char *const *)b); }

static int uniq_ll(const long long *a, int n) {
  long long *c = malloc(sizeof(long long) * n); memcpy(c, a, sizeof(long long) * n);
  qsort(c, n, sizeof(long long), cmp_ll);
  int u = n ? 1 : 0; for (int i = 1; i < n; i++) if (c[i] != c[i - 1]) u++;
  free(c); return u;
}
static int uniq_str(char **a, int n) {
  char **c = malloc(sizeof(char *) * n); memcpy(c, a, sizeof(char *) * n);
  qsort(c, n, sizeof(char *), cmp_str);
  int u = n ? 1 : 0; for (int i = 1; i < n; i++) if (strcmp(c[i], c[i - 1]) != 0) u++;
  free(c); return u;
}

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

/* ===================== Integer (Fixed) =============================== */
static void run_ilist(const char *group, const char *pfx) {
  IL *base = il_new(1000); for (int i = 0; i < 1000; i++) il_push(base, i);
  IL *pos = il_new(1000); for (int i = 1; i <= 1000; i++) il_push(pos, i);
  IL *neg = il_new(1000); for (int i = 1; i <= 1000; i++) il_push(neg, -i);

  ROW("append", { IL *l = il_new(4); for (int i = 0; i < 1000; i++) il_push(l, i); checksum = l->n; il_free(l); });
  ROW("append_batch", { IL *l = il_new(4); for (int b = 0; b < 100; b++) for (int i = 0; i < 10; i++) il_push(l, i); checksum = l->n; il_free(l); });
  ROW("prepend", { IL *l = il_new(4); for (int i = 0; i < 1000; i++) il_ins(l, 0, i); checksum = l->n; il_free(l); });
  ROW("copy", { long long a = 0; for (int k = 0; k < 1000; k++) { IL *c = il_new(base->n); for (int i = 0; i < base->n; i++) il_push(c, base->a[i]); a += c->n; il_free(c); } checksum = a; });
  ROW("distinct", { IL *d = il_new(5000); for (int i = 0; i < 5000; i++) il_push(d, i % 1000); checksum = uniq_ll(d->a, d->n); il_free(d); });
  ROW("groupby", { IL *g = il_new(2000); for (int i = 0; i < 2000; i++) il_push(g, i % 100); checksum = uniq_ll(g->a, g->n); il_free(g); });
  ROW("set", { long long v[200]; for (int i = 0; i < 200; i++) v[i] = i; for (int p = 0; p < 10; p++) for (int j = 0; j < 200; j++) v[j] = v[j] + 1; long long s = 0; for (int j = 0; j < 200; j++) s += v[j]; checksum = s; });
  ROW("sort", { long long c[50]; for (int i = 0; i < 50; i++) c[i] = (i * 7919) % 50; qsort(c, 50, sizeof(long long), cmp_ll); checksum = 50; });
  { IL *a = il_new(20000), *d = il_new(20000), *s = il_new(20000);
    for (int i = 0; i < 20000; i++) { il_push(a, i); il_push(d, 19999 - i); il_push(s, (long long)i * 7919 % 20000); }
    long long *tmp = malloc(sizeof(long long) * 20000);
    #define SORTHASH(src) { memcpy(tmp, (src)->a, sizeof(long long) * 20000); qsort(tmp, 20000, sizeof(long long), cmp_ll); long long h = 0; for (int i = 0; i < 20000; i++) h = (h * 31 + tmp[i]) % 1000000007; checksum = h; }
    ROW("sort_asc", SORTHASH(a));
    ROW("sort_desc", SORTHASH(d));
    ROW("sort_rand", SORTHASH(s));
    #undef SORTHASH
    free(tmp); il_free(a); il_free(d); il_free(s); }
  ROW("all", { long long a = 0; for (int k = 0; k < 200; k++) { int ok = 1; for (int i = 0; i < pos->n && ok; i++) if (!(pos->a[i] > 0)) ok = 0; a += ok; } checksum = a; });
  ROW("any", { long long a = 0; for (int k = 0; k < 200; k++) { int any = 0; for (int i = 0; i < neg->n && !any; i++) if (neg->a[i] > 0) any = 1; a += !any; } checksum = a; });
  ROW("chunks", { long long a = 0; for (int k = 0; k < 200; k++) a += (base->n + 9) / 10; checksum = a; });
  ROW("contains", { long long a = 0; for (int k = 0; k < 500; k++) { int f = 0; for (int i = 0; i < base->n && !f; i++) if (base->a[i] == 1000) f = 1; a += !f; } checksum = a; });
  ROW("drop", { long long a = 0; for (int k = 0; k < 500; k++) a += base->n - 500; checksum = a; });
  ROW("filter", { long long a = 0; for (int k = 0; k < 200; k++) { int c = 0; for (int i = 0; i < base->n; i++) if (base->a[i] % 2 == 0) c++; a += c; } checksum = a; });
  ROW("find", { long long a = 0; for (int k = 0; k < 500; k++) { int idx = -1; for (int i = 0; i < base->n; i++) if (base->a[i] == 999) { idx = i; break; } a += idx; } checksum = a; });
  ROW("findIndex", { long long a = 0; for (int k = 0; k < 500; k++) { int idx = -1; for (int i = 0; i < base->n; i++) if (base->a[i] >= 999) { idx = i; break; } a += idx; } checksum = a; });
  ROW("findLastIndex", { long long a = 0; for (int k = 0; k < 500; k++) { int idx = -1; for (int i = base->n - 1; i >= 0; i--) if (base->a[i] <= 5) { idx = i; break; } a += idx; } checksum = a; });
  ROW("flatten", { long long a = 0; for (int k = 0; k < 200; k++) a += 100 * 10; checksum = a; });
  ROW("forEach", { long long a = 0; for (int k = 0; k < 200; k++) for (int i = 0; i < base->n; i++) a += base->a[i]; checksum = a; });
  ROW("get", { long long a = 0; for (int k = 0; k < 100; k++) for (int i = 0; i < 1000; i++) a += base->a[i]; checksum = a; });
  ROW("getOr", { long long a = 0; for (int k = 0; k < 100; k++) for (int i = 0; i < 1000; i++) a += base->a[i]; checksum = a; });
  /* insert accumulates like append/prepend, but gen_list.py's OP_ORDER emits it
   * here (alphabetically, between getOr and mid) — keep the row order in step. */
  ROW("insert", { IL *l = il_new(4); for (int i = 0; i < 1000; i++) il_ins(l, l->n / 2, i); checksum = l->n; il_free(l); });
  ROW("mid", { long long a = 0; for (int k = 0; k < 500; k++) a += 500; checksum = a; });
  ROW("partition", { long long a = 0; for (int k = 0; k < 200; k++) { int c = 0; for (int i = 0; i < base->n; i++) if (base->a[i] % 2 == 0) c++; a += c; } checksum = a; });
  ROW("reduce", { long long a = 0; for (int k = 0; k < 500; k++) { long long s = 0; for (int i = 0; i < base->n; i++) s += base->a[i]; a += s; } checksum = a; });
  ROW("reduceRight", { long long a = 0; for (int k = 0; k < 500; k++) { long long s = 0; for (int i = base->n - 1; i >= 0; i--) s += base->a[i]; a += s; } checksum = a; });
  ROW("removeAt", { IL *l = il_new(base->n); for (int i = 0; i < base->n; i++) il_push(l, base->a[i]); long long c = 0; while (l->n > 0) { memmove(&l->a[0], &l->a[1], sizeof(long long) * (l->n - 1)); l->n--; c++; } checksum = c; il_free(l); });
  ROW("replace", { long long a = 0; for (int k = 0; k < 200; k++) a += base->n; checksum = a; });
  { IL *b5 = il_new(500); for (int i = 0; i < 500; i++) il_push(b5, i);
    ROW("sortBy", { long long a = 0; for (int k = 0; k < 200; k++) { long long *c = malloc(sizeof(long long) * 500); for (int i = 0; i < 500; i++) c[i] = -b5->a[i]; qsort(c, 500, sizeof(long long), cmp_ll); a += -c[0]; free(c); } checksum = a; });
    il_free(b5); }
  ROW("sum", { long long a = 0; for (int k = 0; k < 1000; k++) { long long s = 0; for (int i = 0; i < base->n; i++) s += base->a[i]; a += s; } checksum = a; });
  ROW("take", { long long a = 0; for (int k = 0; k < 500; k++) a += 500; checksum = a; });
  ROW("transform", { long long a = 0; for (int k = 0; k < 200; k++) a += base->n; checksum = a; });
  ROW("window", { long long a = 0; for (int k = 0; k < 100; k++) a += base->n - 9; checksum = a; });
  ROW("zip", { long long a = 0; for (int k = 0; k < 100; k++) a += base->n; checksum = a; });

  il_free(base); il_free(pos); il_free(neg);
}

/* ===================== String (Dynamic) ============================== */
static void run_slist(const char *group, const char *pfx) {
  char b[24];
  SL *base = sl_new(1000); for (int i = 0; i < 1000; i++) { snprintf(b, sizeof b, "s%d", i); sl_push(base, b); }
  SL *pos = base;  /* strNonEmpty over base — all non-empty */
  SL *neg = base;  /* NOT any(base, strNonEmpty) == 0 */

  ROW("append", { SL *l = sl_new(4); for (int i = 0; i < 1000; i++) { snprintf(b, sizeof b, "s%d", i); sl_push(l, b); } checksum = l->n; sl_free(l); });
  ROW("append_batch", { SL *l = sl_new(4); for (int bb = 0; bb < 100; bb++) for (int i = 0; i < 10; i++) { snprintf(b, sizeof b, "a%d", i); sl_push(l, b); } checksum = l->n; sl_free(l); });
  ROW("prepend", { SL *l = sl_new(4); for (int i = 0; i < 1000; i++) { snprintf(b, sizeof b, "s%d", i); sl_ins(l, 0, b); } checksum = l->n; sl_free(l); });
  ROW("copy", { long long a = 0; for (int k = 0; k < 1000; k++) { SL *c = sl_new(base->n); for (int i = 0; i < base->n; i++) sl_push(c, base->a[i]); a += c->n; sl_free(c); } checksum = a; });
  ROW("distinct", { SL *d = sl_new(5000); for (int i = 0; i < 5000; i++) { snprintf(b, sizeof b, "d%d", i % 1000); sl_push(d, b); } checksum = uniq_str(d->a, d->n); sl_free(d); });
  ROW("groupby", { SL *g = sl_new(2000); for (int i = 0; i < 2000; i++) { snprintf(b, sizeof b, "s%d", i); sl_push(g, b); } long long *lens = malloc(sizeof(long long) * g->n); for (int i = 0; i < g->n; i++) lens[i] = (long long)strlen(g->a[i]); checksum = uniq_ll(lens, g->n); free(lens); sl_free(g); });
  ROW("set", { char *v[200]; for (int i = 0; i < 200; i++) { snprintf(b, sizeof b, "s%d", i); v[i] = strdup(b); } for (int p = 0; p < 10; p++) for (int j = 0; j < 200; j++) { size_t L = strlen(v[j]); v[j] = realloc(v[j], L + 2); v[j][L] = '!'; v[j][L + 1] = 0; } long long s = 0; for (int j = 0; j < 200; j++) { s += (long long)strlen(v[j]); free(v[j]); } checksum = s; });
  ROW("sort", { char *c[50]; for (int i = 0; i < 50; i++) { snprintf(b, sizeof b, "s%d", i); c[i] = strdup(b); } qsort(c, 50, sizeof(char *), cmp_str); for (int i = 0; i < 50; i++) free(c[i]); checksum = 50; });
  { SL *a = sl_new(20000), *d = sl_new(20000), *s = sl_new(20000);
    for (int i = 0; i < 20000; i++) { snprintf(b, sizeof b, "%06d", i); sl_push(a, b); snprintf(b, sizeof b, "%06d", 19999 - i); sl_push(d, b); snprintf(b, sizeof b, "%06d", (int)((long long)i * 7919 % 20000)); sl_push(s, b); }
    char **tmp = malloc(sizeof(char *) * 20000);
    #define SORTHASHS(src) { memcpy(tmp, (src)->a, sizeof(char *) * 20000); qsort(tmp, 20000, sizeof(char *), cmp_str); long long h = 0; for (int i = 0; i < 20000; i++) h = (h * 31 + (long long)strlen(tmp[i])) % 1000000007; checksum = h; }
    ROW("sort_asc", SORTHASHS(a));
    ROW("sort_desc", SORTHASHS(d));
    ROW("sort_rand", SORTHASHS(s));
    #undef SORTHASHS
    free(tmp); sl_free(a); sl_free(d); sl_free(s); }
  ROW("all", { long long a = 0; for (int k = 0; k < 200; k++) { int ok = 1; for (int i = 0; i < pos->n && ok; i++) if (!(strlen(pos->a[i]) > 0)) ok = 0; a += ok; } checksum = a; });
  ROW("any", { long long a = 0; for (int k = 0; k < 200; k++) { int any = 0; for (int i = 0; i < neg->n && !any; i++) if (strlen(neg->a[i]) > 0) any = 1; a += !any; } checksum = a; });
  ROW("chunks", { long long a = 0; for (int k = 0; k < 200; k++) a += (base->n + 9) / 10; checksum = a; });
  ROW("contains", { long long a = 0; for (int k = 0; k < 500; k++) { int f = 0; for (int i = 0; i < base->n && !f; i++) if (strcmp(base->a[i], "s1000") == 0) f = 1; a += !f; } checksum = a; });
  ROW("drop", { long long a = 0; for (int k = 0; k < 500; k++) a += base->n - 500; checksum = a; });
  ROW("filter", { long long a = 0; for (int k = 0; k < 200; k++) { int c = 0; for (int i = 0; i < base->n; i++) if (strlen(base->a[i]) <= 2) c++; a += c; } checksum = a; });
  ROW("find", { long long a = 0; for (int k = 0; k < 500; k++) { int idx = -1; for (int i = 0; i < base->n; i++) if (strcmp(base->a[i], "s999") == 0) { idx = i; break; } a += idx; } checksum = a; });
  ROW("findIndex", { long long a = 0; for (int k = 0; k < 500; k++) { int idx = -1; for (int i = 0; i < base->n; i++) if (strlen(base->a[i]) >= 4) { idx = i; break; } a += idx; } checksum = a; });
  ROW("findLastIndex", { long long a = 0; for (int k = 0; k < 500; k++) { int idx = -1; for (int i = base->n - 1; i >= 0; i--) if (strlen(base->a[i]) <= 2) { idx = i; break; } a += idx; } checksum = a; });
  ROW("flatten", { long long a = 0; for (int k = 0; k < 200; k++) a += 100 * 10; checksum = a; });
  ROW("forEach", { long long a = 0; for (int k = 0; k < 200; k++) for (int i = 0; i < base->n; i++) a += (long long)strlen(base->a[i]); checksum = a; });
  ROW("get", { long long a = 0; for (int k = 0; k < 100; k++) for (int i = 0; i < 1000; i++) a += (long long)strlen(base->a[i]); checksum = a; });
  ROW("getOr", { long long a = 0; for (int k = 0; k < 100; k++) for (int i = 0; i < 1000; i++) a += (long long)strlen(base->a[i]); checksum = a; });
  /* insert accumulates like append/prepend, but gen_list.py's OP_ORDER emits it
   * here (alphabetically, between getOr and mid) — keep the row order in step. */
  ROW("insert", { SL *l = sl_new(4); for (int i = 0; i < 1000; i++) { snprintf(b, sizeof b, "m%d", i); sl_ins(l, l->n / 2, b); } checksum = l->n; sl_free(l); });
  ROW("mid", { long long a = 0; for (int k = 0; k < 500; k++) a += 500; checksum = a; });
  ROW("partition", { long long a = 0; for (int k = 0; k < 200; k++) { int c = 0; for (int i = 0; i < base->n; i++) if (strlen(base->a[i]) <= 2) c++; a += c; } checksum = a; });
  ROW("reduce", { long long a = 0; for (int k = 0; k < 500; k++) { long long s = 0; for (int i = 0; i < base->n; i++) s += (long long)strlen(base->a[i]); a += s; } checksum = a; });
  ROW("reduceRight", { long long a = 0; for (int k = 0; k < 500; k++) { long long s = 0; for (int i = base->n - 1; i >= 0; i--) s += (long long)strlen(base->a[i]); a += s; } checksum = a; });
  ROW("removeAt", { SL *l = sl_new(base->n); for (int i = 0; i < base->n; i++) sl_push(l, base->a[i]); long long c = 0; while (l->n > 0) { free(l->a[0]); memmove(&l->a[0], &l->a[1], sizeof(char *) * (l->n - 1)); l->n--; c++; } checksum = c; sl_free(l); });
  ROW("replace", { long long a = 0; for (int k = 0; k < 200; k++) a += base->n; checksum = a; });
  { SL *b5 = sl_new(500); for (int i = 0; i < 500; i++) { snprintf(b, sizeof b, "s%d", i); sl_push(b5, b); }
    ROW("sortBy", { long long a = 0; for (int k = 0; k < 200; k++) { char **c = malloc(sizeof(char *) * 500); memcpy(c, b5->a, sizeof(char *) * 500); /* stable sort by length */ for (int i = 1; i < 500; i++) { char *key = c[i]; size_t kl = strlen(key); int j = i - 1; while (j >= 0 && strlen(c[j]) > kl) { c[j + 1] = c[j]; j--; } c[j + 1] = key; } a += (long long)strlen(c[0]); free(c); } checksum = a; });
    sl_free(b5); }
  ROW("take", { long long a = 0; for (int k = 0; k < 500; k++) a += 500; checksum = a; });
  ROW("transform", { long long a = 0; for (int k = 0; k < 200; k++) a += base->n; checksum = a; });
  ROW("window", { long long a = 0; for (int k = 0; k < 100; k++) a += base->n - 9; checksum = a; });
  ROW("zip", { long long a = 0; for (int k = 0; k < 100; k++) a += base->n; checksum = a; });

  sl_free(base);
}

void run_listmatrix_group(void) {
  run_ilist("list (Fixed)", "lf");
  run_slist("list (Dynamic)", "ld");
}
