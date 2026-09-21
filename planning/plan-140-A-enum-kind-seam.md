# plan-140-A: An enum-kind seam for built-in overload resolution

Last updated: 2026-09-20
Overall Effort: large (3h–1d) — the whole plan-140 feature: `toInt(ENUM)`
(plan-140-B) and `toString(ENUM)` (plan-140-C)
Effort: medium (1h–2h)
Depends on: nothing

plan-140 makes `toInt(e)` legal for any enum value `e`, returning the
member's 0-based declaration index as an `Integer` (plan-140-B). It also makes
`toString(e)` legal, returning the member's name (plan-140-C). This sub-plan
builds the seam both need, and changes no program's behavior.

The built-in overload resolver cannot tell an enum from any other declared
type. `general::resolve_call` (`src/codegen/builtins/general/mod.rs:333`)
decides from `&[ParameterType]` alone, and every declared type (record, union,
enum) is the same opaque `ParameterType::Named`/`declared(name)`. There is no
enum variant (`src/types.rs:155`, `enum ParameterType`). The modules that call
the resolver each hold their own view of the type declarations: `ir::shape`
has `TypeShape.is_enum` (`src/ir/shape.rs:439`), codegen has
`TypeModel.enum_members` (`src/codegen/engine/builder/mod.rs:996`), and the
package verifier has `type_decl_info` (`src/ir/verify/values.rs:696`).

The single behavioral outcome of **this sub-plan**: every built-in resolution
site that decides whether a call is *accepted* or *dispatched* passes the
resolver an oracle answering "is this type an enum?". Because no built-in
consults the oracle yet (plan-140-B turns it on), every program compiles to
byte-identical output.

References:

- `src/codegen/builtins/mod.rs:465` — `resolve_call_return_type_typed`, the
  typed entry every site calls.
- `src/codegen/builtins/general/mod.rs:333` — `resolve_call`; `:397` — the
  `TO_INT` arm.
- `.ai/compiler.md` — the Hard Completion Gate and the fixture rules.
- `src/docs/spec/architecture/12_monomorphization.md` §"Built-in-named
  overrides" — the gap-fill rule that makes monomorph's resolution a
  *dispatch* decision, not just typing.
- `.ai/codegen-invariants.md` — before touching any codegen file.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| The tree builds | `cargo build --release` → exit 0 | MET (2026-09-20: `target/release/mfb` built 13:42 and used all session) |
| The golden gate is green before the change, so a diff afterwards is this plan's | `bash scripts/artifact-gate.sh target/release/mfb all` → no diffs | NOT MEASURED — run before Phase 1 |

Everything below is written against a tree where both hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- A `TypeKinds` oracle (`fn is_enum(&self, t: &ParameterType) -> bool`) reaches
  the built-in resolver from every site that decides acceptance or dispatch, and
  `bash scripts/artifact-gate.sh target/release/mfb all` shows **no diffs**.

### Non-goals (explicit constraints)

- No built-in accepts anything it did not accept before. `general::resolve_call`
  may receive the oracle but must not consult it in this sub-plan.
- No change to `ParameterType` (no enum variant). That type is a
  registry/type-checker key with `Hash`/`Eq` pinned by
  `parameter_type_hash_agrees_with_eq`. Adding a kind to it would change
  equality for every declared type.
- No change to diagnostics text, IR, package metadata or the ABI.
- The typing-only sites (see §2) are not wired. Their answer for `toInt` is
  settled in plan-140-B by a name-keyed fallback, not by this oracle.

## 2. Current State

`resolve_call_return_type_typed(callee, arg_types, strict)`
(`src/codegen/builtins/mod.rs:465`) dispatches to per-package resolvers. For
bare general built-ins it reaches `general::resolve_call(name, arg_types)`,
whose `TO_INT` arm (`general/mod.rs:397`) accepts exactly `String`, `Byte`,
`Float`, `Fixed`, `Money` or `Scalar`, or `(String, Integer)`. Anything else,
including every declared type, returns `None`.

What a site does with `None` depends on what it uses the answer for. There are
three kinds of site:

1. **Acceptance** (strict): a `None` is a diagnostic. `ir::shape`
   (`shape.rs:2531`, `:2588`, `:2651` …) reports `TYPE_CALL_ARGUMENT_MISMATCH`.
   `ir::verify::compat` (`compat.rs:65`, `:88`) does the same for decoded
   package IR.
2. **Dispatch** (lenient, but decides behavior): `monomorph::lower`
   `resolve_general_builtin_override` (`monomorph/lower.rs:984`) selects a
   **user** override only when the built-in rejects the argument types (the
   gap-fill rule). The existing fixture
   `tests/rt-behavior/functions/func_override_toint_user` depends on this: it
   declares `FUNC toInt(m AS Cash)` over a *record*, which must still dispatch
   to the override after plan-140-B.
3. **Typing** (lenient): `ir::lower::expression_type` (`ir/lower.rs:3959` and
   four more), codegen's `static_type_name*` (`builder_value_semantics.rs:1211`,
   `:1401`), `type_utils.rs:60` and `data_objects.rs:1532`. These only infer a
   result type. `type_utils.rs:60` already falls back to the name-keyed
   `builtins::call_return_type(target)`; `ir/lower.rs:3959` falls back only for
   a package override, else yields no type.

### Measured populations

| What | Count | Command |
|---|---|---|
| Call sites of `resolve_call_return_type_typed` outside tests and its definition | 25, across 9 files | `grep -rn "resolve_call_return_type_typed(" src --include='*.rs' \| grep -v "fn resolve_call_return_type_typed" \| grep -v "/tests" \| wc -l` → 25 |
| …per file | shape.rs 7, verify/compat.rs 8, ir/lower.rs 5, monomorph/lower.rs 3, builder_value_semantics.rs 2, data_objects.rs 2, type_utils.rs 1, regex/mod.rs 1, builtins/mod.rs 1 | same command, `awk -F: '{print $1}' \| sort \| uniq -c` |
| Files among those that already read enum-kind facts | shape.rs 4 refs, ir/lower.rs 4, builder_value_semantics.rs 2, monomorph/lower.rs 1, verify/compat.rs 0, type_utils.rs 0, data_objects.rs 0, regex/mod.rs 0 | `grep -cE 'is_enum\|enum_members\|TypeDeclKind::Enum\|ImportedTypeKind::Enum\|IrTypeKind::Enum\|kind == "enum"' <file>` |

### Verified properties

- **Enum values are their 0-based ordinal at runtime.** Read:
  `validation.rs:288` assigns `enum_members[(type, member)] = index` from
  `type_.members.iter().enumerate()`. `builder_values.rs:2836` lowers
  `Type.Member` to `move_immediate(IMMEDIATE_CLASS_ENUM_ORDINAL, ordinal)`.
  `builder_value_semantics.rs:1607` matches `CASE Type.Member` by
  `compare_immediate(ordinal)`. The spec lists enums as inline scalars
  (`src/docs/spec/memory/03_heap-values.md:54`).
- **`ir::shape` knows enum-ness per type:** `TypeShape.is_enum`, filled from
  `TypeDeclKind::Enum` (`shape.rs:741`) and `ImportedTypeKind::Enum`
  (`shape.rs:782`). Read.
- **Codegen knows enum-ness:** `TypeModel.enum_members` is keyed by
  `(enum type, member)`. A type is an enum iff some key has it as its first half.
  Read (`builder/mod.rs:996`). There is no direct `is_enum` helper yet.
- **UNVERIFIED:** that `ir::verify`'s `type_decl_info` carries the declaration
  *kind* (read so far only for its owner file, `values.rs:696`). Task in Phase 1.
- **UNVERIFIED:** which enum source `monomorph::lower` has at `:984` (1 enum
  reference in the file, not yet read). Task in Phase 1.

## 3. Design Overview

One new trait in `src/codegen/builtins/mod.rs`:

```rust
pub(crate) trait TypeKinds {
    fn is_enum(&self, t: &ParameterType) -> bool;
}
pub(crate) struct NoTypeKinds;          // is_enum → false
```

and one new entry point:

```rust
pub(crate) fn resolve_call_return_type_with_kinds(
    callee: &str, arg_types: &[ParameterType], strict: bool, kinds: &dyn TypeKinds,
) -> Option<ParameterType>
```

`resolve_call_return_type_typed` becomes
`resolve_call_return_type_with_kinds(.., &NoTypeKinds)`, so every unwired site
keeps its exact behavior. `general::resolve_call` gains a `kinds` parameter that
it ignores in this sub-plan.

The **acceptance** and **dispatch** sites switch to the `_with_kinds` entry,
passing an adapter over the declarations they already hold:

| Site kind | Files | Oracle source |
|---|---|---|
| Acceptance, source | `ir/shape.rs` | `self.types[t].is_enum` |
| Acceptance, package | `ir/verify/compat.rs` | `type_decl_info` kind (Phase 1 verifies it carries one) |
| Dispatch | `monomorph/lower.rs:984` | monomorph's type declarations (Phase 1 locates them) |
| Lowering | codegen `CodeBuilder` | `TypeModel` (a new `is_enum_type` helper over `enum_members`) |

**The typing sites are left on `NoTypeKinds`.** For `toInt` the answer is
always `Integer`, whatever the argument. plan-140-B gives them that answer
through the name-keyed fallback (`general::nominal_return_type(TO_INT) =
Integer`, `general/mod.rs:310`), not through the oracle. That avoids threading
type declarations into five typing paths that don't need them.

**Correctness gate: byte-identity.** This sub-plan is provably neutral: the
oracle is plumbed but no resolver arm reads it. So
`scripts/artifact-gate.sh … all` with **zero diffs** is the acceptance check,
on every target the gate covers. A diff means an adapter changed behavior (for
example, a site switched to the wrong strict flag). Root-cause it from one
fixture's dump and fix it. It is never evidence against the design.

**Where the risk concentrates:** the dispatch site. If monomorph ever reports a
*record* as an enum, `func_override_toint_user` would silently lose its
override in plan-140-B. The adapter must answer from declaration kind, never
from "is a declared type".

Rejected alternatives:

- **Add `ParameterType::Enum(name)`.** It is the most direct option, but it
  changes equality and hashing for a type that keys `TypeModel`'s tables and
  the registry (plan-111-C). Every producer of a declared type would have to
  know its kind, and package metadata encodes types structurally. Blast radius
  far beyond one built-in.
- **Thread-local "current enum names" set.** Hidden global state across the
  compiler's pipeline passes and per-file contexts; wrong the moment two
  packages declare an enum with the same bare name.
- **Special-case `toInt` inline at each site without a seam.** Four copies of
  the same kind test with no shared contract. This is the two-lists-drift shape
  `inline_builtin_fallibility_depends_on_args`'s comment warns against
  (`builtins/mod.rs:313`).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` for partial, with what remains. Moot tasks are
> struck through with evidence, never deleted. Fill `Commit:` the moment a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — Locate the oracle sources

Read-only; settles the two UNVERIFIED rows so Phase 2's tasks name real fields.

- [ ] Read `src/ir/verify/` for `type_decl_info`'s value type; record in §2
      whether it carries the declaration kind, and if not, which verifier field
      does (e.g. the IR type table's `kind`).
- [ ] Read `src/monomorph/lower.rs` around `resolve_general_builtin_override`
      (`:984`) and its one enum reference; record which structure answers "is
      type T an enum" there.
- [ ] Update §3's oracle-source table with both answers (file:symbol).

Acceptance: §2 has no UNVERIFIED row and §3's table names a concrete symbol per
site.
  Check: `grep -c UNVERIFIED planning/plan-140-A-enum-kind-seam.md` → 0 (est. 1 min).
Commit: —

### Phase 2 — The seam, wired, behavior-neutral

- [ ] `src/codegen/builtins/mod.rs`: add `TypeKinds`, `NoTypeKinds`, and
      `resolve_call_return_type_with_kinds`. Make
      `resolve_call_return_type_typed` delegate with `&NoTypeKinds`.
- [ ] `src/codegen/builtins/general/mod.rs`: `resolve_call` takes
      `_kinds: &dyn TypeKinds`, unread until plan-140-B's `TO_INT` arm reads it.
      Update its unit tests' helper `rt(...)` to pass `&NoTypeKinds`.
- [ ] `src/codegen/engine/builder/mod.rs`: `TypeModel::is_enum_type(&self, t)`
      → `self.enum_members.keys().any(|(ty, _)| ty == t)`. If Phase 1 or a
      profile shows this is hot, add an `enum_types: HashSet<ParameterType>`
      filled beside `enum_members` in `validation.rs:288`.
- [ ] `src/ir/shape.rs`: an adapter over `self.types` (`is_enum`), used by all
      7 resolver calls.
- [ ] `src/ir/verify/compat.rs`: an adapter over the source Phase 1 found, used
      by its strict calls (`:65`, `:88`) and by `:209` if Phase 1 shows it
      decides acceptance.
- [ ] `src/monomorph/lower.rs:984`: an adapter over the source Phase 1 found.
- [ ] Tests: a unit test in `src/codegen/builtins/general/mod.rs`'s test module
      proving `resolve_call` returns the same answer for every existing
      `rt(TO_INT, …)` and `rt(TO_STRING, …)` case with an always-true and an
      always-false oracle. That pins the "not consulted yet" property.

Acceptance: every program compiles to byte-identical output.
  Check: `cargo build --release && bash scripts/artifact-gate.sh target/release/mfb all`
  → no diffs (est. 10–20 min; this is a code-motion change across 9 files and
  the resolver feeds every built-in call, so a whole-corpus byte-identity run is
  the smallest check that proves no site changed behavior).
  Check: `cargo test --bin mfb codegen::builtins::general` → pass (est. 2 min).
Commit: —

## Validation Plan

- Tests: the Phase 2 unit test (oracle not consulted).
- Coverage check: every one of the 25 resolver call sites either calls
  `_with_kinds` or is a typing site deliberately left on `NoTypeKinds`; list
  them in the commit message (`grep -rn "resolve_call_return_type_with_kinds(" src`).
- Runtime proof: none needed. The sub-plan changes no behavior, and the
  artifact gate is the proof.
- Doc sync: `src/docs/spec/architecture/12_monomorphization.md` §"Built-in-named
  overrides" gains one sentence: the gap-fill check is kind-aware (it receives a
  `TypeKinds` oracle). Cite `[[src/codegen/builtins/mod.rs:resolve_call_return_type_with_kinds]]`.
  Gate: `cargo test --bin mfb spec` and `scripts/spec-census.sh --citations`.
- Final gate (once, after Phase 2): `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  → all pass (est. per the harness; the whole acceptance suite is required by
  `.ai/compiler.md` after any compiler change).

## Open Decisions

- Trait object vs. closure for the oracle — **`&dyn TypeKinds`**
  (recommended): nameable in signatures, room for a second kind query later
  (`is_record` for a future `toString` rule) without another signature change.
  vs. `&dyn Fn(&ParameterType) -> bool`: fewer lines, but a second query means
  a second parameter.

## Corrections

## Summary

A plumbing change with no behavioral effect: the built-in resolver learns to
*ask* whether a type is an enum, and the four places that decide acceptance or
dispatch learn to *answer*. The risk is concentrated in the monomorph dispatch
adapter, whose wrong answer would only surface in plan-140-B. That is why the
Phase 2 unit test pins "not consulted" here, and plan-140-B and plan-140-C
each carry an override-precedence fixture.
