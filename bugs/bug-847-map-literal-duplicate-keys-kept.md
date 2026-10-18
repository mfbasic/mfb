# bug-847: a `Map OF K TO V { … }` literal with a repeated key builds a map holding that key twice

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/syntax/collections/map-literal-duplicate-constant-key-invalid (constant-key half); the run-time half has none yet

## Cause (coordinator, bug-staging-24)

- Rule: a map never holds two entries with equal keys; a map literal naming the same constant key twice is a located compile error (owner decision 2026-10-05, done on bug-staging-24). OWNER RULING 2026-10-18: non-constant keys that turn out equal at run time RAISE `ErrInvalidArgument` (77050002) at the literal, and no map is built (not last-wins).
- Where: no verifier rule checks constant-key uniqueness (`src/ir/verify/values.rs:check_value_depth`, `IrValue::MapLiteral` arm), and `builder_collection_layout.rs:lower_map_literal` writes one entry per slot with no equality test.
- Fix (remaining half): `builder_collection_layout.rs:lower_map_literal` inserts each entry through the map's own key lookup (as `lower_set_literal` does) and raises `ErrInvalidArgument` when a key is already present, freeing what it built; `mfb spec memory`/language map-literal text and the literal's declared errors state it.

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

## Partial fix (constant keys only; the doc stays open)

Done: `src/ir/verify/values.rs:check_value_depth`, `IrValue::MapLiteral` arm, now refuses a
literal that names the same constant key twice with the new rule `TYPE_DUPLICATE_MAP_KEY`
(`2-203-0146`, registered in `src/rules/table.rs` and `mfb spec diagnostics rule-codes`). A key
is a constant when it is a literal or a negated numeric literal (`constant_key`); `Float` keys
compare bitwise (`0.0` and `-0.0` stay distinct), `-0` equals `0` for the other numeric types.
It applies to a local and to a module-level `LET G AS Map ... { "a" := 1, "a" := 2 }` alike
(measured: both report). Spec: `src/docs/spec/language/12_collections.md` (the bullet naming the
rule; it states that a run-time collision of non-constant keys is not yet specified).

Limits: the diagnostic is located at the literal's statement line, not at the second key (the IR
carries a line per statement, not per key). One report per repeated key occurrence.

NOT done (waits on the owner, asked 2026-10-17): the run-time collision of non-constant keys
(`ErrInvalidArgument` recommended, or last-wins). `lower_map_literal`
(`src/codegen/collection/layout/builder_collection_layout.rs`) still writes one entry per slot
with no equality test; nothing there was changed. The rt-behavior fixture in the Validation Plan
belongs to that half.

Test: fixture `tests/syntax/collections/map-literal-duplicate-constant-key-invalid` (String,
Integer, Boolean and negated-Integer duplicates, plus distinct-key literals that must build).
RED on the pre-fix compiler: all five duplicate literals built with no diagnostic. GREEN: five
reports, one per literal, none for the distinct ones. Ran `bash scripts/test-fast.sh --accept
'map-literal-duplicate-constant*'`.

Goldens expected to change: none existing (a repeated constant key never compiled in a passing
fixture except by mistake; the new fixture's own `golden/build.log` only).
