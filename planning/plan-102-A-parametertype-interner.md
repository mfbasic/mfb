# plan-102-A: ParameterType interner + type-model completeness

Last updated: 2026-08-23
Overall Effort: huge (>3d) — the whole plan-102 feature
Effort: large (3h–1d)
Depends on: nothing

Replace `ParameterType`'s `Box::leak` interning with a real `Copy` symbol
interner, and complete the structural type vocabulary (`MapEntry OF`, `Result OF`)
so a single `ParameterType` can represent every type the middle-end manipulates.
This is the foundation for holding `ParameterType` at every HIR/IR node cheaply —
without it, typing those nodes would leak one `&'static str` per distinct type
spelling in the whole program.

This sub-plan is also the **lead document** for the plan-102 feature: it carries
the shared goal, prerequisites, measured populations, design overview, and the
A→F roadmap. Sub-plans B–F are complete plans in their own right but do not
restate the shared design — read this first.

References:

- `src/types.rs` — `ParameterType`, `parse`, `name`, the `Box::leak` interner.
- `src/codegen/registry/mod.rs` — the registry, the sole current `ParameterType`
  consumer; re-exports `ParameterType`.
- `src/monomorph/helpers.rs` — `unify_type`/`substitute_type_params`, which
  model `MapEntry OF`/`Result OF` (`:67`, `:86`) that `parse` does not.
- `.ai/testing-gates.md` — artifact-gate, byte-identity, acceptance harness.
- `.ai/codegen-invariants.md` — arch-neutral codegen/IR invariants.

## Prerequisites

Shared by every plan-102 sub-plan; stated once here.

| Must be true | Command | Status |
|---|---|---|
| On a feature branch, not main | `git rev-parse --abbrev-ref HEAD` (≠ `main`) | MET (worktree-P-102) |
| Baseline gate captured (the branch's pre-existing `.ncode` diff set) | `scripts/artifact-gate.sh target/release/mfb all` → record the DIFF set | MET — 13 diffs recorded in `planning/plan-102-baseline-diffs.txt` (2026-08-23) |
| Full suite green at HEAD | `rustup run 1.96.0 cargo test` | MET — only failure is the recorded `artifact_gate_all` baseline (13 diffs); all other binaries green (`--no-fail-fast`) |

> The Command column is the truth; re-run and update Status before continuing and
> before deciding to stop. This whole feature is behavior-preserving, so the
> baseline `.ncode`/`.ncodesum` diff set (the branch's pre-existing noise) MUST be
> captured before phase 1 — every phase's acceptance is "no NEW diff vs this
> baseline", and you cannot judge "new" without it.

Everything below is written against the world where these hold.

## 1. Goal

- `ParameterType::Named`/`Var` hold a `Copy` interned symbol (`Symbol(u32)` /
  `TypeId`) instead of a `Box::leak`ed `&'static str`; `ParameterType` is `Copy`
  in its leaves and cheap to clone; **no `Box::leak` remains in `src/types.rs`.**
- `ParameterType` can structurally represent `MapEntry OF K TO V` and
  `Result OF T` (the two shapes `monomorph` handles that `parse` today folds into
  `Named`), so a later pass can put `ParameterType` at every node with no
  representational gap.
- `parse` and `name` round-trip byte-for-byte identically to today for every
  input (the registry and all current consumers see no behavior change).

### Non-goals (explicit constraints)

- **No change to compiled output.** Every fixture compiles to byte-identical
  `.ncode`/`.ncodesum` across all targets (this is a pure representation swap).
- No change to source-language semantics, diagnostics wording, or the `.mfp`/IR
  wire format (`ir/binary.rs`, `ir/json.rs` keep emitting type *strings* via
  `name()`).
- No new public/CLI surface.
- Do **not** move any type field off `String` yet outside `src/types.rs` — the
  AST, monomorph, and IR stay exactly as they are; this sub-plan only changes how
  `ParameterType` interns and what shapes it can hold.

## 2. Current State

`ParameterType` (`src/types.rs:22`) is a structural enum already used as the
registry's internal type currency. The string↔enum boundary is `parse`
(`src/types.rs:146`, the only string→enum site) and `name`
(`src/types.rs:221`, enum→`Cow<'static,str>`). Nominal/variable leaves are
interned by leaking: `parse`'s fallback arm is
`ParameterType::Named(Box::leak(other.to_string().into_boxed_str()))`
(`src/types.rs:216`). `Var` is **never produced by `parse`** — it is only written
by hand in registry descriptors (verified: read `src/types.rs:146-218`, no `Var`
arm; the fallback is always `Named`).

Today the leak is bounded and acceptable because `ParameterType` only lives at the
low-frequency registry boundary. It becomes unacceptable the moment `ParameterType`
is stored per HIR/IR node (plan-102 B–E), where it would leak every distinct type
spelling in the program.

### Measured populations

| What | Count | Command |
|---|---|---|
| `ParameterType::parse` call sites | 33 | `rg -c 'ParameterType::parse' src/ \| awk -F: '{s+=$2} END{print s}'` |
| `Box::leak` in `src/types.rs` | 1 | `rg -c 'Box::leak' src/types.rs` |
| `ParameterType::Named` / `Var` construction sites | 422 | `rg -n 'ParameterType::Named\|ParameterType::Var' src/ \| wc -l` |
| `.name()` calls in `types.rs` + `registry/mod.rs` | 42 | `rg -c '\.name\(\)' src/types.rs src/codegen/registry/mod.rs \| awk -F: '{s+=$2} END{print s}'` |
| `src/types.rs` size | 702 | `wc -l src/types.rs` |

### Verified properties

- **`parse` cannot produce `Var`.** Read `src/types.rs:146-218`: every non-scalar,
  non-container identifier falls to the `Named(Box::leak(...))` arm (`:216`). VERIFIED.
- **`parse` has no `MapEntry OF` / `Result OF` arm.** Read `src/types.rs:146-218`:
  neither prefix is matched; both become `Named`. `monomorph::helpers` *does* model
  them (`unify_type` `src/monomorph/helpers.rs:67,86`). So the shapes exist in the
  middle-end but not in `ParameterType`. VERIFIED.
- **`ParameterType` derives `Clone, Debug, PartialEq, Eq`** (`src/types.rs:21`) —
  a `Copy` `Symbol` leaf keeps `Eq`/`PartialEq` as integer comparison and makes
  clone cheap. VERIFIED (read the derive).

## 3. Design Overview

This is the whole plan-102 layering. The target pipeline:

```
source ─parse────→ AST   (tree, type fields are String — UNCHANGED, stays string)
       ─elaborate→ HIR   (tree, name-resolved, typed with ParameterType, GENERIC)
       ─monomorph→ IR    (typed with ParameterType, CONCRETE — generics erased)
       ─codegen──→ machine code
```

- **AST** stays string-based (surface syntax; the parser cannot classify `Var`
  without scope — see plan-102-C). It is never retyped.
- **HIR** is the new typed, name-resolved, still-generic layer. `elaborate`
  (AST→HIR) is where *all* semantic analysis lives: name resolution, type
  inference/checking, overload resolution, and the string→`ParameterType` typing.
- **monomorph** becomes the HIR→IR boundary that erases generics, operating on
  `ParameterType` (structural, interned) instead of string surgery.
- **IR** is the existing concrete representation, retyped to `ParameterType`.

**The whole feature is provably-neutral: the compiled output must not change.**
It is an internal representation rearchitecture, so **byte-identity
(`artifact-gate all` + `test-accept`) is the correct, primary gate for every
phase**, alongside `cargo test`. A NEW `.ncode` diff (beyond the captured
baseline) is a bug to root-cause (objdump one fixture) and fix — never a signal
the design is dead.

### The A→F roadmap (letter order = implementation order)

| Sub-plan | Delivers | Effort | Depends on |
|---|---|---|---|
| **A** (this) | `Copy` symbol interner; `MapEntry`/`Result` variants; leak gone | large | — |
| **B** | Typed IR: `ir::lower` converts String→`ParameterType` at the AST→IR boundary; IR fields + consumers retyped; wire seams keep `name()` | x-large | A |
| **C** | HIR data model + `elaborate(concrete AST → HIR)`; `ir::lower` switched to HIR→IR | x-large | B |
| **D** | Lift `elaborate` above monomorph (generic AST → generic HIR); relocate overload resolution into elaboration (temporary HIR→AST bridge feeds the still-string monomorph) | x-large | C |
| **E** | Move monomorph onto HIR (typed `unify`/`substitute`, HIR→HIR); remove the bridge; retire string monomorph | large | D |
| **F** | Retire redundant string-based front-end type work; consolidate resolver/syntaxcheck type checks onto HIR | medium | E |

B, C, D are each x-large and **must be re-measured and split into parts at their
own kickoff** (the write-plan split rule); their exact internal shape depends on
the HIR node design fixed in C. This roadmap fixes the order and the seams, not
the intra-sub-plan phase counts.

**Where the risk concentrates:** design uncertainty is highest in **C/D**
(can `elaborate` own name-resolution + `Var` classification + overload resolution
as one pass, and is the HIR node shape right?) — that is why C stands up the HIR
machinery on *concrete* (post-monomorph, no-`Var`) code first, the cheapest place
to falsify the HIR design. Correctness blast radius is highest in **B** (676
`.type_` sites, all of codegen) and **E** (monomorph rewrite) — both gated by
byte-identity.

### Rejected alternatives

- **Type the AST directly (put `ParameterType` on AST nodes).** Rejected: the
  parser cannot classify `Var` vs `Named` without scope, and the AST must hold
  invalid/unresolved source spellings verbatim for diagnostics. (Decided across
  this feature's design discussion; the AST stays string-based.)
- **Give monomorph an internal typed representation but leave it on the string
  AST.** Rejected: a half-measure that still re-parses strings at monomorph's
  boundary and does not give the type checker or IR a typed representation. The
  HIR unifies all of it.
- **Skip the interner, keep `Box::leak`.** Rejected: leaks one string per distinct
  type spelling per compile once `ParameterType` is per-node — an unbounded leak
  in a long-running `mfb` process / test suite. Correctness-over-performance:
  a leak for convenience is not acceptable.

## 4. Interner design

A process-wide (or compilation-scoped) string interner returning a `Copy`
`Symbol(u32)`:

- A `Symbol` interns a `&str` to a small integer; equal strings → equal `Symbol`;
  `Symbol` resolves back to `&'static str` (or a scoped `&str`) for `name()`.
- `ParameterType::Named(Symbol)` and `ParameterType::Var(Symbol)` replace the
  `&'static str` leaves. `Eq`/`Ord`/`Hash` on `Symbol` are integer ops.
- `parse`'s fallback arm interns instead of leaking:
  `ParameterType::Named(Symbol::intern(other))`.
- `name()` for `Named`/`Var` resolves the `Symbol` to its text; scalars/containers
  render exactly as today (byte-identical).

Interner storage: a `OnceLock<Mutex<StringInterner>>` (or the existing pattern the
codebase uses for process-global tables). The registry already builds its tables
behind `OnceLock`; mirror that. The interner is append-only, so a `Symbol` is
stable for the process lifetime and resolves without locking on the read path if
backed by a boxed-slice arena. **This replaces a leak with a bounded, deduplicated
table — the same set of distinct strings, interned once instead of leaked once.**

Precedent to mirror: `binary_repr` already interns type names into a `type_id`
table (`src/binary_repr/builder.rs`), so an integer-ID type-name model is idiomatic
here.

## 5. MapEntry / Result variants

Add to `ParameterType`:

- `MapEntryOf(Box<ParameterType>, Box<ParameterType>)` — key/value, rendering
  `MapEntry OF K TO V`.
- `ResultOf(Box<ParameterType>)` — success type, rendering `Result OF T`.

Wire them into `parse` (two new prefix arms mirroring `Map OF`/`List OF`), `name`
(two new render arms), and the registry's structural `unify`/`substitute`/
`leaf_matches` recursion (they must recurse into the new children exactly as
`ListOf`/`MapOf` do). Because nothing currently *constructs* these variants (today
they are `Named`), adding them is byte-identical **only if** `parse` now produces
them where it previously produced `Named` — verify that no registry descriptor or
matcher relied on the old `Named("MapEntry OF …")` spelling (grep the registry for
literal `"MapEntry OF"` / `"Result OF"` matches before landing).

## Compatibility / Format Impact

Nothing externally observable changes. `name()` output is byte-identical, so the
`.mfp`/IR wire format (which serializes type strings) is unchanged. The interner
is process-internal.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — capture the baseline gate

One line: record the branch's pre-existing `.ncode`/`.ncodesum` diff set and a
green suite, so every later phase can prove "no NEW diff".

- [x] Run `rustup run 1.96.0 cargo build --release --bin mfb`. (release built, 1m14s)
- [x] Run `scripts/artifact-gate.sh target/release/mfb all`; save the sorted DIFF
      set to `planning/plan-102-baseline-diffs.txt` (git-ignored or noted as
      scratch). This is the "pre-existing" set. (13 DIFF lines: 5 win64 `*_codegen_cover_rt`,
      4 crypto-ec cross-arch, 4 macos-app-mode app.ncode — recorded.)
- [x] Run `rustup run 1.96.0 cargo test`; confirm the only failure (if any) is the
      recorded `artifact_gate_all` pre-existing set. (`--no-fail-fast`: sole failure
      is `artifact_gate_all`; all other binaries green, 3612 lib + integration all ok.)

Acceptance: `planning/plan-102-baseline-diffs.txt` exists and `cargo test` is
green except for the recorded pre-existing gate diffs. VERIFIED.
Commit: 19dd9ef86

### Phase 2 — add the `Copy` symbol interner (no `ParameterType` change yet)

One line: introduce the interner as a standalone primitive with unit tests, before
wiring it into `ParameterType` — a primitive with no callers is safe to land alone.

- [x] Add the interner (new module, e.g. `src/types.rs` submodule or
      `src/intern.rs`): `Symbol(u32)`, `intern(&str) -> Symbol`,
      `Symbol::resolve(self) -> &'static str`. Append-only, `OnceLock`-backed.
      (Added `src/intern.rs`: `Symbol(NonZeroU32)` over `OnceLock<Mutex<Interner>>`,
      dedup map + index table, `resolve` returns `&'static str`; registered in
      `main.rs`.)
- [x] Tests: interning the same string twice yields equal `Symbol`; distinct
      strings yield distinct `Symbol`; `resolve(intern(s)) == s` for a corpus
      including composite names. (5 unit tests, all pass.)

Acceptance: interner unit tests pass; `cargo test` green (no other code changed).
VERIFIED (5/5 intern:: tests pass). NOTE: standalone primitive emits transient
dead_code warnings until Phase 3 wires it into `ParameterType` (next commit).
Commit: ba11589c2

### Phase 3 — switch `Named`/`Var` to `Symbol`; delete the leak

One line: flip the two leaves to `Symbol` and repoint the 422 construction sites +
`parse`/`name`.

- [x] Change `ParameterType::Named`/`Var` to hold `Symbol` (`src/types.rs`).
- [x] `parse` fallback interns instead of `Box::leak` (now `ParameterType::named(other)`).
- [x] `name()` resolves the `Symbol` for `Named`/`Var` (`elem.resolve()`/`name.resolve()`).
- [x] Repoint the ~422 `ParameterType::Named(...)`/`Var(...)` construction sites to
      pass a `Symbol` (most are registry descriptors passing a `&'static str`
      literal → wrap in `Symbol::intern` or a `const`-friendly helper). Measure the
      residual after an initial sweep: `rg -n 'Named\("|Var\("' src/`. (Added
      `ParameterType::named`/`var` interning constructors; sed-repointed 416
      `ParameterType::Named(`/`Var(` call sites + 25 bare test-module calls to the
      helpers; `bindings` map in registry `unify`/`substitute` re-keyed
      `&'static str` → `Symbol`. Residual bare `Named("`/`Var("` constructions: 0.)
- [x] Confirm `rg -c 'Box::leak' src/types.rs` → 0. (VERIFIED: 0.)

Acceptance: `cargo test` green; `artifact-gate all` shows **no NEW diff** vs the
Phase-1 baseline; `rg 'Box::leak' src/types.rs` is empty. VERIFIED — full suite's
sole failure is the recorded `artifact_gate_all` baseline; `diff` of gate output
vs `plan-102-baseline-diffs.txt` is IDENTICAL; leak count 0.
Commit: 9d1af3130

### Phase 4 — add `MapEntryOf` / `ResultOf` variants

One line: complete the type vocabulary so a later pass has no representational gap.

- [x] Add the two variants (`src/types.rs`) + `parse` prefix arms + `name` render
      arms + registry `unify`/`substitute`/`leaf_matches` recursion. (Added
      `MapEntryOf`/`ResultOf` + `map_entry_of`/`result_of` constructors; parse arms
      mirror `Map OF`/`List OF`; unify/substitute/contains_var + the container
      fail-set updated. `leaf_matches` needs no change — both are containers handled
      before the leaf catch-all.)
- [x] Grep the registry for any code that matched the old `Named("MapEntry OF …")`
      / `Named("Result OF …")` spelling and update it to the new variant
      (`rg -n '"MapEntry OF|"Result OF' src/`). (0 matches in registry/types — nothing
      relied on the old spelling; byte-identity confirms it below.)
- [x] Tests: `parse("MapEntry OF String TO Integer").name()` round-trips;
      `parse("Result OF Nothing").name()` round-trips; a unify/substitute test over
      each new variant. (`map_entry_and_result_parse_into_variants_and_round_trip` in
      types.rs; `unify_substitute_over_map_entry_and_result_variants` +
      `contains_var` extended in registry — all pass.)

Acceptance: round-trip + unify/substitute tests pass; `artifact-gate all` shows no
NEW diff vs baseline; `cargo test` green. VERIFIED — new unit tests pass; gate
`diff` vs baseline IDENTICAL; full suite's sole failure is the `artifact_gate_all`
baseline.
Commit: f67c31783

## Validation Plan

- Tests: interner unit tests (Phase 2); `parse`/`name` round-trip tests for the
  new variants (Phase 4); the existing `src/types.rs` and registry tests must stay
  green.
- Coverage check: the interner and new variants are exercised by the added unit
  tests and by the whole existing registry suite (which round-trips types).
- Runtime proof: `artifact-gate all` byte-identical (modulo baseline) proves real
  programs still compile to the same machine code.
- Doc sync: none expected (internal representation; `name()` output unchanged). If
  `.ai/codegen-invariants.md` describes the `Box::leak` interning, update it.
- Acceptance: `rustup run 1.96.0 cargo test`; `scripts/artifact-gate.sh
  target/release/mfb all` (no NEW diff); `scripts/test-accept.sh
  target/release/mfb /tmp/accept-out` (no NEW mismatch); `rustup run 1.96.0 cargo
  fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Interner scope: process-global vs compilation-scoped.** Recommend
  process-global `OnceLock` (matches the registry's existing pattern, simplest,
  and `mfb` compiles one project per process). A compilation-scoped interner
  would be needed only if a single process compiles many independent projects
  (not the case today). (§4)
- **`Symbol` width: `u32` vs `NonZeroU32`.** Recommend `NonZeroU32` so
  `Option<Symbol>` is niche-packed to 4 bytes. (§4)

## Corrections

<Filled in during execution.>

## Summary

The engineering risk here is low and contained: it is a mechanical representation
swap in one file plus its 422 construction sites, gated by byte-identity. Its
value is entirely as the foundation — it is the one hard prerequisite that makes
holding `ParameterType` at every HIR/IR node (B–E) affordable. Nothing above
`src/types.rs` changes shape in this sub-plan.
