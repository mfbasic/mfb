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

## `Implementation::Resolve` (rename `Custom` → self-describing) for computed dispatch

**Status:** idea only — do NOT start without a `plan-NN`.

**Motivation.** `Implementation::Custom` is a meaningless name (same disease as the
now-`#[deprecated]` `Same`): it tells you nothing about *what* code runs or *where*.
All it means is "argument-dependent — go ask this package's `BuiltinResolver`",
which hides both the selection logic and the candidate implementations across a
`dyn BuiltinResolver` impl plus a scatter of sibling/source functions. The only
genuine remaining home for `Custom` is **computed / open-set dispatch** the monomorph
overload machinery cannot express — the archetype is `vector`:

- `vector_package.mfb` hand-authors ~173 `FUNC __vector_<member>_<suffix>`
  implementations, one per `(member, element, dimension)` shape.
- `VectorResolver::implementation_name` picks one by **computing** the target name
  with `format!("__vector_{member}_{suffix}")` from `vector_shape(type)`
  (`src/builtins/vector.rs`). The options are invisible at the call/descriptor —
  you must grep the resolver *and* the `.mfb` companion to see what a call resolves
  to (contrast `::os`, which lists `posix`/`win` inline).

**Rough shape (to be designed, not committed here):**

- Add `Implementation::Resolve { resolver, variants }` — the honest, self-describing
  form of a *closed-set* `Custom`:
  - `resolver: fn(&[String] /*arg types*/, Option<&str> /*expected*/) ->
    Result<Option<&'static str /*key*/>, ()>` — the computed selection, pure over
    the call's static types (`Err(())` preserves the `TYPE_OVERLOAD_AMBIGUOUS`
    path). For `vector` the resolver body is the existing `format!`/`vector_shape`
    logic, returning the map key.
  - `variants: &'static [Variant]` — the candidate implementations **listed inline
    on the descriptor**, keyed by name; each `Variant` carries `{ name, return_type,
    implementation: Implementation }` so the map literally holds the `::mfb`/
    `::native`/`::rewrite` for each shape. Large regular families (vector's 173) are
    populated by a `const fn`/macro over a shared `SHAPES` list so the map and the
    resolver's key rule come from ONE source (no drift), not 173 hand-typed rows.
- Once `::resolve` covers the computed cases, mark **`BuiltinFunction::custom` /
  `Implementation::Custom` `#[deprecated]`** (as `Same` already is) with a note
  steering to `::resolve` (computed/resolver dispatch) or native overloads (the
  `encoding::utf8Encode`/`utf8Decode` pattern — same-named source implementations
  the monomorphizer mangles; see commit `8696556a0`).

**Open question the plan MUST answer first (do NOT assume `::resolve` is needed):**
whether `vector` even needs a resolver, now that builtin sources are injected
*before* monomorphization (commit `8696556a0`) and a builtin can be a real native
overload set. `vector::length(v)` selecting `__vector_length_<shape>` by the arg's
shape type is a **parameter overload** — declaring the implementations as same-named
`FUNC __vector_length(v AS Float2) …` / `(v AS Float3) …` / … would let the
monomorphizer mangle them to private `$`-symbols with NO resolver and NO `Custom` at
all, exactly like `utf8Decode`. `::resolve` only earns its place if some `vector`
dispatch genuinely cannot be expressed as an overload set (e.g. selection on a
*computed* axis that is neither arg-type nor return-type). Evaluate that per member
before adding the machinery — the earlier `Implementation::Resolve` attempt
(reverted in `4d379173`) was worth reverting precisely because it was aimed at
`encoding`, which was plain overloading, not a computed family.

**Guardrails when it is planned:** behavior-preserving per member — the resolved
concrete target (hence emitted `.ncode`) must not change, so byte-identity gate
every affected package (`vector`, and anything injecting it). No new public names:
the variant targets stay internal `__vector_*` symbols, never descriptor members
(the whole point — `Custom`'s computed targets were already internal; keep them so).
