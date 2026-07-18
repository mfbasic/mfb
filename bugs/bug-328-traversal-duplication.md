# bug-328: NIR/IR tree traversal is hand-written ~60 times — one constant-folding helper set is byte-duplicated, and the copies have already diverged on `Match` guards

Last updated: 2026-07-18
Effort: x-large (1d–3d)
Severity: LOW
Class: Other (cleanup) / Dead-code

Status: Open
Regression Test: existing `src/target/shared/validate.rs` + `src/ir/verify` unit
suites; new `nir/visit.rs` and `ir/value.rs` visitor unit tests added in Phase 2.

The target layer and the IR layer each re-implement the same recursive tree walk
dozens of times, once per analysis. Two concrete consequences are measurable
today. First, five `native_*` constant-folding helpers exist as **two
byte-identical 117-line copies** whose only textual difference is five
`pub(super)` visibility keywords — both copies are live and reached from
different callers. Second, because every walk is hand-written, the copies have
**already drifted**: the capability-validation walk in
`src/target/shared/validate.rs` does not traverse `NirMatchCase::guard`, while
its two twins in `src/target/shared/plan/symbols.rs` do — and each of those twins
carries a comment claiming it mirrors a guard traversal that `validate` in fact
does not perform.

The single correct behavior a fix produces: one traversal seam per IR level — a
`NirVisitor` trait in `src/target/shared/nir/visit.rs` and a shared `visit_value`
next to `IrValue` — so that every analysis inherits the same, complete recursion,
and adding a `NirValue`/`IrValue` variant is a single edit rather than a dozen.
Unifying the walkers necessarily **repairs** the `Match`-guard divergence; see
Fix Design — that is an intended behavior delta, not a byte-identical refactor.

References:

- `src/docs/spec/architecture/13_native-ir.md` — the NIR data model the walkers
  traverse.
- bug-118 — fixed the identical guard omission in `plan/symbols.rs`; cited by the
  two surviving guard traversals at `src/target/shared/plan/symbols.rs:339` and
  `:568`.
- `bugs/bug-300-docs-deadcode-low-cluster.md` E14 — the *same* guard omission in
  `plan/function_builder.rs`, filed separately; subsumed by this fix.
- Found during the cleanup review (Agent 09 items 1 and 4; Agent 12 item 3).

## Current State

All figures below were measured against the working tree, not estimated.

### 1. The constfold helpers are byte-identical apart from five keywords

```
$ sed -n '457,573p' src/target/shared/validate.rs      > /tmp/a.txt
$ sed -n '709,825p' src/target/shared/plan/symbols.rs  > /tmp/b.txt
$ wc -l /tmp/a.txt /tmp/b.txt
     117 /tmp/a.txt
     117 /tmp/b.txt
$ diff /tmp/a.txt /tmp/b.txt
1c1
< fn native_constant_value(
---
> pub(super) fn native_constant_value(
30c30
< fn native_static_string_value(
---
> pub(super) fn native_static_string_value(
60c60
< fn native_strings_package_static_string_value(
---
> pub(super) fn native_strings_package_static_string_value(
79c79
< fn native_static_graphemes_value(
---
> pub(super) fn native_static_graphemes_value(
91c91
< fn native_primitive_text(
---
> pub(super) fn native_primitive_text(
```

The five differing lines are the *entire* diff: 117 lines each, five `pub(super)`
tokens. Verbatim, the first of the five (identical in both files apart from the
signature line — `src/target/shared/validate.rs:457-484`,
`src/target/shared/plan/symbols.rs:709-736`):

```rust
fn native_constant_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Global { .. } => None,
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Binary { op, .. } if op == "&" => native_static_string_value(value, constants)
            .map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            }),
        _ => None,
    }
}
```

Both copies are live — neither is dead code:

- `validate.rs` copy: `validate.rs:274`, `:300`, `:390`, `:391`.
- `symbols.rs` copy: `symbols.rs:504`, `:535`, and
  `plan/function_builder.rs:30`, `:56`, `:251`, `:258`.

### 2. Traversal count (measured, not estimated)

Counted by attributing every `NirOp::Match {` match arm and every self-recursive
`NirValue::{Binary,RuntimeCall}` match to its enclosing `fn`:

| Walk kind | Count | Files |
| --- | --- | --- |
| Functions containing a `NirOp::Match` arm (op walkers) | **29** | `validate.rs` (3), `plan/symbols.rs` (4), `plan/function_builder.rs` (1), `code/module_analysis.rs` (8), `code/builder_control.rs` (3), `code/data_objects.rs` (3), `code/function_lowering.rs` (5), `nir/json.rs` (1), `nir/lower.rs` (1) |
| Self-recursive `NirValue` walkers | **30** | same modules plus `code/type_utils.rs`, `code/builder_value_semantics.rs`, `plan/mod.rs` |
| Self-recursive `IrValue` walkers, non-test | **13** | `ir/verify/mod.rs` (4), `ir/binary.rs`, `ir/json.rs`, `ir/lower.rs`, `ir/package.rs`, `binary_repr/writer.rs` (3), `target/shared/nir/lower.rs`, `target/shared/runtime/usage.rs` |

### 3. The copies have diverged on `Match` guards

`src/target/shared/validate.rs:333-343` — `collect_runtime_calls_from_ops_with_constants`,
the walk that feeds **capability validation**:

```rust
            NirOp::Match { value, cases } => {
                collect_runtime_calls_from_value(value, calls, constants);
                for case in cases {
                    let mut case_constants = constants.clone();
                    collect_runtime_calls_from_ops_with_constants(
                        &case.body,
                        calls,
                        &mut case_constants,
                    );
                }
            }
```

No `case.guard` traversal. Its two twins do have one, and both cite bug-118:

- `src/target/shared/plan/symbols.rs:336-341` (`collect_platform_imports_from_ops`)
  — *"A guard may call a builtin that needs a platform import … so the import is
  not missing at link (bug-118)."*
- `src/target/shared/plan/symbols.rs:566-571` (`collect_runtime_symbols_from_ops_with_constants`)
  — *"A runtime call used only in a `WHEN` guard must have its symbol emitted
  too, mirroring the guard traversal `validate_nir` performs (bug-118)."*

The second comment is **factually wrong today**: the capability collector at
`validate.rs:333` performs no such traversal. (A *different* pass,
`validate_ops` at `validate.rs:968`, does walk guards at `:1027` — which is
presumably how the claim survived review.) `plan/function_builder.rs` walks no
guard at all in any pass, which is bug-300-E14.

### 4. The IR side: three near-identical walkers, all exhaustive

`src/ir/verify/mod.rs` holds three depth-bounded `IrValue` walkers with the same
shape, the same `MAX_DEPTH` guard, and the same near-identical doc comment:

| Symbol | Line | Length | `IrValue::` arms | Wildcard arm? |
| --- | --- | --- | --- | --- |
| `collect_local_reads_value_depth` | `:4942` | 62 lines | 20 | none |
| `collect_closures_depth` | `:5056` | 66 lines | 20 | none |
| `walk_captures_depth` | `:5206` | 64 lines | 20 | none |

192 lines total. A fourth lives at `src/ir/package.rs:290`
(`rewrite_value_targets`, 77 lines, also no wildcard arm), and an op-level pair
sits at `src/ir/verify/mod.rs:4905` (`collect_local_reads_op`) and `:5122`
(`collect_closures_ops`).

Because none of the four has a `_ =>` arm, adding an `IrValue` variant is a
compile error at each site — the maintenance cost is enforced, not merely
implied.

## Root Cause

There is no traversal seam at either IR level. `NirOp`/`NirValue`
(`src/target/shared/nir/mod.rs:149`, `:241`) and `IrValue`
(`src/ir/value.rs:21`) are plain enums with no companion visitor, so every new
analysis is written by copying the nearest existing `match` and editing the arms
that matter. The copy is correct on the day it is made and drifts thereafter,
because nothing ties the copies together: a fix applied to one walker (bug-118)
must be manually replicated to every sibling, and bug-118 reached two of the four
`Match`-handling sites in the plan/validate cluster.

The constfold duplication is the degenerate case of the same cause: `validate.rs`
needed the folding predicate that `plan/symbols.rs` already had, but `symbols.rs`
is a `plan` submodule and its helpers are `pub(super)`, so the whole 117-line
block was copied across the module boundary rather than hoisted to a shared home.

## Goal

- Exactly one recursive traversal implementation per IR level: a `NirVisitor`
  trait with `walk_ops`/`walk_value` in `src/target/shared/nir/visit.rs`, and a
  shared `visit_value` alongside `IrValue` in `src/ir/value.rs`.
- Exactly one copy of the five `native_*` constant-folding helpers, in a new
  `src/target/shared/nir/constfold.rs`.
- Adding a `NirValue`, `NirOp`, or `IrValue` variant requires editing the visitor
  and only those analyses that genuinely care about the new variant.
- Every `NirOp::Match` traversal walks `case.guard`.

### Non-goals (must NOT change)

- **Emitted output does not change**, with the single exception noted below.
  `scripts/artifact-gate.sh` (execution-free artifact diff) must show no delta
  outside the intended `Match`-guard change, and `scripts/test-accept.sh` must be
  green.
- The `NirOp`/`NirValue`/`IrValue` data model itself — no variants added,
  removed, renamed, or re-shaped. This is a traversal refactor only.
- The `.mfp` / `.nplan` / `.nobj` wire and dump formats.
- The `MAX_DEPTH` recursion cap semantics on the IR side
  (`src/ir/verify/mod.rs`) — the shared `visit_value` must preserve the
  depth-bounded behavior, not silently unbound it. Removing the cap "because the
  visitor doesn't need it" is a forbidden shortcut: it reintroduces the
  stack-overflow class the cap exists to prevent.
- Do **not** paper over the `Match`-guard divergence by teaching the new visitor
  to skip guards in order to preserve byte-identical output. The divergence is a
  defect; the fix repairs it.

## Blast Radius

Every site found by the counting script above; verdicts:

**Fixed by this bug**

- `src/target/shared/validate.rs:457-573` (five `native_*` helpers) — deleted,
  callers at `:274`, `:300`, `:390`, `:391` repointed to `nir/constfold.rs`.
- `src/target/shared/plan/symbols.rs:709-825` (the twin) — moved to
  `nir/constfold.rs`; callers at `:504`, `:535` and
  `plan/function_builder.rs:30`, `:56`, `:251`, `:258` repointed.
- `src/target/shared/validate.rs:333-343` — gains the `case.guard` traversal
  (**the behavior delta**).
- `src/target/shared/plan/function_builder.rs:136` `NirOp::Match` arm — gains the
  guard traversal, closing bug-300-E14.
- The 29 `NirOp` walkers and 30 `NirValue` walkers listed in Current State §2 —
  converted to `NirVisitor` impls.
- `src/ir/verify/mod.rs:4942`, `:5056`, `:5206` and `src/ir/package.rs:290` —
  converted to `visit_value` callbacks.

**Latent, same hazard, out of scope**

- `src/binary_repr/writer.rs:323`, `:462`, `:662` — three self-recursive
  `IrValue` walkers with the same duplication hazard. Out of scope because
  `binary_repr` is a separate serialization crate-module with its own resource
  and import-collection concerns; folding it in widens an already x-large change.
  Convert in a follow-up once `visit_value` has proven itself.
- `src/ir/binary.rs:1168` (`encode_value`), `src/ir/json.rs:649` (`to_json`),
  `src/target/shared/nir/json.rs:822` — codec walkers. Out of scope: a codec must
  handle every variant explicitly and gains nothing from a default-recursing
  visitor; exhaustive matching is the correct tool there.
- `src/ir/lower.rs:2868` (`lower_expression_with_expected`, 791 lines) — a
  transformer, not a collector; out of scope for the same reason.
- The ~11 self-recursive `IrValue` walkers in `src/ir/tests.rs` — test-local
  one-off predicates; unaffected.

**Unaffected**

- `src/target/shared/code/builder_*.rs` dispatchers that match a single
  `NirValue` variant without recursing — they are not walks and inherit nothing
  from a visitor.

## Fix Design

**NIR side — `src/target/shared/nir/visit.rs` (new).**

A `NirVisitor` trait whose methods all have default bodies that recurse into
every child, so an implementer overrides only what it cares about and inherits
complete traversal for everything else:

```text
trait NirVisitor {
    fn visit_op(&mut self, op: &NirOp)        { walk_op(self, op) }
    fn visit_value(&mut self, v: &NirValue)   { walk_value(self, v) }
    fn visit_ops(&mut self, ops: &[NirOp])    { walk_ops(self, ops) }
}
fn walk_ops<V: NirVisitor + ?Sized>(v: &mut V, ops: &[NirOp]);
fn walk_op<V: NirVisitor + ?Sized>(v: &mut V, op: &NirOp);
fn walk_value<V: NirVisitor + ?Sized>(v: &mut V, value: &NirValue);
```

`walk_op`'s `NirOp::Match` arm walks `value`, then for each case walks
`case.guard` **and** `case.body` — the union of what the existing copies do.
`walk_ops`/`walk_value` are free functions so an override can call back into the
default recursion after doing its own work, which is what most of the 29 op
walkers need.

The scope-sensitive walkers are the risk concentration. Several analyses clone a
constants map per branch (`validate.rs:336`, `symbols.rs:573`, and the loop arms)
and several thread a mutable accumulator. The trait carries no map — the
implementer keeps its own state and overrides `visit_op` for the arms where
branch scoping matters, delegating the rest to `walk_op`. Do not try to
generalize the constants-map scoping into the trait; that would encode one
analysis's policy into the shared seam.

**NIR side — `src/target/shared/nir/constfold.rs` (new).**

Move one copy of the five `native_*` helpers verbatim, `pub(crate)`. Delete both
originals. `nir/` is the correct home: it is the module that owns `NirValue`, and
it is already a shared dependency of both `validate.rs` and `plan/`.

**IR side — `visit_value` in `src/ir/value.rs`.**

Placed next to the `IrValue` definition at `:21` so the enum and its traversal
are edited together. A single depth-bounded free function taking a callback:

```text
fn visit_value(value: &IrValue, depth: usize, f: &mut impl FnMut(&IrValue));
```

It preserves the existing `MAX_DEPTH` cutoff. The three `verify/mod.rs` walkers
and `package.rs:290` become callbacks. `package.rs`'s `rewrite_value_targets`
mutates, so it needs a `visit_value_mut` sibling — write both, or scope the IR
half to the three read-only `verify` walkers and leave `package.rs` for a
follow-up if the mutable variant proves awkward.

**The expected behavior delta — call this out in the commit message.**

Unifying the walkers gives `collect_runtime_calls_from_ops_with_constants` the
guard traversal it currently lacks. Consequence: **capability validation will
start seeing runtime calls that appear only inside a `WHEN … WHERE <expr>`
guard.** A program that today compiles because a backend-gated call hid in a
guard may, after this change, be correctly rejected with "native backend does not
implement runtime helper '<x>'". That is the intended, correct outcome — it is
the exact defect bug-118 fixed on the sibling passes — but it means this refactor
is **not** byte-identical and must not be validated as if it were. Expect
`plan/function_builder.rs` to additionally populate `PlannedFunction.calls` and
`string_literals` from guards, shifting descriptive `.nplan`/`.nobj` goldens for
any fixture with a guard containing a call or string literal (bug-300-E14's
documented, dump-only effect).

**Rejected alternatives.**

- *Derive macro over the enums.* Rejected: adds a proc-macro dependency and a
  build-time cost for ~60 call sites, and the scope-sensitive walkers would still
  need hand-written overrides.
- *One shared `collect_*` function per analysis kind, no trait.* Rejected: the
  analyses differ in accumulator type and branch scoping; a trait with default
  recursion expresses that; a fixed function signature does not.
- *Make `plan/symbols.rs`'s helpers `pub(crate)` and delete only the
  `validate.rs` copy.* Rejected as the whole fix — it resolves item 1 in ten
  minutes but leaves `plan` as the de-facto owner of a NIR-level concern and does
  nothing for the 59 walkers. Worth landing as Phase 2 on its own, then
  continuing.

## Phases

### Phase 1 — characterize the divergence (no behavior change)

- [ ] Add a `validate.rs` unit test with a `NirOp::Match` whose `case.guard`
      contains a gated runtime call; assert the call is **absent** from the
      collected set today. This test documents the divergence and will be
      inverted in Phase 4.
- [ ] Add a unit test asserting `plan/symbols.rs`'s two guard-walking collectors
      **do** see the same call — pinning the asymmetry.
- [ ] Re-run the walker census (Current State §2) and record any drift from the
      29/30/13 figures in this file.

Acceptance: the asymmetry is captured by two tests that pass against current
behavior; the census is current.
Commit: —

### Phase 2 — hoist the constant-folding helpers

- [ ] Create `src/target/shared/nir/constfold.rs` with one `pub(crate)` copy of
      the five `native_*` helpers; declare it in `nir/mod.rs`.
- [ ] Delete `validate.rs:457-573` and `plan/symbols.rs:709-825`; repoint all ten
      call sites.

Acceptance: `cargo build` clean; `scripts/artifact-gate.sh` shows **zero** delta
(this phase is a pure move).
Commit: —

### Phase 3 — introduce the seams, no callers converted

- [ ] Add `src/target/shared/nir/visit.rs` with `NirVisitor` + `walk_ops` /
      `walk_op` / `walk_value`; unit-test that the default impl reaches every
      `NirOp` and `NirValue` variant, **including `NirMatchCase::guard`**.
- [ ] Add depth-bounded `visit_value` (and `visit_value_mut` if `package.rs` is
      in scope) to `src/ir/value.rs`; unit-test the `MAX_DEPTH` cutoff.

Acceptance: new seams exist and are tested; no existing walker converted; artifact
gate shows zero delta.
Commit: —

### Phase 4 — convert the walkers

- [ ] Convert the four `Match`-handling sites in the validate/plan cluster first
      (`validate.rs:333`, `plan/symbols.rs:336`, `:566`,
      `plan/function_builder.rs:136`). Invert the Phase 1 test: the guard call is
      now collected. Close bug-300-E14.
- [ ] Convert the remaining `NirOp`/`NirValue` walkers in
      `code/module_analysis.rs`, `code/function_lowering.rs`,
      `code/data_objects.rs`, `code/builder_control.rs`, in that order — smallest
      accumulator first.
- [ ] Convert `ir/verify/mod.rs:4942`, `:5056`, `:5206` (and `ir/package.rs:290`
      if in scope) to `visit_value`.
- [ ] Write each blast-radius site's verdict back into this file.

Acceptance: Phase 1's inverted test passes; every converted walker's own tests
pass; the blast-radius list has a verdict per site.
Commit: —

### Phase 5 — regenerate expected outputs + full validation

- [ ] Run `scripts/artifact-gate.sh`. Diff must be empty **except** for fixtures
      with a call or string literal inside a `WHEN … WHERE` guard. Enumerate
      those fixtures in this file and confirm each delta is exactly the guard
      contents appearing in `.nplan`/`.nobj`.
- [ ] Regenerate the affected descriptive goldens; review line by line.
- [ ] Run `scripts/test-accept.sh` in full.
- [ ] Confirm no fixture newly fails capability validation; if one does, confirm
      by hand that the rejection is correct (a genuinely gated call in a guard)
      and record it here.

Acceptance: full suite green; every artifact delta traced to a guard traversal;
any new capability rejection justified in writing.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 guard-visibility tests (inverted in Phase 4); the
  Phase 3 visitor-completeness tests asserting every variant *and* `case.guard`
  is reached; the `MAX_DEPTH` cutoff test.
- Runtime proof: `scripts/test-accept.sh` full run — the compiled programs must
  behave identically, since the only semantic change is which programs are
  *rejected*, not what accepted programs do.
- Artifact proof: `scripts/artifact-gate.sh` with the guard-fixture exception
  enumerated and justified.
- Doc sync: fix the false comment at `src/target/shared/plan/symbols.rs:568-570`
  ("mirroring the guard traversal `validate_nir` performs") — after Phase 4 it
  becomes true, so it can stay; if Phase 4 is deferred, correct it immediately.
  Add a short traversal-seam note to
  `src/docs/spec/architecture/13_native-ir.md`.
- Full suite: `scripts/artifact-gate.sh` + `scripts/test-accept.sh`.

## Open Decisions

- Include `src/ir/package.rs:290` (`rewrite_value_targets`) in scope, requiring a
  `visit_value_mut` sibling? **Recommended: defer to a follow-up.** The read-only
  `visit_value` carries the three `verify` walkers; a mutable visitor is a
  distinct design problem and would stall the phase.
- Convert the three `src/binary_repr/writer.rs` walkers now or later?
  **Recommended: later** — separate module, separate concerns, and the change is
  already x-large.
- Land Phase 2 (constfold hoist) as an independent commit even if Phases 3-5
  stall? **Recommended: yes.** It is a pure move with a zero-delta artifact gate
  and removes 117 duplicated lines on its own.

## Summary

The engineering risk is concentrated in Phase 4 and Phase 5, not in the seam
design. Phase 2 (delete 117 duplicated lines) is a mechanical, zero-delta move
that can land immediately. The genuinely delicate part is that unifying the
walkers *repairs* a real divergence — capability validation will begin seeing
runtime calls hidden in `WHEN … WHERE` guards — so this refactor cannot be
validated by asserting byte-identical output. Every artifact delta must be traced
to a guard traversal and confirmed correct by hand. The data model, wire formats,
and `MAX_DEPTH` recursion caps are untouched.
