# plan-78-A: Typed `Operand` value + faithful render (storage unchanged)

Last updated: 2026-08-02
Overall Effort: x-large (1d–3d) — the whole plan-78 feature (A + B + C)
Effort: medium (1h–2h)
Depends on: nothing

Introduce a typed `Operand` enum that will eventually replace the `String`
operand value in `CodeInstruction.fields`, and prove — before touching the
stored representation — that it can render back to the *exact* current operand
strings for every kind the codegen emits. This sub-plan lands the type, routes
the canonical `.field(...)` constructor through it, and adds a round-trip proof,
while **storage stays `String`** so the change is a byte-identical no-op.

The single behavioral outcome: after A, `CodeInstruction::field` accepts an
`impl Into<Operand>` and the stored bytes/goldens are unchanged
(`artifact-gate … all` green with zero diffs); a round-trip test proves
`Operand::parse(s).render() == s` for a corpus of every operand string the
compiler emits.

References:

- `.ai/compiler.md` — codegen test discipline, register lifetimes.
- `src/target/shared/code/code_impl.rs` — the `new`/`field`/`get`/`validate`
  builder and the `ToCodeJson` dump formatter.
- `src/target/shared/code/regalloc/mod.rs` — vreg sentinels (`%v`/`%f`) and
  `parse_vreg`/`vreg_name`.
- `src/arch/encode_operand.rs` — the encoder-side `field`/`immediate`/`shift`
  operand decoders (the parse rules `Operand` must mirror).

## Prerequisites

Stated once here for the whole plan-78 feature; sub-plans B and C point back to
this table. This plan stands alone and is **not** braided with the two CI-side
fixes from the acceptance-hang investigation (watchdog the `mfb test` path in
`scripts/test-accept.sh`; run acceptance against a release binary) — those are
independent and are neither prerequisites nor scope.

| Must be true | Command | Status |
|---|---|---|
| Repo builds clean at HEAD | `cargo build --bin mfb` → exit 0 | MET |
| Baseline goldens green (byte-identity oracle works) | `bash scripts/artifact-gate.sh target/debug/mfb all` → pass | UNVERIFIED — run first |
| Codegen tests green | `cargo test --bin mfb` → ok | UNVERIFIED — run first |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run and update before continuing and before stopping.

## 1. Goal

- A `pub(crate) enum Operand` exists with typed arms for the operand kinds the
  hot path touches — physical register (class + index), virtual register (class
  + id), and integer immediate — plus a `Raw(String)` arm for the long tail
  (labels, symbols, type names, stack-offset sentinels, booleans).
- `Operand::render(&self) -> String` reproduces the exact string the codegen
  emits today for every kind (proven by a round-trip test over a real corpus).
- `CodeInstruction::field` accepts `impl Into<Operand>`; **storage stays
  `Vec<(&'static str, String)>`** (it stores `operand.render()`), so this
  sub-plan changes no emitted byte and no golden.

### Non-goals (explicit constraints)

- **No representation flip.** `CodeInstruction.fields` stays `String`-valued in A
  (that is B). No consumer reads typed operands yet.
- **No byte change.** `artifact-gate … all` must be diff-free after A.
- **`MirInstruction` untouched** (mir.rs:28) — its interning, if ever needed, is
  out of scope for the whole plan-78 (selection is not on the hot path per the
  profile).
- **No `-regalloc bump` behavior change.**

## 2. Current State

`CodeInstruction { op: CodeOp, fields: Vec<(&'static str, String)> }`
(`types.rs:38`). `op` is already the typed `CodeOp` enum (`arch/ops.rs:2`); only
the operand *values* are stringly-typed. Every operand is one of: a virtual
register sentinel `%v<n>`/`%f<n>` (`regalloc/mod.rs:32,37,40,45`); a physical
register name (`x0`/`w0`/`d3`/`xmm*`, `sp` special-cased at `regalloc/mod.rs:194`);
a decimal integer immediate; a boolean `true`/`false`; a label; a symbol
(`_mfb_*`, function/data); a type name; or a stack-offset sentinel
(`incoming_args`/`outgoing_args`, `abi.rs:34,40`). Consumers disambiguate by
field-name role + prefix sniff + numeric parse — there is no tag.

The construction funnel is `CodeInstruction::new(op).field(name, value)`
(`code_impl.rs:4,11`), with ~90 typed constructors above it in `abi.rs`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `CodeInstruction::new` call sites (incl. tests) | 381 | `grep -rn "CodeInstruction::new" src --include=*.rs \| wc -l` |
| Distinct `.field("…")` names | 29 | `grep -rhoE '\.field\("[a-z_0-9]+"' src --include=*.rs \| sort -u \| wc -l` |
| Read-only field names (`vop`,`cond`) | 2 | `grep -rhoE '\.get\("[a-z_0-9]+"\)' src/target src/arch \| sort -u` |
| Inline `CodeInstruction {…}` literals (bypass builder, non-test) | 9 | `grep -rn "CodeInstruction {" src --include=*.rs \| grep -v "\-> CodeInstruction {"` (arch/*/select.rs, linear_scan.rs:388) |
| Files touching `CodeInstruction.fields` (codegen tree) | ~22 | `grep -rl "\.fields" src/target src/arch --include=*.rs \| wc -l` → 27, minus 5 verified type/variant false positives |

### Verified properties

- **Byte-identity is decided purely by the rendered value strings.** The `.ncode`
  and `.mir` dumps iterate `fields` and print each value verbatim via
  `json_string` (`code_impl.rs:258-263`, `mir.rs:748-753`); the encoders parse
  the value strings (`encode_operand.rs:15,31,43`). So if `render()` reproduces
  the current string exactly, dumps and bytes are unchanged — this is the pivot
  the whole plan rests on, and A's round-trip test is what proves it.
- **The operand-kind list above is complete.** Enumerated from the encoder
  decoders (`encode_operand.rs`, `aarch64/encode/operand.rs:30`), the vreg
  sentinels (`regalloc/mod.rs`), and the analysis prefix sniffs
  (`analysis.rs:184-220,304`). Any kind A's `Operand::parse` fails to classify
  falls to `Raw`, so the corpus test cannot silently miss one — a `Raw` that
  should have been typed only costs perf later, never correctness.

## 3. Design Overview

`Operand` is introduced as an *additive* type. The pragmatic arm set lifts only
the hot kinds to typed form (registers, immediates) and keeps everything else as
`Raw(String)` — this bounds B/C's churn while capturing 100% of the measured
`str::eq`/hash cost, which is entirely register/operand parsing in the analysis.

Because storage stays `String` in A (the builder calls `operand.render()` before
pushing), A is a pure no-op refactor whose only risk is `render()` fidelity —
falsified cheaply and first by the round-trip corpus test. The blast-radius work
(flipping storage, migrating consumers) is deferred to B/C.

Rejected: interning every operand string through a global `u32` interner + side
table. It leaves consumers resolving id→string and doesn't give the analysis
typed register ids directly; the typed enum is clearer and faster.

## 4. Detailed Design

```
pub(crate) enum Operand {
    Phys { class: RegClass, index: u32 },   // renders to the ISA name (x0/d3/…)
    VReg { class: RegClass, id: u32 },       // renders to %v<n> / %f<n>
    Imm(i64),                                // renders to decimal
    Raw(String),                             // labels, symbols, types, sentinels, bool
}
```

- `render()` mirrors the exact current spellings: `VReg` via `vreg_name`/
  `fp_vreg_name` (`regalloc/mod.rs:40,50`); `Phys` via the ISA name tables the
  encoders already own; `Imm` via decimal `to_string`; `Raw` verbatim.
- `parse(&str) -> Operand` mirrors the encoder/analysis sniff order (vreg prefix
  → physical name → decimal → else `Raw`). Used only by the corpus test in A;
  producers pass typed operands directly once they migrate (B).
- `From<&str>`/`From<String>` produce `Raw` (so unmigrated call sites compile
  unchanged); typed constructors (`Operand::phys`, `::vreg`, `::imm`) are used
  where the kind is known.
- `CodeInstruction::field(name, v: impl Into<Operand>)` stores `v.into().render()`
  (String) in A.

## Compatibility / Format Impact

None. Storage, dumps, encoders, goldens, and `-regalloc bump` are unchanged after
A. The only new surface is the `Operand` type and the `field` signature accepting
`impl Into<Operand>` (all existing `&str`/`String` args still work via `From`).

## Phases

> **NOTE — tick boxes and fill `Commit:` in the same commit as the work.**

### Phase 1 — Benchmark harness + baseline

Repeatable measurement so B/C have before/after numbers and the perf goal is
checkable. Safe alone (tooling only).

- [ ] Add `scripts/bench-lowering.sh` building three fixed probes — trivial
      baseline, one `regex::match("a","a")` const, full `tests/acceptance`
      compile — printing wall-clock each. Deterministic, no network.
- [ ] Record current baselines (debug + release) in `planning/plan-78-baseline.txt`:
      one-regex 31 s/6 s, full acceptance 4 m21 s/51 s, baseline 0.05 s.
- [ ] Capture the inlined regex function's instruction + vreg count (debug
      `eprintln` behind an env var in `function_lowering.rs:lower_function`, or
      from `-ncode`) so B/C can report "N instructions colored in T ms".

Acceptance: `bash scripts/bench-lowering.sh` prints stable timings on two runs
(±20%); baseline file records the starting numbers incl. the function size.
Commit: —

### Phase 2 — `Operand` type + render + `field` funnel

Introduce the type; route the canonical constructor through it; storage stays
String.

- [ ] Add `Operand` (`code_impl.rs` or a new `operand.rs` in the same module) per
      §4, with `render`, `parse`, `From<&str>`/`From<String>`, `phys`/`vreg`/`imm`.
- [ ] Change `CodeInstruction::field` (`code_impl.rs:11`) to
      `field(name, v: impl Into<Operand>)` storing `v.into().render()`.
- [ ] Tests: `regalloc/tests.rs` (or a new `operand` test module) — round-trip
      `Operand::parse(s).render() == s` over a corpus harvested from real output:
      dump `-ncode` for a handful of fixtures spanning registers/immediates/
      labels/symbols/types, extract every distinct value string, assert round-trip.
- [ ] Run `artifact-gate.sh target/debug/mfb all` — zero diffs.

Acceptance: `artifact-gate … all` byte-identical; the corpus round-trip test
passes with the corpus containing ≥1 of each operand kind in §2 (assert the kind
set is covered, so a missing kind fails the test rather than passing vacuously).
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` (codegen lives in the bin target) incl. the new
  round-trip corpus test.
- Byte-identity: `artifact-gate.sh … all` diff-free (the guardrail).
- Coverage: the new `Operand` code is in the coverage denominator
  (`scripts/coverage-check.sh`, per-file 95%).
- Doc sync: none (no external contract change).
- Acceptance: `cargo test --workspace` + `artifact-gate … all` green.

## Open Decisions

- **`Operand` module location** — extend `code_impl.rs` vs. a new `operand.rs`.
  Recommend a new `operand.rs` under `src/target/shared/code/` for clarity. (§4)
- **`Phys` render source** — reuse the encoders' existing ISA name tables vs. a
  new table. Recommend reuse (single source of truth for name↔index). (§4)

## Corrections

<Filled in during execution.>

## Summary

A is the cheap, byte-identical foundation: it lands the typed `Operand` and — the
one thing that must be true for B/C to be safe — *proves* render fidelity before
any representation changes. All blast-radius work is deferred to B (flip storage)
and C (migrate the hot consumer + fix the `colored_mask_at` quadratic).
