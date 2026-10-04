# bug-798: Inline TRAP changes Float observation timing

Last updated: 2026-10-03
Effort: medium (estimate; lowering design requires investigation)
Severity: HIGH
Class: Correctness
Status: Fixed
Regression Test: `tests/runtime/rt_inline_trap_float_observation.rs` (parity matrix at -O0..-O3); `src/ir/variant_corpus_tests.rs` (capture-only `Checked` round-trip)

Attaching inline TRAP changes the value of a successful Float expression into a
failure. Correct behavior preserves the original expression's observation
boundaries: transient nonfinite intermediates may become finite before observation.

References:

- `src/docs/spec/language/04_types.md`: Float finiteness and mandatory FMA contract.
- `src/docs/spec/language/08_error-model.md`: inline expression recovery.
- `../planning/plan-165-C-conditional-inline-trap.md`: preserve conditional evaluation.

## Failing reproduction

Create an executable project with this manifest and `src/main.mfb`:

```json
{"name":"float_review","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
```

```basic
IMPORT io
FUNC identity(x AS Float) AS Float
  RETURN x
END FUNC
FUNC main() AS Integer
  MUT huge AS Float = 1e200
  LET plain = identity(1.0 / (huge * huge))
  LET trapped = identity(1.0 / (huge * huge)) TRAP(e)
    io::print("unexpected trap")
    RECOVER -1.0
  END TRAP
  io::print(toString(plain) & "," & toString(trapped))
  RETURN 0
END FUNC
```

Commands run: `target/release/mfb build /tmp/mfb-error-review-float`, then
`/tmp/mfb-error-review-float/build/float_review.out`.
Observed on macOS AArch64: build exit 0, run exit 0, stdout:

```text
unexpected trap
0.00,-1.00
```

Expected: `0.00,0.00`, without the handler message. Both forms pass the same
complete arithmetic expression as one argument. The untrapped form is the control.
The spec explicitly permits `1.0 / (1e200 * 1e200)` to become finite before
observation (`rg -n 'intermediate|observation' src/docs/spec/language/04_types.md`).

The reproduction used the existing release executable before implementation edits.
A detached HEAD checkout was created at `/tmp/mfb-error-model-work`; a clean
rebuild and repetition there remain required before fixing to establish binary
provenance independently of the existing executable. No compiler fix is retained.

## Root cause

`src/ir/lower.rs:scan_trap_operands`, `trap_hoist_kind`, and
`rewrite_trap_call` lift individual raising operators into `Checked` results.
`src/codegen/engine/value/builder_values.rs:lower_checked_value` calls
`observe_float` on each captured result. A compiler-created intermediate becomes
an observation boundary even when source has no such boundary.

Evidence commands: `sed -n '2375,2505p' src/ir/lower.rs`,
`sed -n '2645,2755p' src/ir/lower.rs`, and
`sed -n '3385,3440p' src/codegen/engine/value/builder_values.rs`.

## Goal and non-goals

Trapped and untrapped evaluation must agree on successful values, evaluation order,
Float domain checks, actual observation failures, and mandatory contraction.
Do not widen operator-only TRAP eligibility, defer integer failure checks, allow
observable nonfinite Float values, or weaken tests to match premature checking.

## Blast-radius audit

`rg -n 'Checked \{' src` and `rg -n 'observe_float' src/codegen` identify:

- IR trap extraction and native `lower_checked_value`: directly implicated; fix.
- Float arithmetic inside arguments, comparisons and short-circuit branches:
  shared extraction, require targeted reproduction and parity coverage.
- Mandatory Float FMA contraction: a suspected sibling consequence of splitting
  expression trees, not yet reproduced; test before claiming it is broken.
- Integer/Byte/Fixed/Money checked operators: preserve eager failure semantics.
- IR/NIR visitors, binary encoding and decoded-package verification: affected
  only if representing observation separately requires a Checked shape change.
- `RECOVER` and `RETURN` operands: retain their own source observation boundaries;
  their failures must follow handler-body routing, not re-enter this inline trap.

## Fix design

Treat error capture and Float observation as distinct operations. Preserve whole
Float arithmetic trees where possible; capture at the original argument, storage,
or return boundary, not at each synthetic binding. If extraction is unavoidable,
represent whether a capture is an actual observation explicitly and preserve
mandatory FMA grouping. A capture-only intermediate must still deliver Float domain
errors at their original evaluation position. Do not simply stop checking Float:
that would miss real boundary overflows and domain failures.

Before editing, settle the precise representation after tracing calls interleaved
with arithmetic. Reordering a domain failure past a side-effecting call is invalid.
If changing IR encoding, include package compatibility and decoded-package tests;
do not smuggle a semantic flag into generated temporary names.

## Phases

### Phase 1 — Reproduction and design

- [x] Rebuild at detached HEAD and repeat the minimal reproduction.
- [x] Add a RED runtime parity test for the reproduced case.
- [x] Probe comparisons, mandatory FMA, Float MOD/power domain errors, calls between
      operators, and actual argument/assignment overflow boundaries.
- [x] Choose and document the capture/observation representation and format impact.

Acceptance: exact wrong output above is reproduced by a freshly built HEAD
compiler; tests distinguish original boundaries from synthetic intermediates.
Commit: —

### Phase 2 — Preserve observation semantics

- [x] Implement the selected representation without changing operator-only eligibility.
- [x] Add valid/error fixtures under `tests/rt-behavior/trap` and `tests/rt-error/trap`.
- [x] Update error-model, Float, IR and package-format specs wherever affected.

Acceptance: ordinary/trapped parity and true-error tests pass across optimization
levels, with side-effect ordering asserted. Run the dedicated runtime test target
(estimate: minutes after build).
Commit: —

### Phase 3 — Regression gates

- [x] Run `cargo test --no-fail-fast` and the full acceptance gate before any
      existing-golden correction; investigate each unexpected diff.
- [x] Correct only disproved expectations with history/contract evidence, then
      rerun affected checks and required final gates.

Acceptance: full test and artifact/acceptance gates pass; runtime output proves
boundary preservation. Full gates may take hours (estimate).
Commit: —

## Resolution (2026-10-03)

**Representation.** `IrValue::Checked` / `NirValue::Checked` carry
`observe: bool`. It is TRUE where the capture meets an observation boundary.
It is FALSE for a `Float` `MOD`/`^` lifted from inside an operator tree, which
is captured for its domain error only. The flag is explicit in the node, not
in temp names.

- **Encoding:** tag 22 = observing (unchanged), new tag 23 = capture-only. Every
  existing package re-encodes byte-identically.
- **JSON:** prints `"observe": false` only for the new form, so no existing
  `.ir` golden moved (the `*trap*`, `*float*` and `arithmetic*` acceptance
  fixtures all match).

**Desugar.** `trap_hoist_kind` / the scan and rewrite walks take an
`under_operator` context. Operator operands (arithmetic or comparison) are not
boundaries; call arguments, constructor fields, list and map elements and the
bound root are.

- A `Float` `+ - * / DIV` / negation is lifted only at a boundary
  (`float_operator_raises_only_when_observed`), so each `Float` tree is captured
  once, where the untrapped form observes it. That also keeps `a * b ± c` whole
  for FMA contraction.
- `MOD`/`^` stay lifted at their own position, so domain errors keep source order
  relative to calls. Their capture observes only at a boundary.
- Non-`Float` operators are unchanged.

**Proof.** The matrix in `/tmp/b798m`, now the regression test, shows
fix/main:

| row | fix | main |
|---|---|---|
| transient recovers | 0.00 ✓ | trapped 77050015 ✗ |
| boundary overflow | trapped 77050015 ✓ | ✓ |
| comparison operand | TRUE ✓ | trapped ✗ |
| FMA `big*2-big` | finite ✓ | trapped ✗ |
| `MOD` domain before a later call | 77050012, side effect not run ✓ | ✓ |
| `^` overflow inside a tree | 0.00 ✓ | trapped ✗ |
| `^` overflow at boundary / `^` domain | trapped ✓ | ✓ |

The fix gives identical output at -O0/1/2/3. The `ir::` unit tests pass (920),
and so do the `rt_inline_trap_*` / `rt_callback_failure_reaches_trap` runtime
tests. Specs: `mfb spec language error-model` rule 11, `architecture ir` /
`native-ir` / `ir-json-artifact`, and `package ir-section`. This unblocks
plan-165-C/D, which listed bug-798 as a prerequisite.
