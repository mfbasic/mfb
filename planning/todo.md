# Planning TODO

Ideas captured but not yet planned. Each entry is a note, not a design — when one
is picked up, write it into a real `plan-NN` (see the `write-plan` skill) first.

## Registry encapsulation cleanup

**Status:** idea only — do NOT start without a `plan-NN`.

**Motivation.** After plan-95 the builtin registry (`src/codegen/registry.rs`)
owns the descriptor types, but construction is still spread out and duplicated:

- Every package hand-writes a `static PKG: BuiltinModule = BuiltinModule { … }`
  struct literal (28 of them), and each also carries its *own* function-builder
  helper (`collections::native`, `general::gfn`, `strings::strings_fn`, `math::mf`,
  …) that all rebuild the same `BuiltinFunction { … }` shape.
- The registry struct fields (`BuiltinFunction`, `BuiltinModule`) are all
  `pub(crate)`, so the "registry" is an open record any package fills in directly,
  not an API. Its invariants live in scattered per-package helpers.
- plan-95 already started the fix: `BuiltinFunction::native(name, slug, intro,
  desc, errors, overloads, lower)` is the first registry-owned constructor (for
  `Implementation::Native` functions).

**Rough shape (to be designed, not committed here):**

- Add the remaining constructors on the registry types:
  - `BuiltinFunction::same(...)` / `BuiltinFunction::rewrite(symbol, ...)` for the
    other `Implementation` kinds (mirroring `BuiltinFunction::native`).
  - `BuiltinModule::new(name, doc_intro, doc_desc, functions, types, source, resolver)`.
- Migrate all 28 packages to those constructors; delete the per-package `*_fn`
  helpers.
- Once all construction goes through the constructors, make the struct fields
  private (drop `pub(crate)` on the fields, keep accessors) so the registry is a
  real API, not an open record.
- Consider whether the parallel `Lowering` (Helper/Inline) field still earns its
  place once `Implementation` carries the actual lowering (plan-95 §Open — the
  `Native` migration may make `Lowering` redundant for migrated functions).

**Guardrails when it is planned:** byte-identity gate per package (this is a
provably-neutral refactor — same emitted `.ncode`), one package per phase or small
batch, no behavior change. Naming note: prefer constructors (`::native`, `::new`)
over imperative `*_add_*` names — the descriptors are `const` values, not a
mutable registry.
