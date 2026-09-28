/* GROUP: closure — the C peer for closure.mfb (P1 coverage). C has no closures,
 * so a function value is the conventional pair {function pointer, env pointer}:
 *   - hof_param: a noinline user HOF calls through the pointer; the named FUNC
 *     and the (capture-free) LAMBDA are both plain functions with a NULL env;
 *   - closure_ret: the "returned lambda" is a heap-allocated env holding the
 *     captured scale/offset plus the code pointer, built once per closure and
 *     invoked through the pointer.
 *   - capture_mut: mfb's forEach LAMBDA updating an outer MUT is a callback
 *     taking an env struct that points at the loop-body accumulator (the
 *     by-reference capture) and holds the captured scale by value; a noinline
 *     for_each calls it through the pointer for every element.
 * noinline on the HOF/factory keeps -O2 from collapsing the indirect call into
 * straight-line arithmetic (mfb calls through the FUNC value too).
 *
 * Expected checksums: hof_param=2499997500000, closure_ret=34546050000,
 * capture_mut=1996501500. */
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "closurebench.h"

typedef long long (*CloFn)(const void *env, long long x);

typedef struct {
  CloFn fn;
  const void *env;
} Closure;

__attribute__((noinline)) static long long clo_apply(Closure f, long long x) {
  return f.fn(f.env, x);
}

static long long clo_step(const void *env, long long x) {
  (void)env;
  return x * 2 + 1;
}

static long long clo_lambda(const void *env, long long x) {
  (void)env;
  return x * 3 - 1;
}

typedef struct {
  long long scale, offset;
} AffineEnv;

static long long clo_affine(const void *env, long long x) {
  const AffineEnv *e = env;
  return x * e->scale + e->offset;
}

__attribute__((noinline)) static Closure clo_make_affine(long long k, long long offset) {
  AffineEnv *e = malloc(sizeof *e);
  e->scale = k * 2 + 1;
  e->offset = offset;
  Closure c = {clo_affine, e};
  return c;
}

static void test_closure_hof_param(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    Closure named = {clo_step, NULL};
    Closure lam = {clo_lambda, NULL};
    bench_opaque(&named); /* the call targets are unknown to the optimizer, */
    bench_opaque(&lam);   /* like mfb's FUNC values */
    for (long long i = 0; i < 1000000; i++)
      acc += clo_apply(named, i) + clo_apply(lam, i);
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_closure_hof_param = %lld\n", checksum);
  record("closure", "hof_param", t, RUN);
  free(t);
}

static void test_closure_closure_ret(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    for (int rep = 0; rep < 100; rep++) {
      Closure f = clo_make_affine(rep % 7, rep);
      bench_opaque(&f);
      for (long long i = 0; i < 10000; i++) acc += f.fn(f.env, i);
      free((void *)f.env);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_closure_closure_ret = %lld\n", checksum);
  record("closure", "closure_ret", t, RUN);
  free(t);
}

typedef void (*EachFn)(void *env, long long x);

typedef struct {
  long long *total; /* the captured MUT, by reference */
  long long scale;  /* the captured LET, by value */
} AccumEnv;

static void clo_accum(void *env, long long x) {
  AccumEnv *e = env;
  *e->total += x * e->scale;
}

__attribute__((noinline)) static void clo_for_each(const long long *xs, int n, EachFn fn,
                                                   void *env) {
  for (int i = 0; i < n; i++) fn(env, xs[i]);
}

static void test_closure_capture_mut(void) {
  long long xs[1000];
  for (int i = 0; i < 1000; i++) xs[i] = (i * 7919LL) % 1000;
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    EachFn fn = clo_accum;
    bench_opaque(&fn); /* the callback is unknown to the optimizer, like mfb's */
    for (int rep = 0; rep < 1000; rep++) {
      long long total = 0;
      AccumEnv env = {&total, rep % 7 + 1};
      clo_for_each(xs, 1000, fn, &env);
      acc += total;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_closure_capture_mut = %lld\n", checksum);
  record("closure", "capture_mut", t, RUN);
  free(t);
}

void run_closure_group(void) {
  test_closure_hof_param();
  test_closure_closure_ret();
  test_closure_capture_mut();
}
