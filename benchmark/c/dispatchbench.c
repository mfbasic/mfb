/* GROUP: dispatch (union+MATCH tag dispatch, inline-TRAP recovery)
 *
 * Mirrors benchmark/mfb/src/dispatch.mfb:
 *   union  a perfect binary expression tree (Num/Add/Mul) evaluated many times,
 *          dispatched by a tagged-union switch (the C analogue of MATCH).
 *          Arithmetic is mod 1000000007.
 *   trap   a mixed valid/invalid token stream parsed with strtol + an errno/endptr
 *          check recovering per token (the C analogue of an inline TRAP), so the
 *          error path is taken 1/4 of the time.
 * A per-iteration seed is added at every leaf so each full tree evaluation is
 * distinct (otherwise -O2 hoists the constant eval and the row is noise).
 * Checksums (union=212666511, trap=37475000) match the mfb and Python references. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "dispatchbench.h"

#define M 1000000007L

enum { NUM, ADD, MUL };
typedef struct {
  int tag;
  long value;
  int left, right;
} Node;

/* `seed` is added at every leaf so a per-iteration seed makes each full tree
 * evaluation distinct — otherwise -O2 hoists the loop-invariant constant eval and
 * the row collapses to noise. */
static long eval_node(const Node *nodes, int i, long seed) {
  const Node *n = &nodes[i];
  switch (n->tag) {
    case NUM:
      return (n->value + seed) % M;
    case ADD:
      return (eval_node(nodes, n->left, seed) + eval_node(nodes, n->right, seed)) % M;
    default: /* MUL */
      return (eval_node(nodes, n->left, seed) * eval_node(nodes, n->right, seed)) % M;
  }
}

static void test_dispatch_union(void) {
  enum { TOTAL = 2047, INTERNAL = 1023 };
  static Node nodes[TOTAL];
  for (int k = 0; k < TOTAL; k++) {
    if (k < INTERNAL) {
      nodes[k].tag = (k % 2 == 0) ? ADD : MUL;
      nodes[k].left = 2 * k + 1;
      nodes[k].right = 2 * k + 2;
    } else {
      nodes[k].tag = NUM;
      nodes[k].value = (k % 7) + 1;
    }
  }
  int evals = 2000;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int n = 0; n < evals; n++) acc = (acc + eval_node(nodes, 0, n)) % M;
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "dispatch_union = %ld\n", checksum);
  record("dispatch", "union", t, RUN);
  free(t);
}

static void test_dispatch_trap(void) {
  enum { N = 1000 };
  static char tokens[N][16];
  for (int i = 0; i < N; i++) {
    if (i % 4 == 0)
      snprintf(tokens[i], sizeof tokens[i], "bad");
    else
      snprintf(tokens[i], sizeof tokens[i], "%d", i);
  }
  int passes = 100;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int p = 0; p < passes; p++) {
      for (int i = 0; i < N; i++) {
        char *end;
        long v = strtol(tokens[i], &end, 10);
        if (end == tokens[i] || *end != '\0') v = -1; /* recover: parse failure */
        acc += v;
      }
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "dispatch_trap = %ld\n", checksum);
  record("dispatch", "trap", t, RUN);
  free(t);
}

void run_dispatch_group(void) {
  test_dispatch_union();
  test_dispatch_trap();
}
