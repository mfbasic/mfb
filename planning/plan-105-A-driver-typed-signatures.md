# plan-105-A: Kill the driver's type-string round-trip (typed external signatures)

Last updated: 2026-08-24
Overall Effort: x-large (1d–3d) — the whole plan-105 feature
Effort: large (3h–1d)
Depends on: nothing (plan-102 is a prerequisite; see gate)

Delete the build driver's structured→string→structured type round-trip and carry
imported-package signatures as **typed data** end to end. This is the third
outcome named by the architectural review's Recommendation #1
(`planning/Compiler Pipeline.md:58,67`): the driver currently formats a package
export into a `FUNC(…) AS T` **string**, then re-parses the return type back out
of it with `signature.rsplit_once(" AS ")` (`src/cli/build/mod.rs:411`), and
three helpers must stay in lockstep for the filter not to silently break.

This sub-plan is the **lead document for plan-105**, which finishes the
review's Recommendation #1 outcomes that plan-102/104 do not cover:
**A** (this) = the driver round-trip; **B** = one type-grammar implementation
(resolver/monomorph/syntaxcheck hand-parsers collapse onto
`ParameterType::parse`, plus the `UserOf` variant). The review is the mandate;
each phase below cites the review bullet it discharges.

References:

- `planning/Compiler Pipeline.md` — the review; §"Driver + cross-cutting"
  (`:58`) and Recommendation #1 (`:67`).
- `src/cli/build/mod.rs` — `external_package_function_types`,
  `imported_type_defs`, `imported_resource_closers` (the three lockstep
  helpers), the `rsplit_once(" AS ")` at `:411`.
- `src/ir/lower.rs` — `lower_augmented_project` /
  `lower_project_with_external_functions` signatures
  (`external_function_types: &HashMap<String, String>`), and the string
  splitters that re-parse those signature strings
  (`function_return_from_type`, `function_param_types_from_type`).
- `src/binary_repr/` — where package export signatures are decoded (the true
  source of the data the driver re-derives from strings).

## Prerequisites

Shared by both plan-105 sub-plans; stated once here.

| Must be true | Command | Status |
|---|---|---|
| plan-102 complete and landed | `ls planning/plan-102-* 2>/dev/null` → no matches (archived) | MET (verified 2026-08-24) |
| On a feature worktree, not main | `git rev-parse --abbrev-ref HEAD` ≠ `main` | NOT MET — create `worktree-P-105` |
| Baseline gate captured | `scripts/artifact-gate.sh target/release/mfb all` → record to `planning/plan-105-baseline-diffs.txt` (0 diffs at plan-writing time) | NOT MET — run first |
| Full suite green | `rustup run 1.96.0 cargo test --no-fail-fast` | NOT MET — run first (green at plan-writing time) |

plan-105 does NOT depend on plan-104 (its scope — driver/resolver/monomorph —
is disjoint from NIR/codegen); the two may land in either order. plan-106
requires BOTH.

Known pre-existing noise: `test-accept`'s 2 environmental mismatches (the 5
stdin-EOF `acceptance` sub-tests + the `project_name` harness path bug) — judge
"no NEW mismatch" against that pair.

## 1. Goal

- `cli/build/mod.rs:411`'s `rsplit_once(" AS ")` **does not exist**; no code
  anywhere re-parses a type out of a formatted signature string.
- Imported-package function signatures flow as one typed struct (params:
  `Vec<ParameterType>`, returns: `ParameterType`, plus the resource/state facts
  the three helpers currently re-derive) from the `.mfp` decode boundary to
  `ir::lower`, replacing the `HashMap<String, String>` signature maps
  (9 threading sites: `rg -c 'external_function_types|external_function_params'
  src/cli/build/mod.rs src/ir/lower.rs` → 9).
- The three lockstep helpers (`external_package_function_types`,
  `imported_type_defs`, `imported_resource_closers`) become views over that one
  typed decode — no format-dependent agreement left to maintain.
- `ir::lower`'s signature-string splitters (`function_return_from_type`,
  `function_param_types_from_type`) are deleted (their input no longer exists).

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical `.ncode`/`.ncodesum`/`.ir`/`.nir`
  vs the captured baseline (this is a data-plumbing refactor; byte-identity is
  the correct gate).
- **No `.mfp` wire-format change.** The package file still stores signature
  strings; the decode boundary parses them ONCE via `ParameterType::parse` —
  that is a permitted parse-in boundary, not a round-trip.
- No behavior change to which imports resolve or which diagnostics fire.
- Driver-duplication cleanup beyond the signature plumbing (the review's
  Rec #6 `lower_augmented_project` preamble extraction) is NOT this plan — do
  not braid it in.

## 2. Current State

The driver reads installed packages, formats each export into a
`FUNC(p1, p2) AS R` string, stores it in `external_function_types:
HashMap<String, String>`, and later `rsplit_once(" AS ")` re-parses the return
type out of that string to filter resource-returning imports
(`src/cli/build/mod.rs:405-415` — read it). `ir::lower` receives the string map
and re-parses it again via `function_return_from_type` /
`function_param_types_from_type`. The review calls this "a self-documented
hack" whose format is a silent coupling (`Compiler Pipeline.md:58`).

### Measured populations

| What | Count | Command |
|---|---|---|
| the round-trip site | 1 | `rg -n 'rsplit_once\(" AS "\)' src/cli/build/mod.rs` → `:411` |
| String signature-map threading sites | 9 | `rg -c 'external_function_types\|external_function_params' src/cli/build/mod.rs src/ir/lower.rs` → 9 |
| lockstep helpers | 3 | `external_package_function_types`, `imported_type_defs`, `imported_resource_closers` (`rg -n 'fn (external_package_function_types\|imported_type_defs\|imported_resource_closers)' src/cli/build/mod.rs`) |
| `ir::lower` signature-string splitters to delete | 2 | `rg -n 'fn function_return_from_type\|fn function_param_types_from_type' src/ir/lower.rs` |
| `lower_augmented_project` call sites receiving the maps | 5 | `rg -n 'lower_augmented_project' src/cli/build/mod.rs \| wc -l` |

### Verified properties

- **The `.mfp` decode already yields structured export data** (names, param
  lists, return types as stored strings) — the driver *flattens* it to
  `FUNC(…) AS …` strings and then re-derives structure. Read
  `external_package_function_types`'s body before designing the struct: the
  typed form is what the decode already has in hand, parsed once. UNVERIFIED in
  detail (exact decode fields) — Phase 1's first task is reading the three
  helpers + the decode they consume and recording the field inventory here.

## 3. Design Overview

One typed struct at the decode boundary, threaded everywhere the strings went:

```rust
pub(crate) struct ExternalSignature {
    params: Vec<ExternalParam>,      // type_: ParameterType, has_default: bool
    returns: ParameterType,
    isolated: bool,
    return_resource: bool,           // what rsplit_once(" AS ") existed to learn
    return_state: Option<ParameterType>,
}
```

`external_package_function_types` becomes `external_signatures(…) ->
HashMap<String, ExternalSignature>` (parse each stored type string ONCE at
decode); the resource-import filter reads `sig.return_resource` /
`sig.returns` structurally; `imported_type_defs` / `imported_resource_closers`
derive from the same decode pass. `ir::lower` takes the typed map; its two
string splitters die; the `CallParam`/`function_returns` entries it builds from
externals render `.name()` only where its (still-string, until plan-106)
internal inference maps require it.

**Byte-identity is the gate.** Same class as plan-102/104: the parse↔name
bijection means the typed plumbing carries exactly the strings' information; a
NEW diff is a bug to objdump/root-cause, never a design verdict.

**Correctness risk:** the resource-import filter — the one behavior the
round-trip implemented. Its typed replacement must reproduce the filter's
current accept/reject set exactly; the byte-identity corpus plus the
package-import fixtures (`tests/` rt-behavior imported-resource suites) gate it.

### Rejected alternatives

- **Store `ParameterType` in the `.mfp`.** Rejected: wire-format change for no
  gain — parse-once at decode achieves the same with zero compatibility risk.
- **Keep the string maps and just fix the one rsplit site.** Rejected: leaves
  the format coupling and the lockstep-helper drift the review flagged; the
  round-trip is a symptom, the string signature map is the disease.

## Compatibility / Format Impact

None. `.mfp` bytes unchanged; `.ir`/`.nir` dumps unchanged; no CLI surface.

## Phases

> Tick boxes in the same commit as the work; `- [ ]` means NOT DONE.

### Phase 1 — decode-boundary inventory + typed struct

- [ ] Read the three lockstep helpers and the `.mfp` decode they consume;
      record the exact field inventory in §2 (replacing the UNVERIFIED note).
- [ ] Add `ExternalSignature` (+ `ExternalParam`) where the decode lives;
      build it parse-once from the stored strings; unit-test round-trip against
      a fixture package's known exports.

Acceptance: unit tests pass; `cargo test --no-fail-fast` green (struct not yet
consumed — no golden movement).
Commit: —

### Phase 2 — thread it; delete the round-trip and the splitters

- [ ] Replace the 9 `HashMap<String, String>` threading sites with the typed
      map (`cli/build/mod.rs`, all 5 `lower_augmented_project` call sites,
      `lower_project_with_external_functions`).
- [ ] Replace the `:411` filter with structural reads; delete
      `rsplit_once(" AS ")`.
- [ ] Delete `function_return_from_type` / `function_param_types_from_type`
      from `ir/lower.rs`.
- [ ] Re-derive `imported_type_defs` / `imported_resource_closers` from the
      same decode pass; delete their independent string derivations.
- [ ] Tests: existing package-import integration suites; fixtures constructing
      the old string maps updated to the struct.

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` no NEW diff
vs `planning/plan-105-baseline-diffs.txt`; `rg -n 'rsplit_once\(" AS "\)' src/`
→ 0 hits; **no-backward check**: `rg -n '"FUNC\(' src/cli/build/` shows no
signature-string *construction* remaining on the driver path (the review's
round-trip is gone in both directions).
Commit: —

## Validation Plan

- Tests: decode round-trip unit; package-import + imported-resource rt suites.
- Coverage check: every package-importing fixture flows through the typed map
  (the byte-identity corpus includes ~24 io-importing fixtures + the package
  suites).
- Runtime proof: `artifact-gate all` byte-identical; `test-accept` no NEW
  mismatch.
- Doc sync: none expected (`.mfp` format unchanged); check
  `.ai/resources-packages.md` for driver-plumbing descriptions.
- Acceptance: `cargo test --no-fail-fast`; `artifact-gate all`; `test-accept`;
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Where `ExternalSignature` lives:** `binary_repr` (with the decode) vs `ir`
  (with the consumer). Recommend `binary_repr` — it is decode-boundary data.

## Corrections

<Filled in during execution.>

## Summary

Small surface, high leverage: one struct, nine threading sites, three helpers
collapsed to one derivation, two splitters and the review's flagged hack
deleted. Risk concentrates in reproducing the resource-import filter exactly —
which the byte-identity corpus decides.
