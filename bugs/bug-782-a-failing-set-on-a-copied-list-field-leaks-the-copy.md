# bug-782: a `collections::set` on a copied list field that fails its bounds check leaks the copy

Last updated: 2026-09-29
Effort: small–medium (guess until traced)
Severity: MEDIUM — a leak per failing call (unbounded under retry); output correct
Class: Memory-safety (leak)

Status: Open
Regression Test: to be added (Phase 1)

`LET t AS List OF Float = collections::set(p.trail, slot, 1.0)` — where `p` is a
record bound from `collections::get` and `slot` is out of range — raises
`ErrIndexOutOfRange` as it should, but the copy of `p.trail` that `set` works on is
never freed when the error leaves the statement: 64 B per failing call. Found by the
bug-766 A agent's adversarial probe (`/tmp/b766A-p3`); pre-existing (the pre-bug-766
compiler shows the same).

**The single correct behavior a fix produces:** a program that makes that failing
`set` N times under a `TRAP` ends with `alloc_calls = free_calls` (apart from a
module-level global's own block) and no growth in `live_bytes` with N.

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

- [ ] Phase 1 — RED runtime test (N vs 2N failing calls, `--debug` counters), trace.
- [ ] Phase 2 — free the copy on the raise path.
- [ ] Phase 3 — goldens, full suite.
