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
| plan-111-A complete | `cargo test --test no_type_strings` passes; `rg -c 'Stateful' src/types.rs` → >0 | MET (2026-08-29): gate `4 passed; 0 failed`; `rg -c Stateful src/types.rs` → 22. A's three phases landed as ea2863d6b, 0d034a522, 5dfd69f80. |

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

### End-of-letter spot-check (scoped, read-only)

Before closing this letter, run the scoped artifact gate on the builtins it
touched — **`collections`, `strings`, `thread`** (`ir::verify`/`shape` rules and `monomorph`'s instantiation keys; `thread` covers the `ThreadHandle` planes):

```
scripts/artifact-gate.sh target/release/mfb collections
scripts/artifact-gate.sh target/release/mfb strings
scripts/artifact-gate.sh target/release/mfb thread
```

Measured cost: ~31s per builtin (one builtin = 1 test, 6 builds, 7 goldens).
This is **read-only diffing**: it regenerates nothing and updates no golden. It
is multi-target — per-target goldens (`*.linux-aarch64.ncode` and friends) are
discovered by filename and rebuilt with `-target`, so cross-arch drift is caught
on a macOS host, which no other per-letter check can see.

Expect **0 diffs**. A diff here is this letter's, which is the entire point of
running it now instead of discovering it in G behind six letters of churn —
root-cause it with objdump on one fixture and fix the conversion. **Do not
regenerate a golden here.** All regeneration happens once, in letter G, after
attribution (plan-111-A §3).

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

- Artifact gate: **scoped spot-check only** — the builtins above, ~31s each,
  read-only. The full `artifact-gate.sh all`, `tests/golden.rs`,
  `test-accept.sh` and every golden regeneration run once, in letter G.
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

**12 — the ratchet's `string_keyed_type_maps` population was under-counted by
15; corrected here, once, by a systematic census.** plan-111-A's curated
`TYPE_KEYED_TABLES` was assembled by grepping identifiers containing `type`,
which is why `TypeModel`'s fields had to be added by hand (none of
`record_fields` / `union_names` / `enum_members` contains `type`). The same
blind spot hid others.

Re-censused properly: every `HashMap<String, _>` / `HashSet<String>`
declaration in `src/` extracted with its doc comment, then classified by
whether the **key** is a type name. `ir::verify`'s `TypeEnv` alone holds
**nine** such tables — `records`, `unions`, `resource_closers`,
`resource_sendable`, `field_types`, `record_field_lists`, `enums`,
`type_decl_info`, `private_fields` — where three were listed. Three more were
missing elsewhere: `codegen/link/thunk/link_thunk.rs`'s
`record_native_resources`, `codegen/engine/validation/validation.rs`'s
`native_resources`, and `target/shared/runtime/usage.rs`'s
`resource_union_closes` (the first `target` row).

Budgets raised to the true population — `ir` 8 → 17, `codegen` 9 → 11, new
`target` 1 — for a corrected total of **36**, not 24. Raising is what the gate
permits and what honesty requires; the end state is unchanged, since every row
still reaches 0 by letter G. **Letter C's `TypeModel` population is unaffected
(still 9); letter F inherits the two extra codegen rows and letter G the
`target` one.**

Each new entry was read before listing. Deliberately still excluded, and named
in the constant's doc comment so they are not "fixed" later: `ir::verify`'s and
`monomorph`'s `globals`, codegen's and NIR's `resource_owners` /
`owner_collections`, `ir/shape.rs`'s `state_dropped`, and
`function_lowering.rs`'s `union_extract_reads` — every one keyed by a
**binding** or **local** name, which is legitimately a string.

**13 — a prerequisite no letter covered: `HirItem::Link` was never
elaborated. Landed here as a new phase (Phase 0).** Letter B's §1 requires no
parse in `src/ir` outside `binary.rs`, no `&str` type parameter in `ir` /
`resolver`, and no spelling decision in either. Three of its own listed tasks
could not be done without this:

- `verify/mod.rs`'s `resource_base_type_name` (its last `parse`) exists purely
  for `verify/link.rs:794,801`, which hold LINK spellings.
- `ir/link.rs:521`'s `return_type: &str` is `IrLinkFunction`'s wire field.
- `verify/link.rs:355`'s `function.return_type != "Nothing"`.

All three bottom out in the same fact, which the code stated in three places:
`HirItem::Link` carried the raw `crate::ast::LinkBlock`, the one item kind HIR
does not elaborate. `src/ir/lower.rs:2051` said so outright — *"this call site
IS that item kind's AST→typed boundary … elaborating `LinkBlock` properly is
recorded as a task in plan-106-E"* — and `resolver/resolution.rs` and
`verify/mod.rs` each carried the same admission. The task was recorded and never
scheduled, so no letter owned it.

Per the skill's "a prerequisite exists that no letter covers → land it", it is
done here:

- `hir::HirLinkBlock` / `HirLinkFunction` / `HirLinkParam` / `HirCStructDecl`
  carry `ParameterType`, built by `hir::elaborate_link_block` — in
  `src/hir/mod.rs`, boundary #3, where every other item's AST→type conversion
  already is. Non-type content (the native symbol, ABI slots, `CONST` pins,
  `BIND`/`BUFFER`/`SUCCESS_ON`/`RETURN`/`FREE` clauses) stays as its AST node,
  because none of it is type-domain.
- `IrLinkFunction`'s `params` / `return_type` / `return_state_type` and
  `IrCStruct`'s `maps_to` are typed. **The wire format is unchanged**:
  `src/ir/binary.rs` (boundary #2) renders on encode and parses on decode, so
  `.mfp` bytes are byte-identical. Only the in-memory shape moved.

What that removed, beyond the three tasks above:

- `resolver`'s `resource_base_type` — a **THIRD** hand-rolled copy of the
  `STATE` grammar, and the only one carrying no composite guard at all, so
  `List OF RES File STATE Cursor` would have truncated to `List OF RES File`
  (the bug-429 shape). It is now `ParameterType::without_state`.
  `hand_rolled_grammar/resolver` → **0**.
- `resolver::is_c_abi_type` and `verify::link`'s private twin: both matched a
  reject-list against a rendered name; both ask the interned `Symbol` now. The
  two lists stay deliberately separate (`verify`'s includes `CVoid`), as their
  comments require.
- `resolver::resolve_type_by_name`'s LINK callers, and with them 2 of the
  resolver's 3 `&str` type parameters.

**It also cleared codegen sites letter B does not own**, because the consumers
stopped being handed a spelling: `parse_sites/codegen` 96 → 94
(`builder/mod.rs` parsed `function.return_type` twice) and
`spelling_compares/codegen` 60 → 57 (`== "String"` on a LINK return, ×3). Those
are letters D–F's rows and they simply arrive smaller.

Scope note against letter B's "do not touch `src/codegen`": that non-goal is
about not converting codegen's `&str` helper *signatures* into D/E's work early.
No codegen signature changed here. The edits in `link_thunk.rs`,
`validation.rs`, `builder/mod.rs` and `nir/json.rs` are consumers rendering with
`.name()` at their own use site so the tree compiles — plus the two parses and
three compares that deleted themselves.

## Summary

Risk is concentrated in two places: `ir/verify/values.rs`'s literal-range arms
(a converted arm that stops firing is a silently dropped diagnostic, which
`diag-set-diff.sh` catches) and `monomorph`'s `Unknown` handling (a wildcard
where a provisional binding belongs re-opens bug-442, which only the new
refinement regression catches).

Untouched: all of `src/codegen` (letters C–F), `src/ir/binary.rs` (boundary #2),
and the registry's string API (letter C).
