/* GROUP: tree — the C peer for tree.mfb (recursive-union trees).
 *
 * mfb models the tree as a recursive UNION whose member TYPE holds the union
 * itself (owned child nodes, taken apart with MATCH). The C analogue is a
 * malloc'd struct node per tree node, freed when the tree is dropped:
 *   binarytrees  the Benchmarks Game binary-trees shape — stretch tree, one
 *                long-lived tree, and 2^(MAXD-d+4) short-lived trees at each
 *                depth d = 4..MAXD step 2, built, checked (node count), freed.
 *                A leaf is a node with NULL children (the payload-free BLeaf).
 *   treewalk     one heap-numbered weighted tree (depth 16) built untimed, then
 *                walked WALKS times computing a depth-weighted sum; the starting
 *                level varies per walk so -O2 cannot hoist a constant walk.
 * Checksums (binarytrees=674478, treewalk=932241450) match mfb and Python. */
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "treebench.h"

/* ----- binarytrees --------------------------------------------------------- */

#define BT_MAXD 12
#define BT_MIND 4

typedef struct BNode {
  struct BNode *left, *right; /* both NULL for a leaf */
} BNode;

static BNode *bottom_up_tree(int depth) {
  BNode *n = malloc(sizeof *n);
  if (depth <= 0) {
    n->left = n->right = NULL;
  } else {
    n->left = bottom_up_tree(depth - 1);
    n->right = bottom_up_tree(depth - 1);
  }
  return n;
}

static long long item_check(const BNode *n) {
  if (n->left == NULL) return 1;
  return 1 + item_check(n->left) + item_check(n->right);
}

static void free_tree(BNode *n) {
  if (n->left != NULL) {
    free_tree(n->left);
    free_tree(n->right);
  }
  free(n);
}

static long long binary_trees(int max_depth) {
  BNode *stretch = bottom_up_tree(max_depth + 1);
  long long total = item_check(stretch);
  free_tree(stretch);
  BNode *long_lived = bottom_up_tree(max_depth);
  for (int depth = BT_MIND; depth <= max_depth; depth += 2) {
    long long iterations = 1LL << (max_depth - depth + BT_MIND);
    long long check = 0;
    for (long long i = 1; i <= iterations; i++) {
      BNode *t = bottom_up_tree(depth);
      check += item_check(t);
      free_tree(t);
    }
    total += check;
  }
  total += item_check(long_lived);
  free_tree(long_lived);
  return total;
}

static void test_tree_binarytrees(void) {
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    checksum = binary_trees(BT_MAXD);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_tree_binarytrees = %lld\n", checksum);
  record("tree", "binarytrees", t, RUN);
  free(t);
}

/* ----- treewalk ------------------------------------------------------------ */

#define TW_DEPTH 16
#define TW_WALKS 50

typedef struct WNode {
  long long value;
  struct WNode *left, *right; /* both NULL for a leaf (WLeaf) */
} WNode;

/* Heap-numbered: node k has children 2k+1 / 2k+2; internal weight (k%5)+1,
 * leaf weight (k%7)+1. */
static WNode *build_weighted(int depth, long long k) {
  WNode *n = malloc(sizeof *n);
  if (depth <= 0) {
    n->value = (k % 7) + 1;
    n->left = n->right = NULL;
  } else {
    n->value = (k % 5) + 1;
    n->left = build_weighted(depth - 1, 2 * k + 1);
    n->right = build_weighted(depth - 1, 2 * k + 2);
  }
  return n;
}

static long long walk_weighted(const WNode *n, long long level) {
  if (n->left == NULL) return n->value * level;
  return n->value * level + walk_weighted(n->left, level + 1) +
         walk_weighted(n->right, level + 1);
}

static void free_weighted(WNode *n) {
  if (n->left != NULL) {
    free_weighted(n->left);
    free_weighted(n->right);
  }
  free(n);
}

static void test_tree_treewalk(void) {
  WNode *tree = build_weighted(TW_DEPTH, 0); /* 131071 nodes, untimed */
  long long *t = alloc_times();
  long long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long long acc = 0;
    for (long long w = 1; w <= TW_WALKS; w++) acc += walk_weighted(tree, w);
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "test_tree_treewalk = %lld\n", checksum);
  record("tree", "treewalk", t, RUN);
  free(t);
  free_weighted(tree);
}

void run_tree_group(void) {
  test_tree_binarytrees();
  test_tree_treewalk();
}
