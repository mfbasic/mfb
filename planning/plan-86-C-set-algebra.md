# plan-86-C — set-algebra (in-place add)

Sub-plan **C** of [plan-86](plan-86-benchmark-perf.md). **DONE via C2 alone; C1 proven unnecessary.**

**Covers (7 P1):** set (Fixed) union (110), symmetricDifference (70), toSet (69), intersection (17),
difference (17); set (Dynamic) union (7.4), add (5.07).

## Root cause
The set-algebra bodies are interpreted `FOR EACH … result = collections::add(result, x)` loops over a
native but **whole-set-copy** `add` (`collection_mutate.rs:391` → `copy_collection_tight`, which also marks
buckets not-ready so the next probe rebuilds the index O(N)). So each add is O(N) → each op is O(n²).

## Fixes
- [x] **C2 — in-place `MUT` set `add`.** New `try_inplace_set_add_assign` (`builder_inplace_assign.rs`, wired
  into the `builder_control.rs` assign-dispatch chain) recognizes `name = collections::add(name, x)` on a
  uniquely-owned MUT Set local and inserts into the live buffer via `lower_map_set_in_place`, skipping the
  copy. Each add is amortized O(1) → each op O(n). Value semantics hold (every bind/assign copies, so the
  named local has no live alias); a live `FOR EACH` over the set is excluded (bug-142). The exact set-add
  sibling of the landed list-append-in-place path. **Retired all 7 rows:** union 110.7→**0.69**,
  symmetricDifference 70.4→1.10, toSet 69.2→1.03, intersection 17.1→0.54, difference 17.0→0.53; set (Dynamic)
  union 7.43→0.17, add 5.07→0.046 — all ≤ 5 ms = complete; checksums unchanged. Commit: `13217bfca`.
- [x] ~~**C1 — native one-pass set-algebra builders**~~ — **moot: proven unnecessary.** C2 alone dropped every
  op to ≤ 1.1 ms; the interpreted bodies are already the right algorithm once `add` is O(1), so separate
  native builders would be pure redundancy (measured, not a hunch).

## Acceptance
`set (Fixed)`/`(Dynamic)` checksums unchanged + set fixtures + `scripts/artifact-gate.sh`. Landed:
`set-algebra-rt` + `set-behavior-rt` pass with unchanged goldens; new `set-inplace-add-rt` pins value
semantics (`orig=1,2 copy=1,2,3` — a bind copies), idempotence, 1000-add geometric growth. Gate-clean (no
byte-identity fixture uses set algebra; scoped collections gate 0-diff).

## Corrections
- **C2 alone retired all 7 P1 rows; C1 is unnecessary** — the O(n²) was entirely the whole-set copy inside
  `add`, not the interpreted loop structure. The set `State-*` matrix rows (whole-record rebuild, bug-430)
  still stand; C2 helps their `add` but the STATE-mutation cost is the bug-430 residual.
