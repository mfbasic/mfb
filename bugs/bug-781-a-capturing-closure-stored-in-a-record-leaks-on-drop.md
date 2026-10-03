# bug-781: a capturing closure stored in a record field leaks when the record is dropped

Last updated: 2026-10-03
Effort: large — a memory-model change to function values, not a drop-path fix (guess)
Severity: MEDIUM — documented arena-lifetime accumulation, not a spec violation
Class: Other (memory-model limitation)

Status: Open — fix decided (single-owner closures, below); ready to implement
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

## What is allocated, and when

Two events, kept distinct throughout this document:

- **Create** — the program evaluates a `LAMBDA` expression. Every create
  allocates two blocks: the 16-byte closure object `{code, env}` and the capture
  environment (one 8-byte word per capture, holding the captured values).
  `mfb spec memory closures`, *Function Reference vs Closure*.
- **Call** — the function value is invoked (`g(1.0)`). A call allocates nothing;
  it reads the captured values out of the environment.

The environment's **layout** (which captures, their types) is fixed per `LAMBDA`
in the source. Its **contents** differ per create, so each create needs its own
environment. In the repro the `LAMBDA` is created once per loop iteration with a
different `factor`, so 10 iterations make 10 closure objects and 10 environments
(the measured `alloc_calls − free_calls = 20`):

```text
iteration 1:  h₁.f, g₁   ──► closure₁  ──► env₁  (factor = 1.0)
iteration 2:  h₂.f, g₂   ──► closure₂  ──► env₂  (factor = 2.0)
...
iteration 10: h₁₀.f, g₁₀ ──► closure₁₀ ──► env₁₀ (factor = 10.0)
```

Within one iteration, the `f` parameter of `keep`, `h.f` and `g` all hold the
**same** pointer to that iteration's closure — copying a function value copies
only the 8-byte pointer (*Ownership and Reference Semantics*). At the end of each
iteration `h` and `g` are dropped, but closureₙ and envₙ are not, because no
holder owns them: freeing on one holder's drop would leave the others dangling.
They stay until the arena is torn down — for `main`, program exit.

## FINDING (fix-bug, 2026-09-29): the leak is the specified behaviour

Reproduced (`/tmp/r781.py`, `mfb build --debug`, 10 iterations): every record that
holds a capturing closure keeps it — built via `keep()` or directly, with or without
a field read, and inside a `List` (20 blocks / 320 B each); a capturing closure bound
to a plain local is freed (0). No record/list drop path frees a `FUNC` field.

That is what the spec says. `mfb spec memory closures`, *Ownership and Reference
Semantics*, gives function values pointer semantics and the **arena-lifetime
closure rule**: a closure "is freed only when that arena is torn down — never
individually". `mfb spec memory heap-values` *Records* excludes function fields
from record copy/drop: "resource and function pointers retain their separate
ownership rules". The only individual frees today are the scope-drop of a provably
non-escaping closure (plan-77 M6, `builder_owned_cleanup.rs:emit_closure_drop`)
and the bug-572 / bug-767 C free of a closure passed only to a non-retaining
callback position.

## Fix (decided 2026-10-03): single-owner closures — no reference counting

Reference counting is **not** an option: `mfb spec language memory-semantics`
(§14) and design principle 7 rule out "tracing GC, reference counting". It is
also unnecessary. The leak exists only because function values are shared
pointers with no owner; the fix gives them one owner per copy, the same rule as
every other ordinary value:

1. **Copy duplicates the environment.** Copying a function value (into a record,
   a binding, an argument, a collection) deep-copies its environment, so every
   holder owns its own. Moving at last use (§14.1) avoids most copies.
2. **Drop frees it.** Dropping a holder frees its closure object and environment.
3. **The environment describes itself.** A `FUNC(Float) AS Float` field type does
   not say what was captured, so record copy/drop cannot generate the env
   copy/drop from the field type. At each `LAMBDA` the compiler does know the
   capture types, so it generates a copy routine and a drop routine for that
   `LAMBDA`'s environment — treating the environment as an anonymous record of the
   captures and reusing the existing record deep-copy / graph-drop machinery.
   Each environment carries a pointer to its `LAMBDA`'s routines (e.g. a header
   word).
4. **Record copy/drop calls them.** The record copy and drop walkers, on reaching
   a `FUNC` field, call the routines the environment points to instead of
   skipping the field.

**Why copying is safe:** a created closure cannot change — captures are `LET`
values copied at create (§14.4), and the only by-reference capture (`MUT` in a
`collections::forEach` action) never escapes the call. Functions are not
comparable (`mfb spec language types` §4.11), so no program can observe a copy
versus the original. Closures behave like records (immutable data, no identity,
cleanup is only memory), not like resources (shared identity, `CLOSE` side
effect), so record ownership is the right model.

**Optional, separable:** the 16-byte `{code, env}` pair could be stored inline in
the holder (two words) instead of as a separate block, saving one allocation per
create. That changes the function-value ABI everywhere and is not required for
the fix. The environment itself cannot be inlined into a record slot generally:
a `FUNC` field can hold any `LAMBDA` of that signature, whatever its capture count.

### Copy rule

A function value is copied only when two live holders each need it. Everywhere
else no copy is made:

- **Last use moves** (§14.1): `Holder[f := f]` in `keep` and a closure returned
  or stored at its last use transfer ownership.
- **Non-retaining call positions borrow**: a callback position proven not to
  retain its argument (`is_nonescaping_callback_arg`, the bug-572 / bug-767 C
  positions) uses the caller's closure without copying it.

So a large capture is copied only when the program really keeps two holders.

### Plan scope

Every holder of a function value follows the same rule, or one type has two
ownership models: locals, arguments, returns, `List`/`Map` slots, union and
`Result` payloads (stored as pointers today, *heap-values* `Result`), `STATE`
fields, thread messages, and runtime-retained handlers such as
`http::Route.handler`.

### Resolved: auxiliary environment words never reach an escaping closure

The environment can carry addresses into the creating frame after the captures
(`builder_values.rs:lower_value`, `NirValue::Closure` arm): the borrowed
self-update scratch (`self_update.rs:scratch_closure_captures`), shared String
capacity shadows (`string_self_update.rs:string_shadow_captures`), and the
`os::appResourcePath` base cache (`string_self_update.rs:lambda_shares_resource_base`).
Copying an environment would copy those addresses verbatim, which is only safe
if such a closure never outlives its creator. It never does:

- All three are added only for a lambda whose body holds a self-update
  assignment.
- A lambda can assign only to a by-reference captured `MUT` local. Assigning a
  module-level `MUT` is rejected: `LAMBDA(v AS String) -> g = strings::replace(g, tag, v)`
  with `g` global fails `TYPE_UNKNOWN_VALUE` ("Assignment target `g` is not a
  local binding") — probe built with `target/release/mfb build`, 2026-10-03.
- A `MUT` capture is accepted only as a direct `collections::forEach` action
  (§14.4, `is_nonescaping_callback_arg`). Storing the same lambda over a local
  `MUT g` in a record field fails `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` (same probe).

Such a closure is therefore never stored, returned, or copied, and the fix needs
no handling for these words. There is no dangling pointer at HEAD.

**Decision (owner, 2026-10-03):** fix the leak with the single-owner closure
model above. The arena-lifetime closure rule is retired; `mfb spec memory
closures`, `mfb spec memory heap-values` (*Records*, *Result*), and
`mfb spec language memory-semantics` §14.1 / §14.4 change with it.

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

Function values have pointer semantics with no owner (*Ownership and Reference
Semantics*), and record copy/drop skips function fields (*heap-values* *Records*).
Several holders can share one closure, so no holder's drop may free it, and it
survives until arena teardown. Confirmed by the FINDING reproduction: direct
construction without `keep` and without the `h.f` read leaks identically, so the
original hypothesis 2 (aliasing through the parameter or field read) is not the
cause.

## Goal

- The repro and its variants (direct construction, field read, record in a list,
  record reassigned) end with `live_bytes 0`, output unchanged.

### Non-goals (must NOT change)

- Closure call semantics, capture-by-value, and a non-capturing `FUNC` value (a
  static descriptor, no allocation) are unchanged.
- No reference counting and no GC: ownership stays single-owner, lexical drop.
- Forbidden: freeing an environment another holder still uses. Under the fix no
  two holders share one, so this holds by construction.

This fix **does** change the memory model for function values (pointer
semantics → single-owner value semantics); the spec edits above are part of it.

## Blast Radius

Every place a function value is stored, copied, or dropped (*Plan scope*): the
closure constructor (`builder_values.rs:lower_value`, `NirValue::Closure`), the
owned-value copy and drop paths that exclude function types today
(`builder_values.rs:lower_value_owned`, `is_freeable_flat_value`), record /
union / `Result` / collection copy and drop walkers
(`graph_drop.rs:lower_graph_drop_walker`), `STATE` fields, thread messages, and
runtime helpers that retain a handler (`http::Route.handler`). Phase 1 completes
this list from the code before Phase 2 starts.

## Phases

### Phase 1 — failing test + audit

- [ ] Runtime leak test (model: `tests/runtime/rt_callback_failure_frees.rs`'s
      `--debug` counters) over the variants: `keep` + field read, direct
      construction, record in a `List`, record reassigned, closure in a `Map`
      value, closure in a union / `Result` payload, closure returned from a
      function. RED today (`live_bytes > 0`), output unchanged.
- [ ] Audit every holder kind in *Blast Radius*; list each copy and drop site.

Commit: —

### Phase 2 — self-describing environment

- [ ] At each `LAMBDA`, generate a copy routine and a drop routine for its
      environment (the captures as an anonymous record, using the existing
      deep-copy / graph-drop machinery).
- [ ] Add an environment header word pointing at that `LAMBDA`'s routines.
      A non-capturing `FunctionRef` (static descriptor, `env = 0`) needs none.

Commit: —

### Phase 3 — single owner at every holder

- [ ] Copying a function value copies the closure object and calls the env copy
      routine; last use moves (§14.1); non-retaining callback positions
      (`is_nonescaping_callback_arg`) borrow.
- [ ] Dropping a holder frees the closure object and calls the env drop routine:
      bindings, record / union / `Result` fields, collection elements, `STATE`
      fields, thread messages, runtime-retained handlers.
- [ ] Remove the function-type exclusions from `lower_value_owned` and
      `is_freeable_flat_value`.

Commit: —

### Phase 4 — spec

- [ ] `mfb spec memory closures`: replace *Ownership and Reference Semantics* and
      the arena-lifetime closure rule with single-owner semantics; document the
      env header and routines.
- [ ] `mfb spec memory heap-values`: function fields/payloads are owned (*Records*,
      *Result*).
- [ ] `mfb spec language memory-semantics` §14.1 / §14.4: function slots copy the
      closure, not a pointer; dropping a binding reclaims it.
- [ ] Gate: `cargo build`, `cargo test --bin mfb spec`.

Commit: —

### Phase 5 — full validation

- [ ] Phase 1 tests GREEN; goldens (inspect one before accepting any diff); full
      suite; acceptance -O1/-O3.

Commit: —
