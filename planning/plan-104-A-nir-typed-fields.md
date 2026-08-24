# plan-104-A: Typed NIR data model (flip the 25 type fields to ParameterType)

Last updated: 2026-08-24
Overall Effort: huge (>3d) — the whole plan-104 feature
Effort: large (3h–1d)
Depends on: nothing (plan-102 is a prerequisite, not a dependency — see gate)

Flip the NIR data model's type-spelling fields from `String` to
`crate::types::ParameterType`, so the typed pipeline plan-102 built (HIR →
monomorph → IR, all `ParameterType`) extends one layer further down instead of
rendering back to strings at the IR→NIR boundary. This is the foundation the
rest of plan-104 (converting codegen's string type compares to native matches)
builds on. After this sub-plan the boundary *stops doing work*: `nir::lower`
today renders 19 `.name()` strings per node kind; it will clone the IR's
already-typed fields instead.

This sub-plan is also the **lead document** for the plan-104 feature: it carries
the shared goal, prerequisites, measured populations, design overview, and the
A→D roadmap. Sub-plans B–D are complete plans in their own right but do not
restate the shared design — read this first.

References:

- `src/target/shared/nir/mod.rs` — the NIR data model (`NirModule`, `NirValue`,
  `NirOp`, `NirFunction`, …).
- `src/target/shared/nir/lower.rs` — the IR→NIR boundary (`lower_module`,
  `merge_packages`).
- `src/target/shared/nir/json.rs` — the `.nir` JSON dump serializer (a
  **golden-checked artifact**: per-target `*.nir` files under `tests/`, checked
  by `scripts/artifact-gate.sh`).
- `src/target/shared/nir/constfold.rs`, `visit.rs` — the const-folder and the
  shared value walker.
- `src/types.rs` — `ParameterType` (interned `Symbol` leaves, complete
  vocabulary incl. `MapEntryOf`/`ResultOf`, post-plan-102-A).
- `src/docs/spec/architecture/13_native-ir.md` — the NIR spec chapter (describes
  the *serialized* JSON, which stays byte-identical).
- `.ai/testing-gates.md`, `.ai/codegen-invariants.md`.
- `planning/completed/plan-102-B-typed-ir.md` — the direct precedent (the same
  flip one layer up), including the Correction that discovered codegen consumes
  NIR, not IR — the gap this plan closes.

## Prerequisites

Shared by every plan-104 sub-plan; stated once here.

| Must be true | Command | Status |
|---|---|---|
| plan-102 complete and landed on main | `ls planning/plan-102-* 2>/dev/null` → no matches (all six letters in `planning/completed/`) | MET (re-verified 2026-08-24 in worktree: no matches, all six in `planning/completed/`) |
| On a feature branch/worktree, not main | `git rev-parse --abbrev-ref HEAD` (≠ `main`) | MET — `worktree-P-104` at `.claude/worktrees/P-104` |
| Baseline gate captured | `scripts/artifact-gate.sh target/release/mfb all` → record the DIFF set to `planning/plan-104-baseline-diffs.txt` | MET 2026-08-24 — 0 diffs (`artifact-gate [all]: 1249 tests, 1396 build(s), 1718 golden(s) checked, 0 diff(s)`), recorded |
| Full suite green at HEAD | `rustup run 1.96.0 cargo test --no-fail-fast` | MET 2026-08-24 — run on a pristine detached worktree at main tip `cc93d4d67` (`git worktree add --detach /tmp/p104-baseline`): exit 0, 0 FAILED suites |
| Lowering perf baseline captured | `scripts/bench-lowering.sh target/release/mfb` → record the three probe times to `planning/plan-104-bench-baseline.txt` | MET 2026-08-24 — trivial 0.65/0.35, one-regex 30.72/6.74, acceptance 276.06/49.85 (debug/release s), recorded |

Everything below is written against the world where these hold.

> The Command column is the truth; re-run and update Status before continuing
> and before deciding to stop. The whole feature is behavior-preserving, so the
> baseline `.ncode`/`.ncodesum`/`.nir` diff set MUST be captured before phase 1 —
> every phase's acceptance is "no NEW diff vs this baseline". The perf baseline
> exists because this feature is perf-motivated AND its Phase-A shims can
> *transiently* add allocations (see §3 risk); the final state (after D) must
> not be slower than the baseline.

Known pre-existing noise (do not re-diagnose): `test-accept` fails 2 mismatches
in this environment even on clean main — the 5 stdin-EOF `acceptance` io
sub-tests (reproduced with a main-tip binary) and the `project_name` harness
path bug under deep worktree paths. Judge test-accept as "no NEW mismatch"
against that pair.

## 1. Goal

- Every NIR type-spelling field is `ParameterType`, not `String`: the 25 fields
  across `NirEntryPoint.returns`, `NirField.type_`, `NirImport.returns`,
  `NirImportParam.type_`, `NirGlobal.type_`, `NirFunction.returns`,
  `NirParam.type_`, `NirOp::{Bind,StoreGlobal,For,ForEach}.type_`, and
  `NirValue::{Const,LocalRef,Global,FunctionRef,Closure,Capture,Constructor,
  UnionWrap(union_type,member_type),UnionExtract,WithUpdate,ListLiteral,
  SetLiteral,MapLiteral}`.
- The IR→NIR boundary (`nir::lower`) **clones** the IR's `ParameterType` fields
  instead of rendering them (`.name().into_owned()` deleted); nothing below the
  boundary re-parses a type string it got from the IR.
- The `.nir` JSON dump renders `name()` at the emit point only
  (`nir/json.rs`), so the golden-checked `*.nir` artifact bytes are unchanged.

### Non-goals (explicit constraints)

- **No change to compiled output.** Byte-identical `.ncode`/`.ncodesum`/`.nir`/
  `.nplan`/`.nobj` across all targets — this is a pure representation swap, the
  same provably-neutral class as plan-102, and byte-identity is the correct
  primary gate.
- No change to the NIR JSON schema or bytes (`13_native-ir.md` documents the
  serialized form; it stays true verbatim).
- `kind`/`visibility`/`name`/`member`/`op`/`symbol`/`target` fields are NOT type
  spellings and stay `String` (exactly as plan-102-B kept `IrType.kind`).
- Do **not** convert codegen consumers natively in this sub-plan — they compile
  via `.name()` shims at read sites; the native conversion is B/C/D. (Same
  staging as plan-102-B Phase 1.)
- `NirValue::Call`/`CallResult`/`RuntimeCall`/`ResultValue`/`ResultIsOk`/
  `ResultError`/`MemberAccess`/`Binary`/`Unary`/`Local` carry **no** type field
  today (unlike their `IrValue` counterparts — NIR lowering drops the
  annotations codegen re-derives); this plan does NOT add type fields to them.
  Adding annotations would change the `.nir` dump bytes and is out of scope.

## 2. Current State

`nir::lower_module`/`merge_packages` (`src/target/shared/nir/lower.rs`) convert
the typed IR (post-plan-102: all `ParameterType`) into NIR by **rendering**
every type field back to a `String` — 19 `.name()` calls
(`rg -c '\.name\(\)' src/target/shared/nir/lower.rs` → 19), e.g.
`type_: binding.type_.name().into_owned()` (`lower.rs:132`). Codegen (192,071
lines: `find src/codegen -name '*.rs' | xargs wc -l` → 192071) then does string
type work on those fields for the rest of compilation. The three
`constfold.rs` folds construct `type_: "String".to_string()` Consts
(`constfold.rs:28,34,40`).

plan-102-B's Correction documented this exact gap: "codegen consumes NIR, not
IR — the `.name().into_owned()` renders at the IR→NIR boundary ARE the correct
final form **for plan-102-B**; typing NIR is a distinct, larger effort." This
plan is that effort.

### Measured populations

| What | Count | Command |
|---|---|---|
| NIR `String` type-spelling fields (`nir/mod.rs`) | 25 | `rg -c 'type_: String\|returns: String\|union_type: String\|member_type: String' src/target/shared/nir/mod.rs` → 25 |
| IR→NIR boundary renders to delete | 19 | `rg -c '\.name\(\)' src/target/shared/nir/lower.rs` → 19 |
| `.nir` JSON emit sites touching type/returns fields | 53 | `rg -c 'type_\|returns' src/target/shared/nir/json.rs` → 53 (upper bound; the emit seam) |
| `.type_` reads in `src/codegen/` (shim/convert blast radius) | 533 | per-submodule: engine 123, builtins 298, memory 30, collection 68, cleanup 5, error 9 (`rg -c '\.type_\b' src/codegen/<sub>` each) |
| `.type_` reads in `src/target/` (backends + nir itself) | 16 | `rg -c '\.type_\b' src/target/ \| awk -F: '{s+=$2} END{print s}'` → 16 |
| codegen scalar `== "Integer"`-style compares | 80 | `rg -n '== "(Integer\|String\|Boolean\|Float\|Fixed\|Byte\|Money\|Nothing\|AttributeString\|Scalar)"' src/codegen/ \| wc -l` → 80 (32 files: builtins 16, engine 8, memory 4, link/error/collection/cleanup 1 each) |
| codegen structural `strip_prefix("List OF ")`-style tests | 41 | `rg -n 'strip_prefix\("(List OF \|Set OF \|Map OF \|RES \|MapEntry OF \|Result OF \|Thread OF \|ISOLATED )\|starts_with\("(List OF \|Set OF \|Map OF \|RES \|FUNC)' src/codegen/ \| wc -l` → 41 |
| codegen `format!("List OF …")`-style type builds | 23 | `rg -n 'format!\("(List OF\|Set OF\|Map OF\|Result OF\|MapEntry OF)' src/codegen/ \| wc -l` → 23 |
| backend (`src/target/`) scalar compares / structural tests | 11 / 3 | same patterns over `src/target/` |
| `static_nir_value_type` callers | 12 | `rg -c 'static_nir_value_type' src/ \| awk -F: '{s+=$2} END{print s}'` → 12 |
| codegen `HashMap<String, String>` occurrences (type-valued AND symbol maps mixed) | 365 | `rg -c 'HashMap<String, String>' src/codegen/ \| awk -F: '{s+=$2} END{print s}'` → 365 — **only type-valued ones convert; triaged in B** |
| NIR module size | 2,770 | `find src/target/shared/nir -name '*.rs' \| xargs wc -l` → 2770 |

### Verified properties

- **The `.nir` dump is a golden-checked artifact.** Per-target `*.nir` goldens
  exist (`find tests -name '*.nir'` → e.g.
  `tests/rt-behavior/control-flow/control-flow-if/golden/control_flow_if.macos-aarch64.nir`)
  and `artifact-gate.sh` regenerates/diffs them. So the JSON emit must render
  `name()` byte-identically. VERIFIED (found goldens; read the gate's artifact
  list at `scripts/artifact-gate.sh:127`).
- **`ParameterType::parse↔name` round-trips byte-exact** and the whole plan-102
  pipeline already depends on it (see memory/plan-102 docs). Cloning the IR's
  typed field and rendering `name()` at the JSON emit therefore produces the
  same bytes the current render-at-boundary produces. VERIFIED by plan-102's
  landed gates (0 diffs) plus the round-trip unit tests in `src/types.rs`.
- **`NirValue::Call/CallResult/ResultValue` carry no `type_` field** — read
  `src/target/shared/nir/mod.rs:287-325`. Codegen re-derives those types via
  `static_nir_value_type` + the registry return-type oracle
  (`src/codegen/engine/types/type_utils.rs:19-60`). So the flip does not need to
  invent annotations. VERIFIED (read both files).
- **`LocalValue.type_` is the builder's per-local type string**
  (`src/codegen/engine/builder/mod.rs:414-415`) and `FieldTypes` is
  `HashMap<(String,String),String>` (`type_utils.rs:16`) — the two central
  codegen type stores B converts. VERIFIED (read).

## 3. Design Overview

The A→D layering (letter order = implementation order):

```
IR (ParameterType) ──nir::lower (CLONE, no render)──→ NIR (ParameterType)
    → codegen engine/builder  (B: LocalValue/oracle native)
    → codegen builtins        (C: native matches + typed registry boundary)
    → memory/collections/backends (D: native matches; residue census)
    → .nir JSON emit renders name() (bytes unchanged)     [A]
```

| Sub-plan | Delivers | Effort | Depends on |
|---|---|---|---|
| **A** (this) | NIR fields flipped; boundary clones; JSON emit renders; consumers shimmed | large | — |
| **B** | Engine native: `static_nir_value_type` → `Option<ParameterType>`, `LocalValue.type_`, `FieldTypes`, engine's 123 reads + its locals-map threading | large | A |
| **C** | Builtins native: the 298 builtins reads + 16 scalar-compare files; typed registry call boundary (no parse at `resolve_call*` from codegen) | large | B |
| **D** | memory (30) + collection (68) + cleanup/error (14) + backends (16) native; residual-shim census; perf proof vs baseline; doc sync | large | C |

**Byte-identity is the correctness gate for every phase** (provably-neutral
representation swap — the same class as plan-102). A NEW `.ncode`/`.nir` diff
vs the captured baseline is a bug to root-cause (objdump/diff ONE fixture) and
fix, never a design verdict.

**Where correctness risk concentrates:** C (builtins — 298 reads across many
per-function native lowerings; widest sweep) and B (the builder's local-type
threading feeds regalloc/layout decisions). Scheduled after A proves the model.

**Where design uncertainty concentrates:** almost nowhere — the design is the
proven plan-102-B pattern one layer down. The one genuinely new consideration is
**performance during the transition**: unlike the IR (low-frequency consumers),
codegen reads type fields in its hot lowering loops. A `.name()` shim on a
scalar is `Cow::Borrowed` (free); on a container type it formats a fresh
`String`. So Phase A can transiently regress lowering wall-clock until B–D
convert the hot readers native. That is why the Prerequisites capture a
`bench-lowering.sh` baseline and D's acceptance includes "not slower than
baseline". A transient mid-feature slowdown is acceptable; a final one is not.

### Rejected alternatives

- **Add type annotations to `NirValue::Call`/`CallResult` while flipping.**
  Rejected: changes the golden-checked `.nir` bytes and codegen's re-derivation
  oracle in the same step — un-reviewable; and the oracle exists (B types it).
- **Convert codegen consumers in the same sub-plan as the flip.** Rejected:
  plan-102-B proved the flip-then-sweep staging lands cleanly; one atomic step
  across 192k lines of codegen is un-reviewable.
- **Intern full type spellings as `Symbol` on NIR nodes instead of
  `ParameterType`.** Rejected: keeps every structural test a string operation
  (the point of the feature is native matches); `ParameterType` leaves are
  already interned `Symbol`s.
- **Skip the flip; convert codegen compares against strings in place.**
  Rejected: leaves the parse/format churn at every seam and the boundary
  rendering; "typed below the AST" stays false at the widest layer.

## 4. Detailed design — the flip

- `nir/mod.rs`: the 25 fields become `ParameterType` (import
  `crate::types::ParameterType`). Doc comments updated where they say "the full
  `Set OF T` string" → "the `Set OF T` type".
- `nir/lower.rs`: delete the 19 renders — `binding.type_.name().into_owned()` →
  `binding.type_.clone()`, etc. `merge_packages`' identity-prefixing of
  imported symbols does not touch type fields (verify while editing; it renames
  `target`/symbol strings only).
- `nir/constfold.rs`: `type_: "String".to_string()` → `ParameterType::String`
  (×3).
- `nir/json.rs`: every type/returns emit renders `&field.name()` — the same
  seam pattern as `ir/json.rs` post-plan-102-B. Bytes unchanged.
- `nir/visit.rs`: read-only walker; no type reads expected (verify by compile).
- Consumers (codegen 533 reads + target 16): compile via `.name()` at read
  sites — `.clone()` → `.name().into_owned()`, `&x.type_` to a `&str` slot →
  `&x.type_.name()`, `== "Integer"` → `.name().as_ref() == "Integer"` (B–D
  replace these with native matches; A only makes the tree build). Scalar-const
  hot paths (`NirValue::Const { type_, .. } if type_ == "Integer"`) may take the
  native `matches!(type_, ParameterType::Integer)` immediately where the edit is
  local — cheaper than a shim and reduces the transient perf risk.

## Compatibility / Format Impact

Nothing externally observable changes: `.nir` JSON bytes identical (goldens
prove it), `.ncode`/`.ncodesum` identical, no API/CLI surface. In-memory NIR is
internal to the compiler.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — capture the baselines

One line: record the gate diff set (expected empty at plan-writing time) and the
lowering perf probes, so later phases can prove "no NEW diff" and "not slower".

- [x] `rustup run 1.96.0 cargo build --release --bin mfb`; run
      `scripts/artifact-gate.sh target/release/mfb all`; save the sorted
      DIFF/FAILED set to `planning/plan-104-baseline-diffs.txt`. (0 diffs.)
- [x] Run `scripts/bench-lowering.sh target/release/mfb`; save the three probe
      times to `planning/plan-104-bench-baseline.txt`. (Script takes no exe
      arg — it builds debug+release itself; run as `bash scripts/bench-lowering.sh`.)
- [x] `rustup run 1.96.0 cargo test --no-fail-fast` green (run on a pristine
      detached worktree at main tip `cc93d4d67`, exit 0 / 0 FAILED — the
      in-worktree run was invalidated by Phase-2 edits landing mid-compile).

Acceptance: both baseline files exist; suite green.
Commit: —

### Phase 2 — flip the 25 fields + boundary + JSON emit

One line: the data-model flip with the boundary simplification, `.nir` bytes
unchanged.

- [ ] Flip the 25 type fields in `src/target/shared/nir/mod.rs` to
      `ParameterType`.
- [ ] `src/target/shared/nir/lower.rs`: replace the 19 `.name().into_owned()`
      renders with clones of the IR's typed fields.
- [ ] `src/target/shared/nir/constfold.rs`: the 3 folded Consts construct
      `ParameterType::String`.
- [ ] `src/target/shared/nir/json.rs`: render `name()` at each type/returns
      emit point.
- [ ] Consumer shim sweep (compile-driven): `.name()` at read sites across
      `src/codegen/` (533 reads) and `src/target/` (16 reads); take the native
      `matches!` form for local scalar-compare edits where trivial.
- [ ] Tests: existing suite compiles; fix test fixtures constructing NIR nodes
      with string type fields (`ParameterType::parse("…")` per the plan-102-B
      fixture pattern; assertions compare via `.name()`).

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` shows **no
NEW diff** vs `planning/plan-104-baseline-diffs.txt` (in particular every
`*.nir` golden byte-identical); `rg -c 'type_: String' src/target/shared/nir/mod.rs`
→ 0; **no backward conversion on the compile path** — the gate cannot see a
byte-exact render-back, so check it explicitly:
`rg -n 'name\(\).*ParameterType::parse|ParameterType::parse\(.*\.name\(\)' src/target/shared/nir/ src/codegen/`
finds no IR/NIR round-trip render (a shim reads `.name()`, it never re-parses),
and any deliberately-kept render is named in Corrections, not left silent
(plan-102's post-archive lesson: `6db8e040b`).
Commit: —

## Validation Plan

- Tests: existing NIR/codegen unit + integration suite; NIR-constructing test
  fixtures updated (assertions byte-preserved).
- Coverage check: every compiled fixture flows through `nir::lower` + the JSON
  dump + codegen — the full gate corpus IS the coverage.
- Runtime proof: `artifact-gate all` byte-identical vs baseline (1,718 goldens,
  incl. the per-target `*.nir` dumps).
- Doc sync: none expected for A (the spec documents the serialized JSON, which
  is unchanged); `.ai` doc updates land in D once the feature's final shape
  exists.
- Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast`;
  `scripts/artifact-gate.sh target/release/mfb all` (no NEW diff);
  `scripts/test-accept.sh target/release/mfb /tmp/accept-out` (no NEW mismatch
  beyond the 2 documented pre-existing);
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Shim form during A: `.name()` everywhere vs native `matches!` where the
  edit is local.** Recommend native for scalar compares touched anyway (free,
  reduces transient perf risk), shims for everything else. (§4)
- **`NirEntryPoint.returns`: `ParameterType` vs keep `String`.** It is consumed
  once (entry glue). Recommend flipping for uniformity — the field is in the 25.
  (§4)

## Corrections

- **The claim "the IR is already typed" is false for the op-level fields**:
  `IrOp::{Bind,For,ForEach}.type_` were still `String` (`src/ir/op.rs:8,78,93`;
  plan-102-B typed the decl-level fields and `IrValue`, not `IrOp`), and
  `ir/lower.rs` renders `.name().into_owned()` into them (e.g. `lower.rs:558`).
  That is why the original `nir/lower.rs` used plain `.clone()` (no `.name()`
  render) for those three arms — the 19-render census was correct but the
  inference "clone = already typed" was not. Parsing at the NIR boundary would
  have created a NEW name→parse backward seam (the exact plan-102 post-archive
  defect class), so A's scope was extended: the three `IrOp` type fields flipped
  to `ParameterType` too. Ripple handled: `ir/lower.rs` constructions parse
  their (string-currency, plan-106-A-scoped) computed types at the IR mint
  point; `ir/json.rs` renders `name()` at emit; `ir/binary.rs` decode parses the
  wire string / encode renders (same pattern as the plan-102 fields);
  `ir/verify/*`, `binary_repr/writer.rs`, `runtime/usage.rs` read via `.name()`
  shims (retyped by plan-106-B). No plan-105/106 letter owned this flip
  (`rg 'IrOp' planning/plan-105* planning/plan-106*` → no hits), so it belongs
  here rather than deferred.
- **`bench-lowering.sh` takes no binary argument** (the Prerequisites row's
  `scripts/bench-lowering.sh target/release/mfb` spelling): the script builds
  both debug and release compilers itself and ignores argv. Run bare.
- **`lower_numeric_for` retyped to `&ParameterType`** (not a `.name()` shim):
  its caller held the typed NIR `For.type_` and the callee re-constructed a
  `NirValue::Const` from the string — a render→parse round-trip split across
  the call boundary that the acceptance grep cannot see. The signature change
  removes it; the callee's `LocalValue` store renders `name()` (B converts).
- **Deliberately-kept forward parses on the compile path** (not round-trips,
  named per the acceptance): `ir/lower.rs` `IrOp::{Bind,For,ForEach}`
  construction sites parse ir/lower's internal string currency (retyped by
  plan-106-A); `ir/binary.rs` decode parses wire strings (a genuine string
  boundary). `codegen/` itself gained zero production parses.

## Summary

The mechanical foundation: a 25-field flip whose boundary *deletes* work (the
IR is already typed), gated by the strongest available check (0-diff
byte-identity across 1,718 goldens including the `.nir` dumps). The engineering
risk lives downstream in B–D's consumer sweeps; the one novel concern —
transient allocation cost from shims in codegen's hot loops — is bounded by the
captured perf baseline and D's final perf acceptance.
