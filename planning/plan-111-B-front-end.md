# plan-111-B: remove type strings from ir, monomorph and resolver

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-111-A (the ratchet gate exists so this letter's progress is
countable; `ParameterType: Hash`; `STATE` is a variant, so the typed accessors
this letter calls are structural rather than string-backed).

The front end is the easy half and it is still not clean. `ir::verify` holds a
typed `ParameterType`, renders it to a `String`, and hands it to a **codegen**
helper that parses it again — 46 removable `ParameterType::parse` sites, 23
`&str` type parameters and 19 spelling decisions across `src/ir`,
`src/monomorph` and `src/resolver`.

This letter deletes all of them. Nothing here is a design question: every task
is "change the signature to `&ParameterType`, delete the parse, match the
variant."

See plan-111-A for the shared prerequisites, the five sanctioned boundaries, the
byte-identity gate policy, and the rejected alternatives.

References:

- `src/ir/verify/matching.rs:29-33` — the canonical instance: `resource_base_type`
  returns a typed `ParameterType`, `.to_string()` renders it, then
  `crate::codegen::engine::types::is_result_type(&ty)` re-parses it, when
  `matches!(ty, ParameterType::ResultOf(_))` is the whole answer.
- `src/ir/verify/mod.rs:1189-1207` — `resource_base_type` (typed, correct) beside
  `resource_base_type_name` (its `&str` wrapper that parses and re-renders).
- `src/ir/shape.rs:290-317` — `resolve_table_call_with_byte_literals` threading
  `arg_types: &[String]` into the registry's string resolver and comparing
  `type_name.as_str() == "Integer"`.
- `src/monomorph/lower.rs:237` — instantiation-key matching comparing param and
  argument **spellings** with `*p == "Unknown"`.
- `src/types.rs` — `split_state`/`state`/`without_state`/`is_named`, the typed
  accessors that already exist and are simply not called.

## Prerequisites

See plan-111-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-111-A complete | `cargo test --test no_type_strings` passes; `rg -c 'Stateful' src/types.rs` → >0 | NOT MET until A lands |

## 1. Goal

- `rg 'ParameterType::parse\(' src/ir src/monomorph src/resolver` returns hits
  only in `src/ir/binary.rs` (the IR wire decoder, boundary #2).
- No function in `src/ir`, `src/monomorph` or `src/resolver` takes a type as
  `&str`.
- No `match` arm or `==` in those trees decides on a type spelling.
- **No front-end file calls a `&str`-taking `codegen::` type helper.** The
  render→re-parse round trips through `codegen::engine::types` and
  `codegen::resource` are gone.
- The gate budgets for `ir`, `monomorph` and `resolver` are at 0 for all six
  needle classes.

### Non-goals (explicit constraints)

- See plan-111-A §1 Non-goals — all apply. In particular: no rule semantics
  change, no diagnostic wording/ordering/location change, no wire-format change.
- **Do not touch `src/ir/binary.rs`.** It is boundary #2; its 27 parses stay.
- **Do not touch `src/codegen`.** The `&str`-taking codegen helpers this letter
  stops *calling* keep their `&str` signatures until letters D–F delete them.
  Moving those signatures here would braid this letter into D/E.
- Do not merge `ir::verify` and `ir::shape` logic, or move a rule between them.
  This letter changes representation only.

## 2. Current State

`ir`, `monomorph` and `hir` all carry `ParameterType` in their data structures —
plan-104/105/106 did that work. What survives is the *plumbing between* them:
helper functions still written against `&str`, and a set of shared predicates
that live in `src/codegen` and take a spelling, which the front end calls by
rendering its typed value first.

`src/ir/verify/mod.rs:1197-1207` states the pattern in its own doc comment: it
keeps `resource_base_type_name`, "the name-domain twin", explicitly *because*
some callers hold a `&str`. Those callers are this letter's work.

### Measured populations

At HEAD (`fd09ea809`), 2026-08-29, tests excluded
(`--glob '!**/tests*' --glob '!**/*_tests.rs' --glob '!src/testutil.rs'`),
`src/ir/binary.rs` excluded from the parse counts as boundary #2:

| File | parses | `&str` type params | spelling arms + `==` |
|---|---|---|---|
| `src/monomorph/lower.rs` | 18 | 3 | 2 |
| `src/monomorph/helpers.rs` | 8 | 4 | 0 |
| `src/ir/shape.rs` | 6 | 0 | 3 |
| `src/ir/lower.rs` | 4 | 1 | 3 |
| `src/ir/verify/compat.rs` | 3 | 4 | 0 |
| `src/ir/resource_escape.rs` | 3 | 0 | 0 |
| `src/ir/value.rs` | 2 | 0 | 0 |
| `src/ir/verify/mod.rs` | 1 | 6 | 1 |
| `src/resolver/resolution.rs` | 1 | 1 | 1 |
| `src/ir/verify/values.rs` | 0 | 1 | 6 |
| `src/ir/verify/matching.rs` | 0 | 0 | 2 |
| `src/ir/verify/link.rs` | 0 | 0 | 1 |
| `src/ir/link.rs` | 0 | 1 | 0 |
| `src/resolver/mod.rs` | 0 | 2 | 0 |
| **Total** | **46** | **23** | **19** |

Commands: the three `r '…' src/ir src/monomorph src/resolver` patterns from
plan-111-A §2, piped through `sed 's|:[0-9]*:.*||' | sort | uniq -c`.

Front-end → codegen string-helper call sites, the render→re-parse seams
(`rg -n 'codegen::(engine::types|resource)::' src/ir src/monomorph src/resolver src/cli --glob '!**/tests*'`):

| Site | Calls | Typed replacement that already exists |
|---|---|---|
| `src/ir/verify/matching.rs:31` | `is_result_type` | `matches!(ty, ParameterType::ResultOf(_))` |
| `src/ir/verify/link.rs:898` | `is_collection_type` | `codegen::engine::types::typed_is_collection_type` (`type_utils.rs:349`) |
| `src/ir/verify/link.rs:798` | `state_type_name` | `ParameterType::state` (`types.rs:526`) |
| `src/ir/verify/link.rs:926` | `builtin_resource_close_function` | takes a base name; feed `without_state()` structurally |
| `src/ir/verify/calls.rs:264,265` | `state_type_name` ×2 | `ParameterType::state` |
| `src/ir/verify/calls.rs:278` | `base_resource_name` | `ParameterType::without_state` (`types.rs:532`) |
| `src/ir/verify/mod.rs:1182` | `builtin_resource_close_function` | as above |
| `src/ir/verify/resources.rs:314` | `is_builtin_sendable_resource_type` | needs a typed twin — task in Phase 2 |
| `src/ir/shape.rs:2259,2260` | `base_resource_name`, `builtin_resource_close_function` | `ParameterType::without_state` |
| `src/monomorph/lower.rs:265` | `base_resource_name` | `ParameterType::without_state` |
| `src/cli/build/mod.rs:427` | `base_resource_name` | `ParameterType::without_state` |

### Verified properties

- **Every typed replacement in the table above already exists** except
  `is_builtin_sendable_resource_type`'s — read `src/types.rs:512-545`
  (`split_state`, `state`, `without_state`, `is_named`) and
  `src/codegen/engine/types/type_utils.rs:349` (`typed_is_collection_type`,
  `pub(crate)`, takes `&ParameterType`). This letter is mostly deleting
  round trips around helpers that are already there.
- **`resource_base_type` is already structural** — read
  `src/ir/verify/mod.rs:1189-1195`: `strip_res(type_).without_state()`, no
  rendering. Only its `_name` wrapper renders.
- **UNVERIFIED: whether `src/ir/shape.rs`'s `arg_types: &[String]` can become
  `&[ParameterType]` without touching the registry.** It feeds
  `builtins::resolve_call_return_type(callee, arg_types, true)`
  (`src/ir/shape.rs:295`), which is registry surface owned by letter C. Phase 3
  task 1 resolves this; if the typed registry twin does not exist yet, that
  conversion **moves to letter C** rather than being hand-rolled here.

## 3. Design Overview

Three phases, ordered by independence:

**Phase 1 — the seams (lowest risk, highest symbolic value).** Delete the
render→re-parse round trips: 12 named call sites, each a one-line swap to an
accessor that already exists. Independent of everything else and provably
neutral.

**Phase 2 — `ir::verify` and `ir::shape`.** The bulk of the `&str` parameters
(13 across `verify/mod.rs`, `verify/compat.rs`, `verify/values.rs`, `link.rs`)
and the spelling decisions (13). Delete `resource_base_type_name`
(`src/ir/verify/mod.rs:1200`) once its callers are typed — its existence is the
tell that they were not.

**Phase 3 — `monomorph` and `resolver`.** 26 parses and 4 `&str` parameters, and
the one genuinely subtle site: `monomorph/lower.rs:237` compares instantiation
keys as spellings. Monomorph's *keys* may legitimately stay strings — a mangled
instantiation name is a symbol, not a type — but the **types being compared to
build them** must not be. Phase 3 separates the two.

Correctness risk concentrates in Phase 3. Monomorph picks overloads by concrete
substituted argument types per instantiation (`.ai/codegen-invariants.md`; the
overload-resolution memory), and a `Var` bound to `Unknown` must stay a
*refinable provisional binding*, not be dropped or wildcarded — bug-442's
Option B. A conversion that turns `*p == "Unknown"` into a variant match must
preserve that refinement exactly, or width-agnostic native ops like
`collections::flatten$Unknown` hard-error.

Per plan-111-A §3, the per-phase gate is `cargo test --no-fail-fast -- --skip artifact_gate_all` —
the `--skip` keeps the full cross-target artifact sweep out of the loop, since
`tests/golden.rs`'s only test shells out to `artifact-gate.sh all`. Goldens,
`test-accept.sh` and the artifact gate are swept **once, in letter G**.
B is one of the two letters that also runs `diag-set-diff.sh`, because it is one
of the two that can move a source diagnostic. A diff surfacing in G is a bug to
root-cause with objdump on one fixture, never a reason to stop.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — delete the front-end → codegen render/re-parse seams

12 one-line swaps to accessors that already exist. No signature changes.

- [ ] `src/ir/verify/matching.rs:29-33` — drop the `.to_string()`, replace
      `is_result_type(&ty)` with `matches!(ty, ParameterType::ResultOf(_))` and
      the `ty == "Unknown"` / `ty == "Result"` compares with variant/`is_named`
      checks. The `union_variants`/`enums` lookups that need a name keep it —
      re-key them in letter C, not here.
- [ ] `src/ir/verify/link.rs:798` → `ParameterType::state`; `:898` →
      `typed_is_collection_type`; `:926` → feed `without_state()`.
- [ ] `src/ir/verify/calls.rs:264,265` → `ParameterType::state`; `:278` →
      `ParameterType::without_state`.
- [ ] `src/ir/verify/mod.rs:1182` → typed input via `without_state`.
- [ ] `src/ir/shape.rs:2259,2260` → `ParameterType::without_state`.
- [ ] `src/monomorph/lower.rs:265` → `ParameterType::without_state`.
- [ ] `src/cli/build/mod.rs:427` → `ParameterType::without_state` (drops the
      `.name()` render on `signature.returns`).
- [ ] Add `typed_is_builtin_sendable_resource_type(&ParameterType)` beside the
      `&str` original in `src/codegen/resource/`, and call it from
      `src/ir/verify/resources.rs:314`. (Adding a typed twin is permitted; the
      `&str` original dies in letter E.)
- [ ] Lower the gate's `ir`, `monomorph` and `cli` budgets by what this phase
      removed, in this phase's commit.

Acceptance: `rg -n 'codegen::(engine::types|resource)::' src/ir src/monomorph src/cli --glob '!**/tests*'`
returns only typed-twin calls (no `&str` helper); `cargo test --no-fail-fast -- --skip artifact_gate_all`
green; `scripts/diag-set-diff.sh` 0 differing.
Commit: —

### Phase 2 — `ir::verify` and `ir::shape` take types, not spellings

- [ ] Convert the 6 `&str` type parameters in `src/ir/verify/mod.rs` to
      `&ParameterType`, then **delete `resource_base_type_name`**
      (`src/ir/verify/mod.rs:1200`) — with no `&str` callers it is dead.
- [ ] Convert the 4 in `src/ir/verify/compat.rs` and delete its 3 parses.
- [ ] Convert the 1 in `src/ir/verify/values.rs` and rewrite its 6 spelling
      decisions (`check_const_literal`, `src/ir/verify/values.rs:443-580`) as
      matches on `ParameterType` variants. The literal-range rules
      (`TYPE_BYTE_LITERAL_OVERFLOW`, `TYPE_INTEGER_LITERAL_OVERFLOW`,
      `TYPE_FLOAT_LITERAL_OVERFLOW`, and the `Fixed`/`Money` arms) must fire on
      exactly the same inputs — this is the phase's diagnostic-equality risk.
- [ ] Convert the 1 in `src/ir/link.rs`.
- [ ] Replace `src/ir/shape.rs:432`'s `type_.name() == "Scalar"` with
      `type_.is_named("Scalar")` (`src/types.rs:545`).
- [ ] Delete the 6 parses in `src/ir/shape.rs`, the 4 in `src/ir/lower.rs`, the 3
      in `src/ir/resource_escape.rs` and the 2 in `src/ir/value.rs`, typing each
      caller instead.
- [ ] Rewrite `src/ir/lower.rs:3994`'s `name == "Error"` and its 2 spelling arms
      as variant/`is_named` checks.
- [ ] Lower the `ir` budgets to 0 for `parse_sites` (outside `binary.rs`),
      `str_type_params`, `spelling_match_arms` and `spelling_compares`, in the
      commits that clear them.
- [ ] Tests: the existing `ir/verify` unit tests and `ir/tests.rs` cover these
      rules; add no new tests unless a conversion reveals an uncovered arm — and
      if one does, that is a coverage gap to record in Corrections.

Acceptance: `rg 'ParameterType::parse\(' src/ir` returns hits only in
`src/ir/binary.rs`; the `ir` gate budgets are 0; `cargo test --no-fail-fast -- --skip artifact_gate_all`
green; `scripts/diag-set-diff.sh` 0 differing with
`[exit N]` captured.
Commit: —

### Phase 3 — `monomorph` and `resolver` (largest correctness risk in this letter)

- [ ] **First, separate the two domains in `src/monomorph/`:** list each of the
      26 parses and classify it as (a) *building or reading a mangled
      instantiation key* — a symbol, which may stay a string — or (b) *deciding
      something about a type* — which must not. Record the split in Corrections
      before converting.
- [ ] Convert every (b) site to `ParameterType`. `src/monomorph/lower.rs:237`'s
      `p == a || *p == "Unknown" || *a == "Unknown"` becomes a variant match; the
      `Unknown` arm must remain a **refinable provisional binding** per bug-442
      Option B, not a drop or a wildcard.
- [ ] Convert the 3 `&str` type params in `src/monomorph/lower.rs` and 4 in
      `src/monomorph/helpers.rs`.
- [ ] `src/resolver/resolution.rs:1404` — `name == "Unknown"` is an AST-domain
      query on a template parameter name; verify that from the code and either
      convert it or record it in Corrections as AST-domain with the evidence.
      Do not leave it unclassified.
- [ ] Convert the 2 `&str` type params in `src/resolver/mod.rs` and 1 in
      `src/resolver/resolution.rs`.
- [ ] Lower the `monomorph` and `resolver` budgets to 0, in the clearing commits.
- [ ] Tests: add a monomorph regression pinning the `Unknown` refinement —
      instantiate a generic over an empty collection and assert the later
      concrete occurrence refines the binding (the `collections::flatten$Unknown`
      hard-error is the failure mode).

Acceptance (this is also the letter's end-of-letter gate): all six gate budgets
for `ir`, `monomorph` and `resolver` read 0; `cargo test --no-fail-fast` green
green;
`scripts/diag-set-diff.sh` 0 differing (B's end-of-letter diagnostic sweep).
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast -- --skip artifact_gate_all` — `--no-fail-fast` or
  the `rt_*` tests are skipped; `--skip` or the run is the full artifact sweep.
- Gate: `cargo test --test no_type_strings` — `ir`, `monomorph` and `resolver`
  budgets at 0 and tight.
- Coverage check: `ir/verify/values.rs`'s literal-range arms are the one place a
  conversion could silently stop firing. Confirm each rule code still has a
  fixture hit in the corpus rather than assuming the suite covers it.
- Runtime proof: **deferred to letter G.** No `test-accept.sh` run in this
  letter — the acceptance corpus and its goldens are swept once, at the end
  (plan-111-A §3). The per-phase `rt_*` runtime tests are this letter's
  behavioral signal.

- Artifact gate / goldens: **not run in this letter** (plan-111-A §3).
  `artifact-gate.sh all`, `tests/golden.rs` and `test-accept.sh` all run once,
  in letter G, where any diff is attributed before any golden is regenerated.
- Diagnostics: `scripts/diag-set-diff.sh` → 0 differing, capturing `[exit N]` and
  bare `error:` lines.
- Doc sync: repoint any comment in `src/ir/**` that describes the "name-domain
  twin" pattern — it stops being true in Phase 2.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`monomorph`'s instantiation keys** — recommended: mangled instantiation
  names stay `String` (they are symbols, and `type_instantiations` is a symbol
  table), while every type *decision* that builds them becomes typed. The
  alternative — keying instantiations by `ParameterType` too — is a larger change
  with no bearing on this plan's goal and should be rejected unless Phase 3's
  classification shows the two domains cannot be separated. (§Phase 3)
- **`src/ir/shape.rs`'s `arg_types: &[String]`** — recommended: leave it to
  letter C, which owns the registry signature it feeds. Converting it here would
  mean hand-rolling a typed path around the registry, braiding B into C.
  (§2 Verified properties)

## Corrections

<Filled in DURING execution.>

## Summary

Risk is concentrated in two places: `ir/verify/values.rs`'s literal-range arms
(a converted arm that stops firing is a silently dropped diagnostic, which
`diag-set-diff.sh` catches) and `monomorph`'s `Unknown` handling (a wildcard
where a provisional binding belongs re-opens bug-442, which only the new
refinement regression catches).

Untouched: all of `src/codegen` (letters C–F), `src/ir/binary.rs` (boundary #2),
and the registry's string API (letter C).
