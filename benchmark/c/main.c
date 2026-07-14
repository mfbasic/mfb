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
 * `parse` group (parsebench.c) vendors parson (JSON) + libcsv (CSV) and uses
 * POSIX <regex.h> for regex, so csv/json/regex compare across all three
 * languages. */
#include <math.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "arenabench.h"
#include "bench.h"
#include "bitsbench.h"
#include "churnbench.h"
#include "list.h"
#include "mapbench.h"
#include "mathbench.h"
#include "mathpipe.h"
#include "parsebench.h"
#include "regexbench.h"
#include "scalarbench.h"
#include "strbuildbench.h"
#include "stringbench.h"
#include "vectorbench.h"

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
    printf("  %-15s: %10.3f, %10.3f, %10.3f, %10.3f\n", r->name, r->med, r->avg, r->min, r->max);
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

/* GROUP: math lives in mathbench.c (see run_math_group). */
/* GROUP: list lives in list.c (see run_list_group / run_liststr_group). */
/* GROUP: map lives in mapbench.c (see run_map_group). */
/* GROUP: string lives in stringbench.c (see run_string_group). */
/* GROUP: bits lives in bitsbench.c (see run_bits_group). */

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

/* Write `count` "i\n" lines, optionally with full stdio buffering. */
static long write_lines_buffered(const char *path, int count, int buffered) {
  FILE *f = fopen(path, "w");
  if (buffered)
    setvbuf(f, NULL, _IOFBF, 65536);
  else
    setvbuf(f, NULL, _IONBF, 0); /* one write() per line */
  for (int i = 0; i < count; i++) fprintf(f, "%d\n", i);
  fclose(f);
  return count;
}

/* readnum — read a many-line file back and parse each line to an int, summing
 * them (the read+tokenize hot path; `io read` only counts lines). */
static void test_io_readnum(void) {
  char path[512];
  io_path(path, sizeof path, "c-bench-io-readnum.txt");
  write_lines(path, 20000);
  long long *t = alloc_times();
  long checksum = 0;
  char buf[64];
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    FILE *f = fopen(path, "r");
    long sumv = 0;
    while (fgets(buf, sizeof buf, f)) sumv += atol(buf);
    fclose(f);
    checksum = sumv;
    t[r] = now_ns() - t0;
  }
  remove(path);
  fprintf(stderr, "io_readnum = %ld\n", checksum);
  record("io", "readnum", t, RUN);
  free(t);
}

/* buffered — 20k incremental writes with output buffering ON. */
static void test_io_buf_on(void) {
  char path[512];
  io_path(path, sizeof path, "c-bench-io-bufon.txt");
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    checksum = write_lines_buffered(path, 20000, 1);
    t[r] = now_ns() - t0;
  }
  remove(path);
  fprintf(stderr, "io_buf_on = %ld\n", checksum);
  record("io", "buf_on", t, RUN);
  free(t);
}

/* unbuffered — the same 20k writes with buffering OFF (one write() per line). */
static void test_io_buf_off(void) {
  char path[512];
  io_path(path, sizeof path, "c-bench-io-bufoff.txt");
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    checksum = write_lines_buffered(path, 20000, 0);
    t[r] = now_ns() - t0;
  }
  remove(path);
  fprintf(stderr, "io_buf_off = %ld\n", checksum);
  record("io", "buf_off", t, RUN);
  free(t);
}

/* format — mixed Integer/Float/String formatting to a temp file. */
static void test_io_format(void) {
  char path[512];
  io_path(path, sizeof path, "c-bench-io-format.txt");
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    FILE *f = fopen(path, "w");
    setvbuf(f, NULL, _IOFBF, 65536);
    for (int i = 0; i < 20000; i++) fprintf(f, "%d %.3f row%d\n", i, (double)i * 0.5, i);
    fclose(f);
    checksum = 20000;
    t[r] = now_ns() - t0;
  }
  remove(path);
  fprintf(stderr, "io_format = %ld\n", checksum);
  record("io", "format", t, RUN);
  free(t);
}

/* binary — byte round-trip: build a payload, write it, read it back, sum bytes;
 * 5 passes accumulate (checksum 803430). */
static void test_io_binary(void) {
  char path[512];
  io_path(path, sizeof path, "c-bench-io-binary.bin");
  /* payload = concatenation of "byte%d;" for i=0..255 */
  char *payload = NULL;
  size_t plen = 0, pcap = 0;
  char tok[16];
  for (int i = 0; i < 256; i++) {
    int nl = snprintf(tok, sizeof tok, "byte%d;", i);
    if (plen + (size_t)nl + 1 > pcap) { pcap = (plen + (size_t)nl + 1) * 2; payload = realloc(payload, pcap); }
    memcpy(payload + plen, tok, (size_t)nl);
    plen += (size_t)nl;
  }
  long long *t = alloc_times();
  long checksum = 0;
  unsigned char *back = malloc(plen);
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int pass = 0; pass < 5; pass++) {
      FILE *wf = fopen(path, "wb");
      fwrite(payload, 1, plen, wf);
      fclose(wf);
      FILE *rf = fopen(path, "rb");
      size_t got = fread(back, 1, plen, rf);
      fclose(rf);
      long sb = 0;
      for (size_t k = 0; k < got; k++) sb += back[k];
      acc += sb;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  remove(path);
  fprintf(stderr, "io_binary = %ld\n", checksum);
  record("io", "binary", t, RUN);
  free(t);
  free(payload);
  free(back);
}

/* GROUP: vector lives in vectorbench.c (see run_vector_group). */

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
  test_matmul();

  run_mathpipe_group();

  run_math_group();

  run_list_group();
  run_liststr_group();

  run_listchurn_group();

  run_map_group();

  run_mapchurn_group();

  run_string_group();
  test_string_unibig();

  run_strbuild_group();

  run_bits_group();

  test_record_update();

  test_bignum_modmul();
  test_bignum_modexp();

  run_parse_group();

  run_regexbench_group();

  test_io_write();
  test_io_read();
  test_io_readnum();
  test_io_buf_on();
  test_io_buf_off();
  test_io_format();
  test_io_binary();

  run_vector_group();

  run_arena_group();

  run_scalarbench_group();

  test_primes();

  test_thread_sum();

  print_results();
  return 0;
}
