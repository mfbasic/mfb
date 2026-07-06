/* Unified C benchmark — the native oracle for the MFBASIC benchmark suite.
 *
 * One function per micro-benchmark; each times its workload `run` times
 * (default 10, override with `--run N`) with CLOCK_MONOTONIC, then records
 * median / average / min / max in milliseconds. Results are grouped and printed
 * as:
 *
 *   GROUP:
 *     NAME: MED, AVG, MIN, MAX
 *
 * Every test prints its checksum to stderr so the optimizer cannot delete the
 * workload and the implementations can be cross-checked. Build with
 * `cc -O2 main.c -o bench -lm -lpthread` (the runner builds -O0 and -O2). The
 * `parse` group is intentionally absent: C has no standard-library CSV/JSON/
 * regex parser, so those tests exist only for mfb and Python. */
#include <math.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "bench.h"
#include "list.h"

int RUN = 10;

/* ----- timing + statistics --------------------------------------------- */

long long now_ns(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (long long)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

typedef struct {
  const char *group;
  const char *name;
  double med, avg, min, max;
} Result;

static Result results[128];
static int nresults = 0;

static int cmp_ll(const void *a, const void *b) {
  long long x = *(const long long *)a, y = *(const long long *)b;
  return (x > y) - (x < y);
}

/* Record one benchmark from its per-run elapsed nanosecond samples. */
void record(const char *group, const char *name, long long *times, int n) {
  qsort(times, n, sizeof(long long), cmp_ll);
  double med;
  if (n % 2)
    med = (double)times[n / 2];
  else
    med = ((double)times[n / 2 - 1] + (double)times[n / 2]) / 2.0;
  long long sum = 0, mn = times[0], mx = times[0];
  for (int i = 0; i < n; i++) {
    sum += times[i];
    if (times[i] < mn) mn = times[i];
    if (times[i] > mx) mx = times[i];
  }
  Result r;
  r.group = group;
  r.name = name;
  r.med = med / 1e6;
  r.avg = (double)sum / n / 1e6;
  r.min = (double)mn / 1e6;
  r.max = (double)mx / 1e6;
  results[nresults++] = r;
}

static void print_results(void) {
  printf("# columns: median, average, min, max (milliseconds)\n");
  const char *last = "";
  for (int i = 0; i < nresults; i++) {
    Result *r = &results[i];
    if (strcmp(r->group, last) != 0) {
      printf("\n%s:\n", r->group);
      last = r->group;
    }
    printf("  %-12s: %10.3f, %10.3f, %10.3f, %10.3f\n", r->name, r->med, r->avg, r->min, r->max);
  }
}

/* A fresh times buffer for a test; sized to RUN. */
long long *alloc_times(void) {
  return malloc((size_t)RUN * sizeof(long long));
}

/* ===================================================================== */
/* GROUP: recurse                                                        */
/* ===================================================================== */

static long fib(int n) {
  if (n < 2) return n;
  return fib(n - 1) + fib(n - 2);
}

static void test_fib(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    checksum = fib(35);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "fib = %ld\n", checksum);
  record("recurse", "fib", t, RUN);
  free(t);
}

static long ack(int m, int n) {
  if (m == 0) return n + 1;
  if (n == 0) return ack(m - 1, 1);
  return ack(m - 1, ack(m, n - 1));
}

static void test_ackermann(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    checksum = ack(3, 7);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "ackermann = %ld\n", checksum);
  record("recurse", "ackermann", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: float                                                          */
/* ===================================================================== */

static void test_leibniz(void) {
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double sum = 0.0, sign = 1.0;
    for (int k = 0; k < 1000000; k++) {
      double denom = (double)(2 * k + 1);
      sum += sign / denom;
      sign = sign * -1.0;
    }
    checksum = 4.0 * sum;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "leibniz = %.5f\n", checksum);
  record("float", "leibniz", t, RUN);
  free(t);
}

#define NB 5
static void test_nbody(void) {
  const double PI = 3.141592653589793;
  const double SOLAR_MASS = 4.0 * PI * PI;
  const double DPY = 365.24;
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double x[NB], y[NB], z[NB], vx[NB], vy[NB], vz[NB], mass[NB];
    x[0] = 0; y[0] = 0; z[0] = 0; vx[0] = 0; vy[0] = 0; vz[0] = 0; mass[0] = SOLAR_MASS;
    x[1] = 4.84143144246472090; y[1] = -1.16032004402742839; z[1] = -1.03622044471123109e-01;
    vx[1] = 1.66007664274403694e-03 * DPY; vy[1] = 7.69901118419740425e-03 * DPY; vz[1] = -6.90460016972063023e-05 * DPY;
    mass[1] = 9.54791938424326609e-04 * SOLAR_MASS;
    x[2] = 8.34336671824457987; y[2] = 4.12479856412430479; z[2] = -4.03523417114321381e-01;
    vx[2] = -2.76742510726862411e-03 * DPY; vy[2] = 4.99852801234917238e-03 * DPY; vz[2] = 2.30417297573763929e-05 * DPY;
    mass[2] = 2.85885980666130812e-04 * SOLAR_MASS;
    x[3] = 1.28943695621391310e+01; y[3] = -1.51111514016986312e+01; z[3] = -2.23307578892655734e-01;
    vx[3] = 2.96460137564761618e-03 * DPY; vy[3] = 2.37847173959480950e-03 * DPY; vz[3] = -2.96589568540237556e-05 * DPY;
    mass[3] = 4.36624404335156298e-05 * SOLAR_MASS;
    x[4] = 1.53796971148509165e+01; y[4] = -2.59193146099879641e+01; z[4] = 1.79258772950371181e-01;
    vx[4] = 2.68067772490389322e-03 * DPY; vy[4] = 1.62824170038242295e-03 * DPY; vz[4] = -9.51592254519715870e-05 * DPY;
    mass[4] = 5.15138902046611451e-05 * SOLAR_MASS;

    double px = 0, py = 0, pz = 0;
    for (int i = 0; i < NB; i++) { px += vx[i] * mass[i]; py += vy[i] * mass[i]; pz += vz[i] * mass[i]; }
    vx[0] = -px / SOLAR_MASS; vy[0] = -py / SOLAR_MASS; vz[0] = -pz / SOLAR_MASS;

    for (int s = 0; s < 100000; s++) {
      for (int i = 0; i < NB; i++) {
        for (int j = i + 1; j < NB; j++) {
          double dx = x[i] - x[j], dy = y[i] - y[j], dz = z[i] - z[j];
          double d2 = dx * dx + dy * dy + dz * dz;
          double mag = 0.01 / (d2 * sqrt(d2));
          vx[i] -= dx * mass[j] * mag; vy[i] -= dy * mass[j] * mag; vz[i] -= dz * mass[j] * mag;
          vx[j] += dx * mass[i] * mag; vy[j] += dy * mass[i] * mag; vz[j] += dz * mass[i] * mag;
        }
      }
      for (int i = 0; i < NB; i++) { x[i] += 0.01 * vx[i]; y[i] += 0.01 * vy[i]; z[i] += 0.01 * vz[i]; }
    }
    double e = 0.0;
    for (int i = 0; i < NB; i++) {
      e += 0.5 * mass[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
      for (int j = i + 1; j < NB; j++) {
        double dx = x[i] - x[j], dy = y[i] - y[j], dz = z[i] - z[j];
        e -= mass[i] * mass[j] / sqrt(dx * dx + dy * dy + dz * dz);
      }
    }
    checksum = e;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "nbody = %.9f\n", checksum);
  record("float", "nbody", t, RUN);
  free(t);
}

static void test_mandelbrot(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int w = 600, h = 600, maxiter = 100, inset = 0;
    for (int yy = 0; yy < h; yy++) {
      double im = -1.5 + 3.0 * ((double)yy + 0.5) / (double)h;
      for (int xx = 0; xx < w; xx++) {
        double re = -2.0 + 3.0 * ((double)xx + 0.5) / (double)w;
        double zr = 0, zi = 0;
        int escaped = 0, i = 0;
        while (i < maxiter) {
          double nzr = zr * zr - zi * zi + re;
          double nzi = 2.0 * zr * zi + im;
          zr = nzr; zi = nzi;
          if (zr * zr + zi * zi > 4.0) { escaped = 1; i = maxiter; } else { i++; }
        }
        if (!escaped) inset++;
      }
    }
    checksum = inset;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "mandelbrot = %ld\n", checksum);
  record("float", "mandelbrot", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: math (each kernel run 2000 x 1000 times)                       */
/* ===================================================================== */

#define MATH_KERNEL(fnname, label, expr, init, step)         \
  static void fnname(void) {                                 \
    long long *t = alloc_times();                            \
    double checksum = 0.0;                                   \
    for (int r = 0; r < RUN; r++) {                          \
      long long t0 = now_ns();                               \
      double acc = 0.0;                                      \
      for (int rep = 0; rep < 2000; rep++) {                 \
        double v = (init);                                   \
        for (int i = 0; i < 1000; i++) {                     \
          acc += (expr);                                     \
          v += (step);                                       \
        }                                                    \
      }                                                      \
      checksum = acc;                                        \
      t[r] = now_ns() - t0;                                  \
    }                                                        \
    fprintf(stderr, "%s = %.6f\n", label, checksum);         \
    record("math", label, t, RUN);                           \
    free(t);                                                 \
  }

MATH_KERNEL(test_sin, "sin", sin(v), 0.001, 0.0015)
MATH_KERNEL(test_cos, "cos", cos(v), 0.001, 0.0015)
MATH_KERNEL(test_tan, "tan", tan(v), 0.001, 0.0015)
MATH_KERNEL(test_atan2, "atan2", atan2(v, 1.0 + v), 0.001, 0.0015)
MATH_KERNEL(test_asin, "asin", asin(v), -0.999, 0.001998)
MATH_KERNEL(test_acos, "acos", acos(v), -0.999, 0.001998)
MATH_KERNEL(test_atan, "atan", atan(v), -0.999, 0.001998)
MATH_KERNEL(test_exp, "exp", exp(v * 0.1), 0.001, 0.005)
MATH_KERNEL(test_log, "log", log(v), 0.001, 0.005)
MATH_KERNEL(test_log10, "log10", log10(v), 0.001, 0.005)
MATH_KERNEL(test_pow, "pow", pow(v, 1.5), 0.001, 0.005)
MATH_KERNEL(test_sqrt, "sqrt", sqrt(v), 0.001, 0.005)

/* GROUP: list lives in list.c (see run_list_group). */

/* ===================================================================== */
/* GROUP: map (open-addressing hash tables)                              */
/* ===================================================================== */

typedef struct { char *key; long val; int used; } SSlot;
static unsigned long djb2(const char *s) {
  unsigned long h = 5381; int c;
  while ((c = *s++)) h = ((h << 5) + h) + c;
  return h;
}

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

/* ===================================================================== */
/* GROUP: string                                                         */
/* ===================================================================== */

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

/* ===================================================================== */
/* GROUP: record                                                         */
/* ===================================================================== */

struct URec { int n; char label[16]; };

static void test_record_update(void) {
  long long *t = alloc_times();
  long checksum = 0;
  struct URec recs[100];
  for (int r = 0; r < RUN; r++) {
    for (int i = 0; i < 100; i++) { recs[i].n = i; snprintf(recs[i].label, sizeof recs[i].label, "p%d", i); }
    long long t0 = now_ns();
    for (int pass = 0; pass < 10; pass++)
      for (int j = 0; j < 100; j++) recs[j].n = recs[j].n + 1;
    t[r] = now_ns() - t0;
    long sum = 0;
    for (int j = 0; j < 100; j++) sum += recs[j].n;
    checksum = sum;
  }
  fprintf(stderr, "record_update = %ld\n", checksum);
  record("record", "update", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: bignum (base-2^28 limbs; schoolbook mul + bit-serial mod)      */
/* ===================================================================== */

#define BN_MASK 268435455ULL
#define BN_CAP 24
typedef struct { uint64_t v[BN_CAP]; int n; } bn;

static void bn_norm(bn *a) { while (a->n > 1 && a->v[a->n - 1] == 0) a->n--; }
static int bn_cmp(const bn *a, const bn *b) {
  int n = a->n > b->n ? a->n : b->n;
  for (int i = n - 1; i >= 0; i--) {
    uint64_t ai = i < a->n ? a->v[i] : 0, bi = i < b->n ? b->v[i] : 0;
    if (ai < bi) return -1;
    if (ai > bi) return 1;
  }
  return 0;
}
static void bn_add(bn *r, const bn *a, const bn *b) {
  int n = a->n > b->n ? a->n : b->n; uint64_t c = 0;
  for (int i = 0; i < n; i++) {
    uint64_t ai = i < a->n ? a->v[i] : 0, bi = i < b->n ? b->v[i] : 0, s = ai + bi + c;
    r->v[i] = s & BN_MASK; c = s >> 28;
  }
  r->n = n;
  if (c) r->v[r->n++] = c;
}
static void bn_sub(bn *r, const bn *a, const bn *b) {
  int64_t brw = 0;
  for (int i = 0; i < a->n; i++) {
    int64_t bi = i < b->n ? (int64_t)b->v[i] : 0, s = (int64_t)a->v[i] - bi - brw;
    if (s < 0) { s += 268435456; brw = 1; } else brw = 0;
    r->v[i] = (uint64_t)s;
  }
  r->n = a->n; bn_norm(r);
}
static void bn_mul(bn *r, const bn *a, const bn *b) {
  memset(r->v, 0, sizeof(r->v)); r->n = a->n + b->n;
  for (int i = 0; i < a->n; i++) {
    uint64_t c = 0, ai = a->v[i];
    for (int j = 0; j < b->n; j++) {
      uint64_t s = r->v[i + j] + ai * b->v[j] + c;
      r->v[i + j] = s & BN_MASK; c = s >> 28;
    }
    r->v[i + b->n] += c;
  }
  bn_norm(r);
}
static void bn_shl1(bn *a) {
  uint64_t c = 0;
  for (int i = 0; i < a->n; i++) { uint64_t s = (a->v[i] << 1) | c; a->v[i] = s & BN_MASK; c = s >> 28; }
  if (c) a->v[a->n++] = c;
}
static void bn_mod(bn *x, const bn *m) {
  if (bn_cmp(x, m) < 0) return;
  int nbits = x->n * 28;
  bn r, one, t; r.v[0] = 0; r.n = 1; one.v[0] = 1; one.n = 1;
  for (int i = nbits - 1; i >= 0; i--) {
    uint64_t bit = (x->v[i / 28] >> (i % 28)) & 1;
    bn_shl1(&r);
    if (bit) { bn_add(&t, &r, &one); r = t; }
    if (bn_cmp(&r, m) >= 0) { bn_sub(&t, &r, m); r = t; }
  }
  *x = r;
}
static void bn_modmul(bn *r, const bn *a, const bn *b, const bn *m) { bn_mul(r, a, b); bn_mod(r, m); }

static bn bn_p(void) { bn p = {{268435455, 268435455, 268435455, 4095, 0, 0, 16777216, 0, 268435455, 15}, 10}; return p; }
static bn bn_g(void) { bn g = {{220077856, 27374017, 102176793, 20005201, 252711186, 12636384, 134810123, 5267568, 16909060}, 9}; return g; }

static void test_bignum_modmul(void) {
  long long *t = alloc_times();
  unsigned long long checksum = 0;
  bn p = bn_p(), g = bn_g();
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    bn b = g, tt;
    for (int i = 0; i < 200; i++) { bn_modmul(&tt, &b, &g, &p); b = tt; }
    unsigned long long acc = 0;
    for (int j = 0; j < b.n; j++) acc += b.v[j];
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "bignum_modmul = %llu\n", checksum);
  record("bignum", "modmul", t, RUN);
  free(t);
}

static void test_bignum_modexp(void) {
  long long *t = alloc_times();
  unsigned long long checksum = 0;
  bn p = bn_p(), g = bn_g();
  uint64_t e = 6822318947648322238ULL;
  for (int rr = 0; rr < RUN; rr++) {
    long long t0 = now_ns();
    bn r = {{1}, 1}, b = g, tt;
    for (int i = 0; i < 63; i++) {
      if ((e >> i) & 1) { bn_modmul(&tt, &r, &b, &p); r = tt; }
      bn_modmul(&tt, &b, &b, &p); b = tt;
    }
    unsigned long long acc = 0;
    for (int j = 0; j < r.n; j++) acc += r.v[j];
    checksum = acc;
    t[rr] = now_ns() - t0;
  }
  fprintf(stderr, "bignum_modexp = %llu\n", checksum);
  record("bignum", "modexp", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: io (file-based; matches the mfb line-by-line workload)         */
/* ===================================================================== */

static void io_path(char *out, size_t n, const char *name) {
  const char *tmp = getenv("TMPDIR");
  if (!tmp || !*tmp) tmp = "/tmp";
  snprintf(out, n, "%s/%s", tmp, name);
}

static long write_lines(const char *path, int count) {
  FILE *f = fopen(path, "w");
  for (int i = 0; i < count; i++) fprintf(f, "%d\n", i);
  fclose(f);
  return count;
}

static void test_io_write(void) {
  char path[512];
  io_path(path, sizeof path, "c-bench-io-write.txt");
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    checksum = write_lines(path, 20000);
    t[r] = now_ns() - t0;
  }
  remove(path);
  fprintf(stderr, "io_write = %ld\n", checksum);
  record("io", "write", t, RUN);
  free(t);
}

static void test_io_read(void) {
  char path[512];
  io_path(path, sizeof path, "c-bench-io-read.txt");
  write_lines(path, 20000);
  long long *t = alloc_times();
  long checksum = 0;
  char buf[64];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    FILE *f = fopen(path, "r");
    long lines = 0;
    while (fgets(buf, sizeof buf, f)) lines++;
    fclose(f);
    checksum = lines;
    t[r] = now_ns() - t0;
  }
  remove(path);
  fprintf(stderr, "io_read = %ld\n", checksum);
  record("io", "read", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: vector (scalar 3D geometry, same op order as mfb/python)       */
/* ===================================================================== */

static void test_vector_math(void) {
  long long *t = alloc_times();
  double checksum = 0.0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    double acc = 0.0;
    for (long k = 0; k < 200000; k++) {
      double fk = (double)k;
      double ax = fk + 1.0, ay = fk * 0.5 + 2.0, az = 3.0 - fk * 0.25;
      double bx = 2.0 - fk * 0.125, by = fk + 0.5, bz = fk * 0.75 + 1.0;
      double la = sqrt(ax * ax + ay * ay + az * az);
      double nax = ax / la, nay = ay / la, naz = az / la;
      double lb = sqrt(bx * bx + by * by + bz * bz);
      double nbx = bx / lb, nby = by / lb, nbz = bz / lb;
      double cx = nay * nbz - naz * nby, cy = naz * nbx - nax * nbz, cz = nax * nby - nay * nbx;
      double mx = ax + (bx - ax) * 0.5, my = ay + (by - ay) * 0.5, mz = az + (bz - az) * 0.5;
      double sx = nax * nbx, sy = nay * nby, sz = naz * nbz;
      double dcm = cx * mx + cy * my + cz * mz;
      double lens = sqrt(sx * sx + sy * sy + sz * sz);
      double dx = ax - bx, dy = ay - by, dz = az - bz;
      double dist = sqrt(dx * dx + dy * dy + dz * dz);
      acc += dcm + lens + dist;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "vector_math = %.6f\n", checksum);
  record("vector", "math", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: primes                                                         */
/* ===================================================================== */

static int is_prime(int n) {
  if (n < 2) return 0;
  for (int i = 2; i * i <= n; i++) if (n % i == 0) return 0;
  return 1;
}

static void test_primes(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int primes[1000];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int found = 0, candidate = 2;
    while (found < 1000) {
      if (is_prime(candidate)) primes[found++] = candidate;
      candidate++;
    }
    checksum = primes[999];
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "primes = %ld\n", checksum);
  record("primes", "primes", t, RUN);
  free(t);
}

/* ===================================================================== */
/* GROUP: thread (4 workers x 10,000,000, real pthreads)                 */
/* ===================================================================== */

typedef struct { long start; long result; } SumArg;
static void *sum_chunk(void *p) {
  SumArg *a = p;
  long total = 0;
  for (long i = a->start; i < a->start + 10000000L; i++) total += i;
  a->result = total;
  return NULL;
}

static void test_thread_sum(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    pthread_t th[4]; SumArg args[4];
    for (int k = 0; k < 4; k++) { args[k].start = (long)k * 10000000L; pthread_create(&th[k], NULL, sum_chunk, &args[k]); }
    long total = 0;
    for (int k = 0; k < 4; k++) { pthread_join(th[k], NULL); total += args[k].result; }
    checksum = total;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "thread_sum = %ld\n", checksum);
  record("thread", "sum", t, RUN);
  free(t);
}

/* ===================================================================== */

int main(int argc, char **argv) {
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--run") == 0 && i + 1 < argc) {
      int v = atoi(argv[i + 1]);
      if (v >= 1) RUN = v;
    }
  }
  fprintf(stderr, "running each test %d time(s)\n", RUN);

  test_fib();
  test_ackermann();

  test_leibniz();
  test_nbody();
  test_mandelbrot();

  test_sin(); test_cos(); test_tan(); test_atan2();
  test_asin(); test_acos(); test_atan();
  test_exp(); test_log(); test_log10(); test_pow(); test_sqrt();

  run_list_group();

  test_map_set();
  test_map_lookup();

  test_string_concat();

  test_record_update();

  test_bignum_modmul();
  test_bignum_modexp();

  test_io_write();
  test_io_read();

  test_vector_math();

  test_primes();

  test_thread_sum();

  print_results();
  return 0;
}
