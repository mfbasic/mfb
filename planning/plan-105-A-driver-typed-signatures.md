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
| On a feature worktree, not main | `git rev-parse --abbrev-ref HEAD` ≠ `main` | MET (verified 2026-08-24: `worktree-P-105`) |
| Baseline gate captured | `scripts/artifact-gate.sh target/release/mfb all` → record to `planning/plan-105-baseline-diffs.txt` (0 diffs at plan-writing time) | MET (verified 2026-08-24: 1249 tests, 1718 goldens, **0 diffs**) |
| Full suite green | `rustup run 1.96.0 cargo test --no-fail-fast` | MET (verified 2026-08-24: 62 suites `ok`, 0 `FAILED`) |

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
  `FUNC(…) AS …` strings and then re-derives structure. VERIFIED 2026-08-24 by
  reading the three helpers and the decode they consume. Field inventory
  (`src/binary_repr/mod.rs`):

  | Decode struct | Fields | Read by |
  |---|---|---|
  | `BinaryReprExport` | `name: String`, `kind: BinaryReprExportKind`, `isolated: bool`, `params: Vec<BinaryReprExportParam>`, `return_type: String` | `external_package_function_types*` |
  | `BinaryReprExportParam` | `name: String`, `type_: String`, `has_default: bool` | same |
  | `BinaryReprTypeExport` | `name`, `kind`, `fields`, `variants`, `members`, `foreign_owner` | `imported_type_defs*` |
  | `BinaryReprResourceExport` | `type_name: String`, `close_function: Option<String>`, `sendable`, `close_may_fail`, `native` | `imported_resource_closers` |
  | container header (`read_mfp_header`) | `name`, `version`, … | all three (for the `pkg.` qualifier) |

  Two facts the design turns on:
  1. `isolated` and `has_default` are the ONLY export fields the signature
     string could not carry structurally; `isolated` is what the `ISOLATED `
     prefix encoded, and `has_default` was already dropped on the floor by the
     old plumbing (`CallParam { default: None }` — `ir/lower.rs`).
  2. The three helpers each called `read_package_binary_repr` separately, so
     every dependency `.mfp` was read off disk and fully decoded **three times**
     per build (four counting `read_mfp_header`). Phase 2 collapses that to one
     decode per file via `binary_repr::BinaryReprPackageDecode`.

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

- [x] Read the three lockstep helpers and the `.mfp` decode they consume;
      record the exact field inventory in §2 (replacing the UNVERIFIED note).
- [x] Add `ExternalSignature` (+ `ExternalParam`) where the decode lives;
      build it parse-once from the stored strings; unit-test round-trip against
      a fixture package's known exports.
      — `ir::ExternalSignature { params: Vec<ExternalFunctionParam>, returns:
      ParameterType, isolated: bool }` in `src/ir/types.rs`, built by
      `manifest::package::package_export_signature`. `ExternalParam` was NOT
      added: `ir::ExternalFunctionParam` already is that struct, so the sketch's
      second type would have been a duplicate (see Corrections).
      Round-trip unit: `package_export_signature_round_trips_every_export_shape`
      (15 export shapes incl. STATE returns, `RES`, nested containers,
      higher-order params, `ISOLATED`, thread handles, user generics) asserts
      `signature_type().name()` equals the hand-formatted spelling exactly.

Acceptance: MET — `cargo test --no-fail-fast` green (61/61 non-gate suites `ok`;
the `golden.rs` gate suite was blocked by a peer session's artifact-gate lock and
was run standalone, see Phase 2).
Commit: 15f495ebc

### Phase 2 — thread it; delete the round-trip and the splitters

- [x] Replace the 9 `HashMap<String, String>` threading sites with the typed
      map (`cli/build/mod.rs`, all 5 `lower_augmented_project` call sites,
      `lower_project_with_external_functions`).
      — The two parallel maps (`external_function_types` +
      `external_function_params`) collapse to ONE `HashMap<String,
      ExternalSignature>`, so the 9 sites become 5 (one per call site) plus the
      2 function signatures. `src/testutil.rs:70` was a 10th site the plan's
      count missed (see Corrections).
- [x] Replace the `:411` filter with structural reads; delete
      `rsplit_once(" AS ")`. — `returns_imported_resource` now reads
      `signature.returns` directly; `grep -rn 'rsplit_once(" AS ")' src/` → 0
      live hits (2 doc-comment mentions of the deleted hack remain, by design).
- [x] Delete `function_return_from_type` / `function_param_types_from_type`
      from `ir/lower.rs`. — Deleted **as hand-parsers**: their `strip_prefix`
      cascades are gone, replaced by one `declared_func_parts` helper that
      matches `ParameterType::parse`'s `Func` arm. The plan's premise that
      "their input no longer exists" was FALSE — 3 of their 4 call sites read
      *local/global binding* type strings, not external signatures (see
      Corrections). This also fixed a latent bug in both.
- [x] Re-derive `imported_type_defs` / `imported_resource_closers` from the
      same decode pass; delete their independent string derivations.
      — New `binary_repr::BinaryReprPackageDecode`: one
      `read_package_binary_repr` per `.mfp`, with `name()`/`exports()`/
      `type_exports()`/`resources()` read off it. Each accessor still returns a
      `Result`, preserving today's per-section lossiness exactly.
      `read_mfp_header` is no longer called by any of the three (the manifest
      name comes off the same decode, and the container↔manifest identity check
      guarantees it is the same string).
- [x] Tests: existing package-import integration suites; fixtures constructing
      the old string maps updated to the struct (`src/ir/tests.rs` ×5,
      `src/manifest/package.rs` ×5, `src/testutil.rs` ×1).

Acceptance: MET (2026-08-24).
- `rustup run 1.96.0 cargo test --no-fail-fast` → 61 suites `ok`, 0 `FAILED`
  (the `golden.rs` suite aborted on a peer session's artifact-gate lock —
  "Another artifact-gate (pid 82196) is running" — and was run standalone below).
- `scripts/artifact-gate.sh target/release/mfb all` → **1249 tests, 1396
  build(s), 1718 golden(s) checked, 0 diff(s)** — byte-identical to
  `planning/plan-105-baseline-diffs.txt`. No NEW diff.
- `scripts/test-accept.sh target/release/mfb /tmp/accept-p105` → 2 mismatches
  over 1193 tests: exactly the documented pre-existing pair (the 5 stdin-EOF
  `io` sub-tests, and the `project_name` truncated-path harness bug). No NEW
  mismatch. **Both later disappeared**: after merging main (which brought
  plan-100's fix to `scripts/test-accept.sh`), the same tree runs
  `acceptance tests passed (1271 test(s) ran)` — 0 mismatches. See Corrections 7.
- `grep -rn 'rsplit_once(" AS ")' src/` → 0 live hits.
- **no-backward check**: `grep -rn '"FUNC(' src/cli/build/` → 0 hits. No
  signature-string construction remains on the driver path; the round-trip is
  gone in both directions.
Commit: 15f495ebc

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

- **Where `ExternalSignature` lives:** ~~`binary_repr` (with the decode) vs `ir`
  (with the consumer). Recommend `binary_repr` — it is decode-boundary data.~~
  RESOLVED (2026-08-24): **`ir` (`src/ir/types.rs`)**, against the plan's
  recommendation. Two reasons the recommendation did not survive contact:
  1. `binary_repr`'s structs are the *wire* domain — every type field there is a
     `String`, because that is what the `.mfp` stores. `ExternalSignature`
     carries `ParameterType`, so putting it there would make the wire-decode
     module depend on `crate::types` for a struct it never constructs.
  2. `ir::ExternalFunctionParam` — the per-parameter half of exactly this data —
     already lived in `ir::types` and is already `ParameterType`-typed. Splitting
     the pair across two modules would re-create in miniature the "several places
     must agree" problem this plan exists to remove.

  The parse-once still happens at the decode boundary
  (`manifest::package::package_export_signature`); only the struct's *home*
  moved. `binary_repr` did gain a type — `BinaryReprPackageDecode`, which is
  wire-domain and belongs there.

## Corrections

1. **"Delete `function_return_from_type` / `function_param_types_from_type`
   (their input no longer exists)" — FALSE.** Only 1 of their 4 call sites read
   an external signature string (`ir/lower.rs:181`). The other 3
   (`lower.rs:2144`, `:2153`, `:2288`) read a **local's or a global binding's
   declared type** — `locals: &HashMap<String, String>` and
   `LowerContext::binding_types: HashMap<String, String>`, both HIR-derived and
   still string-typed until plan-106. Deleting the functions outright would have
   broken first-class-function-value typing.

   Measured: `grep -rn 'function_return_from_type\|function_param_types_from_type' src/`
   → 4 call sites + 2 definitions; `grep -n 'locals: &HashMap' src/ir/lower.rs`
   → `HashMap<String, String>`.

   **Repair:** deleted the hand-rolled *grammar* (which is what the review
   objected to) rather than the functions. Both now delegate to a single
   `declared_func_parts` that matches `ParameterType::parse`'s `Func` arm, so
   `ir/lower.rs` holds no copy of the type grammar. The plan's task text is
   corrected in place.

2. **The old splitters were buggy, and the typed replacement fixes them.**
   `function_return_from_type` cut at the **first** `") AS "` and
   `function_param_types_from_type` split parameters on a bare `", "`. For a
   higher-order declared type — `FUNC(FUNC(Integer) AS String) AS File` — that
   yields return type `String) AS File` and the single truncated parameter
   `FUNC(Integer`. `ParameterType::parse` splits at paren depth 0 and returns
   `File` / `[FUNC(Integer) AS String]`. Not a behavior change the gate can see
   (0 diffs over 1718 goldens), because no corpus fixture declares a binding of
   higher-order function type — but it is a real latent fix, so a regression case
   is pinned in `package_export_signature_round_trips_every_export_shape`.

3. **`ExternalSignature`'s field list is smaller than the sketch.** §3 proposed
   `has_default`, `return_resource` and `return_state`. All three were dropped:
   - `has_default` is *already* discarded by the consumer
     (`ir/lower.rs` builds `CallParam { default: None }` unconditionally), so
     carrying it would have been a field nothing reads.
   - `return_resource` cannot be decode-boundary data — it is a predicate over
     `imported_resource_closers`' output, which the driver does not have when the
     signature is decoded. It stays a driver-side closure
     (`returns_imported_resource`), now reading `sig.returns` structurally.
   - `return_state` is read by nobody; the STATE clause rides inside the nominal
     leaf (`Named("SoundFile STATE FileInfo")`) and `base_resource_name` strips it
     at the one place that cares.

   Adding any of the three would have violated the project's no-dead-code rule.

4. **`ExternalParam` was not added** — `ir::ExternalFunctionParam` is already
   exactly that struct (`{ name: String, type_: ParameterType }`) and is already
   `ParameterType`-typed, so the sketch's new type would have been a duplicate.

5. **The "9 threading sites" count was right but the *shape* was wrong, and it
   missed one file.** `grep -c 'external_function_types\|external_function_params'
   src/cli/build/mod.rs src/ir/lower.rs` → 9 as the plan states. But the two maps
   were always passed as a *pair*, so the typed form collapses them into one
   parameter: the 5 `lower_augmented_project` / `lower_project_with_external_
   functions` call sites each lost an argument rather than gaining a typed one.
   The plan's grep also scoped only those two files and so missed
   `src/testutil.rs:70`, a 10th site that had to change for the crate to compile.

6. **The three helpers were decoding each `.mfp` three times over.** Not stated
   in the plan, found while collapsing them: `external_package_function_types`,
   `imported_type_defs` and `imported_resource_closers` each called
   `binary_repr::read_package_binary_repr` independently (plus `read_mfp_header`,
   a fourth read of the same file). The Phase-2 task "re-derive … from the same
   decode pass" is therefore satisfied literally: `BinaryReprPackageDecode` reads
   and decodes each dependency once and all three views come off it.

   Deliberately **not** collapsed: the per-section `Result`s. A first cut returned
   one all-or-nothing struct, which would have changed lossy-path behavior — a
   package with an unreadable resource table would have stopped contributing its
   *function signatures* too. The accessors each return `Result` so recovery stays
   exactly as it was.

7. **The "2 known pre-existing `test-accept` mismatches" were a harness bug, and
   they are gone.** This plan's Prerequisites called them "environmental noise"
   and told the implementer to judge "no NEW mismatch" against that pair. That
   framing was wrong twice over: reproducing on main dates a bug as pre-existing,
   it never makes it environmental — and treating it as noise is what kept it
   alive.

   The real cause (found by the concurrent plan-100 work, confirmed
   independently from this worktree): `scripts/test-accept.sh`'s behavioral-test
   branch ran `mfb test` bare instead of through `run_with_watchdog`, so it
   inherited the driving `while read … < <(find …)` loop's pipe as stdin. Any
   fixture whose `TESTING` blocks read stdin consumed the fixture list — which
   both produced the 5 stdin-EOF `io` sub-test failures AND silently skipped
   fixtures (the truncated `fb/.claude/…` path in the `project_name` error is
   the corrupted read).

   After merging main, this tree runs **`acceptance tests passed (1271 test(s)
   ran)`** — 0 mismatches, up from 1199 tests run. So plan-105 introduced no
   acceptance mismatch, and the pair the Prerequisites told us to tolerate no
   longer exists.

## Summary

Small surface, high leverage: one struct, nine threading sites, three helpers
collapsed to one derivation, two splitters and the review's flagged hack
deleted. Risk concentrates in reproducing the resource-import filter exactly —
which the byte-identity corpus decides.