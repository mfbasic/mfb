# bug-782: a `collections::set` on a copied list field that fails its bounds check leaks the copy

Last updated: 2026-09-29
Effort: small–medium (guess until traced)
Severity: MEDIUM — a leak per failing call (unbounded under retry); output correct
Class: Memory-safety (leak)

Status: Fixed
Regression Test: `tests/runtime/rt_failing_list_update_frees.rs`

`LET t AS List OF Float = collections::set(p.trail, slot, 1.0)` — where `p` is a
record bound from `collections::get` and `slot` is out of range — raises
`ErrIndexOutOfRange` as it should, but the copy of `p.trail` that `set` works on is
never freed when the error leaves the statement: 64 B per failing call. Found by the
bug-766 A agent's adversarial probe (`/tmp/b766A-p3`); pre-existing (the pre-bug-766
compiler shows the same).

**The single correct behavior a fix produces:** a program that makes that failing
`set` N times under a `TRAP` ends with `alloc_calls = free_calls` (apart from a
module-level global's own block) and no growth in `live_bytes` with N.

## STATUS: FIXED (`9e18ce852`, goldens + spec `833ef638b`; gate: suite 276 ok, artifact-gate 0 diffs, acceptance 1582 at -O1 and -O3)

**Mechanism (traced; neither hypothesis).** Not a pending-temporary or a handover
issue: a plain local list leaks the same way. Two builtins allocated before their
range check, and `raise_error`'s exit cannot free a block it never saw:

- `func_set.rs:lower_set`, fixed-width element — bug-365's tight copy
  (`copy_collection_tight`) runs, then `lower_list_set_in_place` range-checks the
  copy. 64 B per failing call for a 2-`Float` list.
- `func_insert.rs:lower_insert` — `collection_argument_as_list_slot` builds the
  singleton list, then `lower_list_insert_collection` range-checks. 48 B per call.

`set` on a variable-width element (bug-147.5 already checked first via `removeAt`)
and `removeAt` did not leak. A `Map` `set` cannot fail its range, so it is out of
scope.

**Fix.** `gen_mutate.rs:emit_list_index_range_check` runs the same check
(`0 <= index < count`, `<= count` for `insert`) first and raises the same
`ErrIndexOutOfRange`. The later check stays (it is shared with the in-place
callers). The only cost is about 27 instructions per call site, measured on
`#regex_setCap` and the collections cover.

**Test.** `rt_failing_list_update_frees` covers 13 spellings under both a
function-level and an inline `TRAP`:

- `set`/`insert` on a field copy, a local, a parameter field, a freshly built
  list and a built `String` item, plus a negative index;
- `set` on a `String` list and `removeAt` as controls.

Before the fix, 7 of the original 9 cases leaked 1 block per failing call. The
repro now ends with only the global `gps`'s 48 B at -O1 and -O3.

## Failing Reproduction

```basic
IMPORT io
IMPORT collections

TYPE P
  age AS Integer
  trail AS List OF Float
END TYPE

MUT gps AS List OF P = []

FUNC bumpCopy(i AS Integer, slot AS Integer, text AS String) AS Integer
  LET p AS P = collections::get(gps, i)
  LET t AS List OF Float = collections::set(p.trail, slot, 1.0)
  RETURN p.age
END FUNC

FUNC main() AS Integer
  gps = [P[age := 1, trail := [0.0, 0.0]], P[age := 2, trail := [0.0]]]
  LET a AS Integer = bumpCopy(0, 9, "1") TRAP(e)
    RECOVER 0
  END TRAP
  io::print("fail " & toString(a))
  gps = []
  RETURN 0
END FUNC
```

`mfb build -O 1 --debug` (compiler before bug-766): prints `fail 0`; `alloc_calls 13`,
`free_calls 11`, `live_bytes 112` — the module-level `gps` accounts for 48 B (one
block, by design); the other 64 B are the leaked copy.

## Root Cause

Not traced. Hypotheses: (1) the copy of the field `set` materializes for its
copying form is not registered as a pending temporary, so the inline bounds-error
exit (`builder_exits.rs:emit_inline_error_exit` → `emit_call_error_exit`, which frees
pending temps) never sees it; (2) the copy is registered, but after the bounds check
that raises. Phase 1 reads the `.ncode` of `bumpCopy` for the raise path.

## Goal / Non-goals

- The repro and its variants (a local list, a `Map` field with `set`, `insert` /
  `removeAt` out of range on a copied field) free the copy on the error path.
- No language-spec or memory-model change; the error, its code and location, and
  every success path are unchanged.

## Phases

- [x] Phase 1 — RED runtime test (N vs 2N failing calls, `--debug` counters), trace.
- [x] Phase 2 — free the copy on the raise path.
- [x] Phase 3 — goldens, full suite.
