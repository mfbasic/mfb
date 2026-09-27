### 1. Empty candidate superset is undocumented
UNIT:      man-page:collections/isSubset
CLAIM:     "The candidate superset, of the same type as `a`. Not modified."
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-isSubset/src/main.mfb` printed `TRUE\nFALSE` for `isSubset({}, {})` and `isSubset({1}, {})`, respectively, after `mfb build … && build/man_page_collections_isSubset.out`.
SUGGESTED: The candidate superset, of the same type as `a`. Not modified. If it is empty, the result is `TRUE` only when `a` is also empty.

### 2. Float membership’s bit-pattern edge is omitted
UNIT:      man-page:collections/isSubset
CLAIM:     "`collections::isSubset` returns `TRUE` when every element of `a` is also in `b`, and `FALSE` otherwise."
VERDICT:   incomplete
EVIDENCE:  A probe with `Set OF Float { -0.0 }` and `Set OF Float { 0.0 }` printed `FALSE\nFALSE`. `src/codegen/builtins/collections/func_is_subset.rs:BODY` delegates membership to `collections::contains`; `src/codegen/builtins/collections/func_contains.rs:DESC_CONTAINS` specifies exact-bit-pattern Float membership.
SUGGESTED: Membership uses the set’s element comparison rules. In particular, `-0.0` and `0.0` are distinct Float elements for this test, even though `-0.0 = 0.0` is `TRUE`.

### 3. Error and type-requirement sentences are run together
UNIT:      man-page:collections/isSubset
CLAIM:     "`isSubset` raises no user-trappable error of its own.Both arguments must be the same `Set OF T`."
VERDICT:   misleading
EVIDENCE:  The rendered page prints `own.Both` with no separating whitespace. The source is `src/codegen/builtins/collections/func_is_subset.rs:DESC`.
SUGGESTED: `isSubset` raises no user-trappable error of its own. Both arguments must be the same `Set OF T`.