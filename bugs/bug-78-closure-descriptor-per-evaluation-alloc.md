# bug-78: every evaluation of a function value arena-allocs a descriptor that is never freed

Last updated: 2026-07-11
Effort: medium (1h–2h) to bound; large to make no-capture function values allocation-free
Severity: MEDIUM (unbounded arena growth in a loop)

**Status (2026-07-11, updated):** User chose the FULL allocation-free redesign.
Scoped in depth this session; it is a substantial cross-phase + cross-backend
feature, NOT a contained patch, so it is documented here for a dedicated staged
effort rather than rushed (rushing risks regressing the function-value/closure
semantics bug-72/73 established, across all 4 backends).

### The design (route A — static descriptor, keeps every consumer unchanged)
All indirect-call / env-access consumers read `{code@0, env@8}` from a descriptor
pointer (`builder_collection_queries.rs:1270`, `builder_emit_helpers.rs:216`,
`runtime_helpers.rs:663`). So the lowest-risk redesign keeps the descriptor shape
and makes a **no-capture** `FunctionRef` return the address of a STATIC descriptor
instead of arena-allocating one per evaluation (`builder_values.rs:357-428`).

Two ways to get a static `{&function, 0}`:
- **A1 — data→code absolute relocation:** emit the descriptor as a data object
  whose code word is a link-time pointer to the function symbol. Blocker:
  `CodeDataObject.value` is a static byte string with NO embedded-relocation
  mechanism, and `RelocIntent` has no absolute data→symbol kind. Needs a new
  `RelocIntent` (e.g. `DataAbs64`) emitted + resolved in all four backends'
  encoders and static linkers (R_AARCH64_ABS64 / R_X86_64_64 / R_RISCV_64 / Mach-O
  ABS64).
- **A2 — BSS descriptor + startup initializer:** emit a zeroed BSS descriptor per
  function-value symbol and populate its code word at startup with the existing
  text-section `adrp;add` (`push_symbol_address`), wired through the entry's
  `global_initializer_symbol` path (`entry_and_arena.rs:9`). No new relocation, but
  needs the init code generated + run before `main`.

### The integration blocker (common to A1/A2 and to any cache/bounded variant)
Descriptors are emitted at the MODULE level (`mod.rs:444+`), but which functions
are referenced as values is only known during PER-FUNCTION lowering (`CodeBuilder`,
one per function). So a fix must thread a set of function-value symbols out of each
`CodeFunction` and aggregate them at module assembly, then emit the descriptors
(+ A2's initializer) once. That cross-phase plumbing + the 4-backend work + full
re-verification of every function-value/collection-of-closures program (bug-72/73)
on all four remotes is the dedicated effort this needs.

Note: it is arena GROWTH (freed at process exit), not a leak that outlives the
process — a perf/memory-pressure issue, not a correctness defect.

### Original finding
Confirmed still present:
`NirValue::FunctionRef` (builder_values.rs) unconditionally `arena_alloc`s a
`CLOSURE_OBJECT_SIZE` descriptor per evaluation with a hardcoded zero env, so a
lambda in a loop grows the arena without bound. The allocation-free route (a bare
code pointer) needs the indirect-call lowering to distinguish a bare pointer from
a closure block, and the descriptor-reuse route needs a new data→code relocation
kind the linker model lacks — both are cross-backend design changes affecting all
5 targets and must not regress bug-73's shared-closure semantics. Needs a plan.
The arena is freed at process exit, so this is a growth/perf issue, not a leak
that outlives the process.

Evaluating a `FunctionRef` or a `Closure` allocates a 16-byte descriptor from the
arena on **every evaluation**. Closure objects are arena-lifetime by design — freed
with the arena, never individually (see `src/docs/spec/memory/09_closures.md`) — so
a lambda created inside a loop grows the arena without bound.

Measured: 2,000,000 fresh lambdas in a loop cost roughly 259 MB.

A lambda with **no captures** needs no allocation at all: it is a bare code pointer.
It allocates anyway.

## Discovery

Found while implementing bug-73 (commit 41578ef3), whose runtime leak check proved
the *collection* storage is bounded while isolating this as a distinct, pre-existing
cost. bug-73's diff does not touch closure creation.

## Failing Reproduction

```basic
IMPORT io

FUNC apply(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer
  RETURN f(x)
END FUNC

FUNC main AS Integer
  MUT total AS Integer = 0
  FOR i AS Integer = 1 TO 2000000
    total = total + apply(LAMBDA(v AS Integer) -> v + 1, i)   ' no captures
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
```

RSS grows to hundreds of MB. Expected: flat — the lambda captures nothing.

## Root Cause

The lowering allocates a descriptor unconditionally, and the arena-lifetime
ownership rule means it is never reclaimed before process exit.

## Goal

Two separable outcomes, in order of value:

1. A function value with **no captures** allocates nothing: it lowers to a static
   function descriptor (or a bare code pointer), so the reproduction above is flat.
2. A capture-carrying closure created in a loop does not grow the arena without
   bound.

### The blocker on (1)

A static function descriptor in the data segment must hold the callee's code
address, which needs a **data-segment → code relocation**. The linker model does not
currently support that relocation kind across the five backends. Either add it, or
lower a no-capture function value to a bare code pointer with no descriptor at all
(preferred if the call site can tell a bare pointer from a closure block — check
whether the indirect-call lowering bug-72 (28c9769e) validated already can).

### The shape of (2)

Under the reference semantics the user chose for bug-73, a closure is shared and
cannot be freed at the creation site — some other binding may still hold the
pointer. Bounding it means either scope-tracking the closure block like an owned
value when it provably does not escape, or an escape analysis. Do not regress
bug-73's guarantee that a collection never frees a closure it merely points at.

## Blast Radius

- The `FunctionRef`/`Closure` lowering; `src/docs/spec/memory/09_closures.md`;
  the linker relocation vocabulary if (1) takes the descriptor route.

## Phases

### Phase 1 — measure and split

- [ ] Pin the no-capture and capture-carrying costs separately with RSS numbers.
- [ ] Determine whether the indirect-call lowering can consume a bare code pointer.

### Phase 2 — no-capture lambdas allocate nothing

- [ ] Bare code pointer, or a static descriptor plus the new relocation kind.

### Phase 3 — bound the capture-carrying case

- [ ] Escape analysis or non-escaping scope tracking.

### Phase 4 — validation

- [ ] RSS flat across a 10x iteration increase for the no-capture case.
- [ ] bug-73's collection-of-closures tests still pass; no closure freed underneath
      a live pointer (Guard Malloc).
- [ ] `scripts/test-accept.sh`.

## Summary

Every lambda evaluation allocates an arena descriptor that is never freed, so a
lambda in a loop grows memory without bound — even one that captures nothing and
should be a bare code pointer.
