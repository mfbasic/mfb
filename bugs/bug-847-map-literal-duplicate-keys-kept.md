# bug-847: a `Map OF K TO V { … }` literal with a repeated key builds a map holding that key twice

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/collections/map-literal-duplicate-key-rt (to add)

**Owner decision (2026-10-05):** a map literal with a repeated key is a compile error. Insert/last-wins semantics is
rejected. Where keys are compile-time constants, equal keys are a located diagnostic. The
fix must also define (and the spec state) what happens when keys are non-constant
expressions that collide at run time. Raise this with the owner before picking a runtime
behavior; do not silently collapse.

A map literal that names the same key more than once is lowered entry-by-entry
into the packed collection block with no de-duplication, so the resulting `Map`
holds duplicate keys. `len` counts every copy, `FOR EACH` yields the key twice
with both values, `collections::keys`/`values` return both, `collections::get`
returns the FIRST copy's value, and a later `collections::set` of that key
updates only one of the copies (the other keeps its stale value). The `Set OF T`
literal, by contrast, inserts each element so duplicates collapse. The bug is
silent: no diagnostic, and every reader reports a different "truth" about the
map.

The single correct behavior a fix produces: a map never holds two entries with
equal keys. Either the literal is rejected at compile time when two constant keys
are equal (owner decision), or — matching `collections::set` and the set literal
— each entry is inserted so a repeated key collapses to one entry (last value
wins, as a sequence of `collections::set` calls would give). Recommended: insert
semantics (keys may be non-constant expressions, so a compile-time check cannot
catch every case anyway).

References:

- `src/docs/spec/language/12_collections.md` (Collections) — the example
  `LET m = Map OF String TO Integer { "a" := 1, "b" := 2 }` and "`Set OF Integer
  { 1, 2, 3 }` ' literal (duplicates collapse)"; the spec says nothing about a
  repeated Map key, but a Map with duplicate keys contradicts the `Map` type
  itself (`src/docs/spec/language/04_types.md` §4 collection type forms) and the
  map-key equality rule in `src/docs/spec/memory/05_collections.md`.
- Found during hardening batch B03, topic `language/12_collections`.

## Failing Reproduction

```
mfb init d2   # then replace src/main.mfb with:
IMPORT io
IMPORT collections
FUNC main() AS Nothing
  MUT m = Map OF String TO Integer { "a" := 1, "a" := 2 }
  io::print("len=" & toString(len(m)))
  m = collections::set(m, "a", 9)
  io::print("after set len=" & toString(len(m)))
  FOR EACH e IN m
    io::print(e.key & "=" & toString(e.value))
  NEXT
  LET vs = collections::values(m)
  io::print("values=" & toString(len(vs)))
END FUNC
```
`/tmp/hardening/mfb build && ./build/d2.out`

- Observed:
  ```
  len=2
  after set len=2
  a=9
  a=2
  values=2
  ```
- Expected (insert semantics): `len=1`, `after set len=1`, one line `a=9`, `values=1`.

Also observed (same build): `Map OF String TO Integer { "a" := 1, "b" := 2, "a" := 3 }`
gives `len=3`, `get(m,"a")=1`, FOR EACH `a=1 b=2 a=3`, `keys` length 3, and
`collections::removeKey(m, "a")` then leaves `len=1`.
`Map OF Integer TO Integer { 1 := 1, 1 := 2 }` gives `len=2`;
`Map OF Boolean TO Integer { TRUE := 1, FALSE := 0, TRUE := 5 }` gives `len=3`, `get(TRUE)=1`.

Contrast (works): `Set OF Integer { 3, 1, 2, 3, 1 }` has `len=3`;
`collections::set` on an existing key replaces in place (len unchanged).

## Root Cause

`src/codegen/collection/layout/builder_collection_layout.rs:lower_map_literal`
lowers each `(key, value)` into a `CollectionValueSlot` and hands the whole vector
to `lower_collection_values`, which sizes and writes one entry per slot
(`count = slots.len()`) — there is no equality test between keys. The sibling
`lower_set_literal` instead starts from an empty set and calls
`lower_map_set_in_place` per element, which is why sets collapse duplicates. No
verifier rule (`src/ir/verify/values.rs:check_value_depth`, `IrValue::MapLiteral`
arm) checks constant-key uniqueness either.

## Goal

- After the fix, every Map literal yields a map whose `len` equals the number of
  distinct keys, and `get`/`FOR EACH`/`keys`/`values` agree.

### Non-goals (must NOT change)

- Layout of a map literal with distinct keys (byte-identical goldens).
- The bitwise key-equality rule for `Float` keys (`+0.0` and `-0.0` stay distinct).

## Blast Radius

- `builder_collection_layout.rs:lower_map_literal` — fixed by this bug.
- `lower_set_literal` — unaffected (already inserts).
- Module-level `LET G AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1, "a" := 2 }`
  (data-object path, `src/codegen/memory/data/data_objects.rs`, `NirValue::MapLiteral`
  arm) — same bug, measured: prints `len=2 a=1`. Fixed by this bug too.

## Fix Design

Either route the map literal through the same per-entry in-place insert as the
set literal (simplest, last-wins) or de-duplicate equal constant keys at lowering
and keep the packed fast path for the distinct case. Rejected: silently keeping
first-wins — it disagrees with `collections::set` semantics.

## Validation Plan

- Regression test: rt fixture printing len/FOR EACH/get for duplicate String,
  Integer, Boolean, record keys.
- Doc sync: state the repeated-key rule in `language/12_collections.md`.

## Open Decisions

- Repeated key: last-wins insert (recommended) vs. compile-time error for equal
  constant keys.
