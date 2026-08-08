# plan-86-B — reduce / reduceRight accumulator

Sub-plan **B** of [plan-86](plan-86-benchmark-perf.md). **DONE (B1+B2); B3 deferred to K.**

**Covers (2 P1, biggest ms):** list (Dynamic) reduce (3048), reduceRight (877).

## Root cause
`reduce`'s native lowering deliberately never freed intermediate accumulators (to avoid a UAF when the
reducer aliases the item — `builder_collection_queries.rs:3398-3416`, plan-26-B), so ~500k intermediate
Strings leaked into the arena — a 3.5× transient-churn penalty on top of the O(n²) `acc & s` concat (fair;
Python's fold does the same). `reduceRight` was interpreted.

## Fixes
- [x] **B1** — free the *previous* accumulator each success iteration, guarded by runtime pointer-equality
  against the item and the old accumulator (seed starts not-owned). Value semantics make pointer equality
  an exact aliasing test, so the bug-307 adopt-item / return-acc cases stay safe; the failure path leaks at
  most one in-flight item. Heavy-fold probe: peak RSS 869,908,480 → 3,702,784 B (~235×), wall 0.34→0.18 s.
  (Landed pre-session; see git log `plan-86-B`.)
- [x] **B2** — native `reduceRight` (reverse-walk twin of `reduce` sharing B1's reclamation via a `reverse`
  flag + `initialize_collection_loop_slots_reverse`/`advance_collection_loop_reverse`); moved from
  source-generic to `NATIVE_MEMBERS`, `.mfb` body deleted, man citations repointed. (Landed pre-session.)
- [~] **B3 (deferred to [plan-86-K](plan-86-K-cow-layout.md))** — an in-place growing accumulator (append
  the item's bytes to a uniquely-owned `acc` in place) would make the fold O(n), but the reducer's `acc & s`
  is *user* code, so it needs K's general uniquely-owned-mutation analysis. reduce/reduceRight still lose to
  Python (fair O(n²) floor); this is the only lever left and it belongs to K.

## Acceptance
`reduce`/`reduceRight` checksums unchanged + a reducer-aliases-item UAF fixture + `scripts/artifact-gate.sh`.
Landed: `tests/rt-behavior/collections/reduce-accumulator-reclaim-rt` (both directions, every aliasing
shape, 500-round UAF stress), `tests/syntax/collections/func_collection_reduceRight_invalid`;
`hof-string-item-lifetime-rt` pins the reducer-adopts-item case.

## Corrections
- B1's reclamation is a **runtime** pointer-equality guard, not a compile-time escape analysis (the reducer
  is an opaque function value, so aliasing is only knowable at run time). B2's migration touched descriptor
  authority + resolver + `native_builtin_target` + `inline_builtin_raw_supported` + two dispatch sites +
  `.mfb` deletion + man-citation repointing (the `man_citations_resolve` test caught the dangling citation).
- B3 was reclassified from "conditional structural note" to **blocked on K**.
