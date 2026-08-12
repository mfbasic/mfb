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

**Prerequisite (must land before the `Custom` deprecation): make the `overloads`
property carry the `Implementation`.** Today `BuiltinFunction.overloads:
&[BuiltinOverload]` is *signature/doc metadata only* — `{ params, return_type }`,
read by `DefaultResolver` for arity/expected-args/return type. The monomorphizer
never looks at it; a builtin's actual overload *implementations* live elsewhere
(same-named `__pkg_*` funcs in the package `.mfb`, per `encoding::utf8Encode`'s
native-overload pattern). Change `BuiltinOverload` to also hold each overload's
`implementation: Implementation` (`::mfb` body / `::native` lowering / `::rewrite`),
so an overloaded builtin is declared **once, wholly on the descriptor** — signature
*and* implementation per overload — instead of split between the descriptor
(signatures) and the `.mfb` companion (bodies). The overload machinery
(`monomorph::resolve_overload`) then resolves against the descriptor's `overloads`
directly, mangling each to a private `$`-symbol by signature.

This is the enabler that lets `Custom` be `#[deprecated]`: every `Custom` case that
is really overloading (`crypto` `_bytes`/`_text`, `datetime` by arity, and —
pending the open question above — likely `vector`) moves onto descriptor `overloads`
with real implementations, leaving `Custom` with no legitimate users.

**And if this works, `::resolve` is probably NOT needed at all.** `overloads`-with-
`Implementation` IS the "map of implementations" `::resolve` was going to add — keyed
by parameter/return signature and resolved *natively* by `resolve_overload`, with no
bespoke `resolver` fn. `::resolve` only survives if some builtin genuinely dispatches
on a *computed axis that is neither arg-type nor return-type* (none is known today —
`vector`'s axis is the argument's shape type, i.e. a parameter overload). So sequence
the plan: **(1)** `overloads` carries `Implementation`; **(2)** migrate the `Custom`
overload cases onto it; **(3)** `#[deprecate]` `Custom`; **(4)** add `::resolve`
*only if* a real computed-axis case remains after (2).

**Guardrails when it is planned:** behavior-preserving per member — the resolved
concrete target (hence emitted `.ncode`) must not change, so byte-identity gate
every affected package (`vector`, and anything injecting it). No new public names:
the variant/overload targets stay internal `__*` symbols, never descriptor members
(the whole point — `Custom`'s computed targets were already internal; keep them so).

## North star: one `implementations` array per builtin (unify overload+impl+lowering+errors+flags)

**Status:** direction only — the proper long-term shape the two entries above step
toward. Larger than either; do NOT start without a `plan-NN` (and land the
`overloads`-carries-`Implementation` prerequisite first).

**Motivation.** `BuiltinFunction` today spreads one function's definition across
several parallel, per-function fields that really describe *each version/overload*
of the function separately:

```rust
pub(crate) overloads: &'static [BuiltinOverload], // signatures (params + return)
pub(crate) implementation: Implementation,        // ONE impl for the whole name
pub(crate) lowering: Lowering,                     // ONE Helper/Inline for the name
pub(crate) errors: &'static [&'static str],        // ONE error set for the name
pub(crate) flags: BuiltinFlags,                    // ONE flag set for the name
```

A single `implementation`/`lowering` per name is exactly why overloading needed the
`Custom`/`Resolve` side-channels: the descriptor can only name *one* way to lower,
so a call that lowers N different ways (by arg/return type) has to escape to a
resolver. The honest model is: **a builtin function is a name + docs + one-or-more
fully-specified implementations**, each defining a signature *and* how that signature
is realized.

**Rough shape (to be designed, not committed here):**

- Collapse `overloads` + `implementation` + `lowering` (+ likely `errors`, and the
  overload-relevant parts of `flags`) into a single `implementations: &'static
  [Implementation]` (≥1). `Implementation` is re-done to carry, per version/overload:
  its **params + return type** (absorbing `BuiltinOverload`), its **lowering**
  (`Helper`/`Inline` — resolving the standing "is `Lowering` redundant" question by
  making it per-implementation), its **kind** (`Mfb`/`Native`/`Rewrite`/`Os`/…), and
  its **errors**. Then `resolve_overload` picks the right element by signature and
  mangles it — overloading, dispatch, and lowering all read from one place.
- Non-overload-varying facets stay function-level on `BuiltinFunction`: `name`,
  `doc_slug`/`doc_intro`/`doc_desc`/`doc_example`. The plan MUST decide the
  function-vs-implementation split for the ambiguous fields — `internal_only` reads
  function-level; `return_type_overloaded` is a property of the *set* (derivable
  from "≥2 implementations share params, differ by return"), not a stored flag;
  `errors` is usually uniform across overloads but is more precise per-implementation.
  Don't blindly push every field down — only the ones that genuinely vary per version.
- With this, `Custom` and `::resolve` both fall away for everything that is really
  overloading (the common case): the `implementations` array *is* the candidate set,
  resolved natively. Any true computed-axis remnant (if one is ever found) is the
  only thing that would still want a resolver.

**Why it's the endpoint, not the start:** it rewrites the construction site of all
~28 packages' function descriptors and the `DefaultResolver` derivations
(arity/expected-args/return type now fold over `implementations`), so it wants to
follow — not precede — the smaller `overloads`-carries-`Implementation` step and the
registry-constructor/encapsulation cleanup (first entry above). Sequence overall:
registry constructors → `overloads` carries `Implementation` → migrate `Custom`
overload cases → deprecate `Custom` → (only if needed) `::resolve` → unify into one
`implementations` array.

**Guardrails when it is planned:** provably-neutral refactor — same emitted `.ncode`
for every package (byte-identity gate per package); the descriptor is a pure
data-shape change, not a behavior change. One package (or small batch) per phase.
