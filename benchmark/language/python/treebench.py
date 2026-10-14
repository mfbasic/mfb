"""Recursive-union tree benchmarks (group `tree`).

Mirrors benchmark/language/mfb/src/tree.mfb. mfb models the tree as a recursive UNION
(owned child nodes, taken apart with MATCH); the Python peer uses tuples:
  binarytrees  the Benchmarks Game binary-trees shape — a node is a
               (left, right) tuple and a leaf is None; stretch tree, one
               long-lived tree, and 2^(MAXD-d+4) short-lived trees at each depth
               d = 4..MAXD step 2, built and checked (node count).
  treewalk     a (value, left, right) / (value,) weighted tree (depth 16) built
               untimed, then walked WALKS times computing a depth-weighted sum.

Expected checksums: binarytrees=674478, treewalk=932241450.
"""
import sys

RUN = 1
now_ns = None
record = None

BT_MAXD = 12
BT_MIND = 4
TW_DEPTH = 16
TW_WALKS = 50


# ---- binarytrees -------------------------------------------------------------

def bottom_up_tree(depth):
    if depth <= 0:
        return None
    return (bottom_up_tree(depth - 1), bottom_up_tree(depth - 1))


def item_check(t):
    if t is None:
        return 1
    return 1 + item_check(t[0]) + item_check(t[1])


def binary_trees(max_depth):
    total = item_check(bottom_up_tree(max_depth + 1))  # stretch tree
    long_lived = bottom_up_tree(max_depth)
    for depth in range(BT_MIND, max_depth + 1, 2):
        iterations = 1 << (max_depth - depth + BT_MIND)
        check = 0
        for _ in range(iterations):
            check += item_check(bottom_up_tree(depth))
        total += check
    return total + item_check(long_lived)


def test_tree_binarytrees():
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        checksum = binary_trees(BT_MAXD)
        times.append(now_ns() - t0)
    print("test_tree_binarytrees = %d" % checksum, file=sys.stderr)
    record("tree", "binarytrees", times)


# ---- treewalk ----------------------------------------------------------------

def build_weighted(depth, k):
    # Heap-numbered: children 2k+1 / 2k+2; internal weight (k%5)+1, leaf (k%7)+1.
    if depth <= 0:
        return ((k % 7) + 1,)
    return ((k % 5) + 1, build_weighted(depth - 1, 2 * k + 1),
            build_weighted(depth - 1, 2 * k + 2))


def walk_weighted(t, level):
    if len(t) == 1:
        return t[0] * level
    return (t[0] * level + walk_weighted(t[1], level + 1)
            + walk_weighted(t[2], level + 1))


def test_tree_treewalk():
    tree = build_weighted(TW_DEPTH, 0)  # 131071 nodes, untimed
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        for w in range(1, TW_WALKS + 1):
            acc += walk_weighted(tree, w)
        checksum = acc
        times.append(now_ns() - t0)
    print("test_tree_treewalk = %d" % checksum, file=sys.stderr)
    record("tree", "treewalk", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_tree_binarytrees()
    test_tree_treewalk()
