# bug-495: bounds-check elimination elides `collections::get`'s check on a stale length → OOB heap read

Last updated: 2026-09-03
Effort: small (<1h for fix (b); medium for fix (a))
Severity: HIGH
Class: security (memory safety — out-of-bounds read / heap info leak)

Status: Open (found in audit-3, Surface 3 MEM-11; reproduced live by the lead at -O0 and -O3)

Regression Test: add `tests/rt-error/collections/bounds_elim_backedge_rt` asserting `7-705-0001`.

## Summary

The plan-86-G1 bounds-check-elimination proof clears `collections::get`'s bounds
check when it can prove `i < len(L)`. The proof scans only the `FOR` loop's own
body for reassignments of `i`/`L`/`n`, but the length fact `n = len(L)` can be
established *outside* the loop and left live when the `FOR` is re-entered by an
**enclosing** loop's back edge after that enclosing loop reassigned `L` to a
shorter list. The check is then gone while the list is short, so `get` reads past
the entry table — a heap out-of-bounds read reachable from a 15-line ordinary
program, at every optimization level.

## Mechanism

`gen_list.rs:77-83` emits the only bounds check on the list `get` path and drops
it entirely when `unchecked`. The `unchecked` flag comes from
`is_provable_index_access` (`func_get.rs:153`), set by `recognize_provable_index`
(`builder_control.rs:1615-1623`):

```rust
let reassigned = crate::codegen::engine::function::collect_reassigned_locals(body);
if reassigned.contains(i) || reassigned.contains(&list) { return None; }
```

`body` is the `FOR`'s own body. The fact `end == len(L) - k` was established by
`resolve_len_list`'s `NirValue::Local(n) => self.len_of_local.get(n)` arm
(`builder_control.rs:1546-1556`) from an earlier `LET n = len(L)`. Lowering is a
single linear pass, so when the `FOR` is nested in an outer loop whose body
reassigns `L` *after* the `FOR`, the fact is still live when the inner loop is
lowered (the invalidation at `builder_control.rs:854-857` runs later, after the
code is already emitted). At runtime the outer loop's second iteration re-enters
the `FOR` with `n` still holding the original length and `L` pointing at a shorter
block — check gone.

The three mandatory negative fixtures
(`tests/rt-error/collections/bounds_elim_{reassigned,headroom,noninduction}_rt`)
all reassign *inside* the `FOR` body — the case the proof does cover. The
back-edge case is untested.

Only `collections::get` reads `is_provable_index_access` (`grep -rn
"is_provable_index_access" src/` → one call site), so this is an OOB **read**, not
a write.

## Reproduction (lead-run, live)

`spikes/audit-3/MEM-11/` — `mfb build spikes/audit-3/MEM-11 &&
./spikes/audit-3/MEM-11/build/mfb_project.out`. Observed at -O0 and -O3:

```
out=24
v=1 … v=8            # pass 0, correct
v=99 v=0 v=80 v=7099613721752949849 v=5907851599868860885 …   # passes 1-2: 14 heap words
```

24 elements from an 8→1 list; passes 1 and 2 leak adjacent heap into a
program-visible list. Control (`FOR i = 0 TO 7`, literal bound, recognizer does
not fire) raises `7-705-0001` correctly. With `List OF String` the same shape
dereferences the OOB `(offset,length)` pair and segfaults.

## Best fix

Either (a) run the no-reassign proof over the *enclosing* op sequence (thread the
enclosing loop body into `recognize_provable_index`, or clear `len_of_local` /
`provable_index_locals` for every local reassigned anywhere in the enclosing loop
body before lowering it); or (b) drop the `NirValue::Local(n) => len_of_local` arm
in `resolve_len_list` and accept only a `Call{target:"len"}` bound that
re-evaluates `len(L)` at each loop entry — a two-line change that keeps the win
for the direct `FOR i = 0 TO len(L)-1` shape. Add the back-edge negative fixture
either way.

## Non-goals

Do not weaken the three existing negative fixtures; do not disable the elision
wholesale (the plan-86-G benchmark win is real); no language-surface change.

## Prior art

None as a defect. The feature is
`planning/completed/plan-86-G-bounds-check-elim.md`, whose proof says
"`i`/`L`/`n` are NOT reassigned anywhere in the body" — the gap is that "the
body" is the `FOR`'s, not the enclosing loop's. Searched `provable_index`,
`bounds`, `elim`, `G1` across `bugs/`, `bugs/completed/`, `bugs/skipped/`,
`audit-1-*`, `audit-2-*`.
