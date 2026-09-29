# bug-781: a capturing closure stored in a record field leaks when the record is dropped

Last updated: 2026-09-29
Effort: medium (1h–2h) — guess until the drop path is read
Severity: HIGH — an unbounded leak (2 blocks per record) for a common shape: a
record with a `FUNC` field built from a capturing `LAMBDA`
Class: Memory-safety (leak)

Status: Open
Regression Test: to be added (Phase 1)

A record whose `FUNC`-typed field holds a **capturing** `LAMBDA` never frees that
closure when the record goes out of scope. Every such record leaks the closure's
two blocks (32 B for one captured `Float`). Output is correct, so the leak is
silent. Found while fixing bug-767 C (a closure *passed* to a callback position is
freed correctly there; a closure *stored* in a record is not).

**The single correct behavior a fix produces:** a record holding a capturing
closure frees the closure exactly once when the record is dropped (scope end,
reassignment, or as part of a containing value), so a program that builds N such
records ends with `alloc_calls = free_calls` and `live_bytes 0`.

References:

- `mfb spec memory` — scope-drop frees: a record owns its fields' values; a closure
  value owns its captured environment.
- Found in bug-767 (`bugs/completed/bug-767-*.md`, sub-issue C).

## Failing Reproduction

```basic
IMPORT io

TYPE Holder
  f AS FUNC(Float) AS Float
END TYPE

FUNC keep(f AS FUNC(Float) AS Float) AS Holder
  RETURN Holder[f := f]
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    LET h AS Holder = keep(LAMBDA(v AS Float) -> v + factor)
    LET g AS FUNC(Float) AS Float = h.f
    n = n + toInt(g(1.0))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
```

`mfb build --debug` (main at the bug-767 branch base, and the pre-bug-767
compiler — identical): prints `65`; `alloc_calls − free_calls = 20`,
`live_bytes 320` over 10 iterations — 2 blocks (one closure) per record.

Contrast (no leak): the same capturing `LAMBDA` passed to `collections::sortBy`,
`collections::transform`, or a user `FUNC` that forwards it to `transform`
(bug-767 C).

## Root Cause

Not yet traced. Hypotheses, in order:

1. The record drop (`emit_owned_value_drop` / the graph or flat drop walker) treats
   a `FUNC` field as a plain pointer word and never runs the closure drop
   (`emit_closure_drop`) on it.
2. Constructing `Holder[f := f]` from a parameter, or reading `h.f` into `g`,
   aliases the closure without transferring ownership, so no owner frees it.

Phase 1 distinguishes them: the same record built directly in `main`
(`LET h AS Holder = Holder[f := LAMBDA(v AS Float) -> v + factor]`) with no
`keep` and no `h.f` read.

## Goal

- The repro and its variants (direct construction, field read, record in a list,
  record reassigned) end with `live_bytes 0`, output unchanged.

### Non-goals (must NOT change)

- No language-spec or memory-model change: this makes record drops honour the
  existing ownership rule. Closure call semantics, capture-by-value, and a
  non-capturing `FUNC` value (no environment block) are unchanged.
- Forbidden: freeing a closure that another owner still holds (the retaining
  positions, e.g. `http::Route.handler`, and a closure copied out of the record
  into a live local).

## Blast Radius

To be completed in Phase 1: every drop path for a value that can contain a `FUNC`
field — records, unions, list/map elements of such records, `STATE` fields.

## Phases

### Phase 1 — failing test + audit

- [ ] Runtime leak test (model: `tests/runtime/rt_callback_failure_frees.rs`'s
      `--debug` counters) over the variants above; RED today.
- [ ] Trace the mechanism; confirm or drop each hypothesis; complete Blast Radius.

Commit: —

### Phase 2 — the fix

- [ ] Drop the closure environment where the record's field is dropped.

Commit: —

### Phase 3 — full validation

- [ ] Goldens (inspect one), full suite, acceptance -O1/-O3.

Commit: —
