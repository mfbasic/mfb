# plan-82-A: Typed register operand representation + faithful rendering

Last updated: 2026-08-02
Overall Effort: huge (>3d) — the whole plan-82 feature (A + B + C + D)
Effort: medium (1h–2h)
Depends on: nothing (plan-78 A/B are already merged; this consumes the `Operand`
type they created)

Make native-register operands a **typed inline value** (`Operand::VReg` /
`Operand::Phys`, a `RegClass` + `u32`) instead of a heap-allocated
`Operand::Raw(Box<str>)`, end to end through the code builder, register
allocator, and instruction encoder. This sub-plan (A) does **not** flip any hot
path — it resolves the two design premises the whole feature rests on and lands
the typed `Phys` arm with **byte-identical rendering**, proven on the dumps.
B/C/D then flip the hot paths onto it.

The single behavioral outcome of plan-82 as a whole: the acceptance compile does
**dramatically fewer heap allocations** and the shipped compiler compiles
`tests/acceptance` far faster, while every emitted byte (machine code and every
`--ncode`/`--mir`/diagnostic dump) is **identical** to today.

## Why this plan exists (the deferred work)

plan-78 introduced `Operand` but **deliberately left every register as
`Raw(Box<str>)`** — the typed `VReg` arm is `#[allow(dead_code)]`, constructed
only by a test (`src/target/shared/code/operand.rs:55-69`). plan-78-C was
scheduled to construct the typed value on the allocator hot path and never did,
because it (a) judged the producer-site migration too large for one plan and
(b) profiled an unrepresentative single-function workload and wrongly concluded
micro-opts would hit the target. They did not (~4% net). **This plan does the
work plan-78 deferred.**

### Measured baseline (profiling spike, 2026-08-02, macOS aarch64)

Measured on `tests/acceptance` (16-file TESTING project, 4071 source lines),
cold compile via `mfb test tests/acceptance`:

| Metric | Debug `mfb` | Release `mfb` | How measured |
|---|---|---|---|
| Total wall | 284 s | 58 s | `time` around `mfb test` |
| Front-end (parse+resolve+verify) | 2.6 s | 1.6 s | `-v` phase lines |
| Runtime (execute tests) | 1.2 s | 0.4 s | total − compile |
| **codegen+link** | **280 s (98.7%)** | **56 s (97%)** | `-v` phase line |
| — code_emit substage | 187 s | 37.7 s | env-gated substage timer |
| — encode substage | ~60 s | 13.4 s | env-gated substage timer |
| **Heap allocations, whole compile** | — | **808,808,429** | counting `GlobalAlloc` |
| Total allocation churn | — | 37 GiB | counting `GlobalAlloc` |
| Allocator self-time share | 19% (81% mfb compute) | **74%** | `sample` top-of-stack sum |
| Allocs in code_emit substage | 594.6 M | 566.0 M | per-substage alloc counter |
| Allocs in encode substage | — | 215.1 M | per-substage alloc counter |
| Instructions built (841 fns) | — | 9,793,755 | `MFB_BENCH_LOWERING` sum |

Diagnosis: the compile is **allocation-bound**, and the allocations are the
per-operand `Box<str>` churn — production (`allocate_*`/`temporary_vreg` →
`String`), cloning (`linear_scan::run` clones instruction operand vectors +
`Box<str>` per rewrite), and rendering (`Operand::render` → `String`, encode's
`REG_ARRAY.position` string scans). In release the allocator itself is 74% of
self-time; in debug the same string work shows as `eq`/`position`/rendering in
mfb's own (unoptimized) code — same root cause, both builds.

References:

- `planning/completed/plan-78-A-operand-type.md` / `-B-flip-storage.md` /
  `-C-typed-regalloc.md` — the `Operand` type, the storage flip, and the
  deferral this plan closes. Read the plan-78-C **Corrections** section first.
- `src/target/shared/code/operand.rs` — `Operand` enum (`:64`), `render`
  (`:103`), `rendered` (`:93`, Cow), `vreg` ctor (`:79`), `RegClass`.
- `src/target/shared/code/regalloc/analysis.rs` — `int_concrete_physical_index`
  (`:257`), `fp_physical_index` (`:307`), the `REG_ARRAY.position` scans
  (`:280,300,357`), `parse_vreg`.
- `src/arch/aarch64/encode/sizing.rs` (`instruction_size`),
  `src/arch/aarch64/encode/emitter.rs` (`Encoder::emit_instruction`) — the
  encode-side register-string consumers.
- `.ai/compiler.md` — runtime-completion gate, validation/function tests,
  register-lifetime rules the codegen changes must respect.

## Prerequisites

These are a precondition on the **whole** plan-82 feature (A/B/C/D), stated once
here; B, C, and D point back to this table.

| Must be true | Command | Status |
|---|---|---|
| plan-78 A/B merged: `Operand` exists and `CodeInstruction.fields: Vec<(&'static str, Operand)>` | `rg -n 'enum Operand' src/target/shared/code/operand.rs` and `rg -n 'fields: Vec<\(&.*str, Operand\)>' src/target/shared/code/types.rs` | MET |
| A clean byte-identity oracle exists (artifact-gate) | `ls scripts/artifact-gate.sh` | MET |
| Acceptance harness runs | `ls scripts/test-accept.sh tests/acceptance/project.json` | MET |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before starting and before any stop.

Everything below is written against the world where these hold.

## 1. Goal

- `Operand` has a **constructed-in-production** typed physical-register arm
  `Phys { class: RegClass, index: u32 }` alongside the existing `VReg`, and a
  render function that turns `{class, index}` into the **exact** register token
  today's `Raw` string carries, for every arch that consumes the code layer
  (aarch64, x86-64, riscv64) — proven by rendering every physical register name
  in `REG_ARRAY` (and the fp table) round-trip and asserting equality.
- No hot path is flipped yet; `field()` still accepts strings and stores `Raw`.
  The typed arms are additive and covered by tests.
- `artifact-gate.sh … all` is byte-identical to pre-plan (this sub-plan changes
  no emitted bytes).

### Non-goals (explicit constraints)

- **Byte-identity is absolute.** No emitted machine-code byte and no dump byte
  (`--ncode`, `--mir`, `--nir`, diagnostics) may change across all of plan-82.
  Every sub-plan gates on artifact-gate byte-identity.
- No public CLI/flag/manifest surface changes.
- Do not change register *allocation results* (which vreg gets which physical
  reg) — only the representation carrying them. Allocation order is load-bearing
  for goldens (see `vreg-alloc-order-load-bearing`).
- No new global allocator / arena in this sub-plan (that is a separate lever, if
  ever; plan-82 removes allocations by representation, not by swapping malloc).

## 2. Current State

`Operand` (`operand.rs:64`) is `VReg { class, id } | Imm(i64) | Raw(Box<str>)`.
Registers — both virtual (`%vN`/`%fN`) before allocation and physical (`x9`,
`d3`, `w0`, `rax`, …) after — are stored as `Raw(Box<str>)`. `render()`
(`:103`) returns the boxed text verbatim for `Raw`; `rendered()` (`:93`) borrows
it as `Cow::Borrowed`. Physical register tokens are produced as strings by the
allocator's rewrite (`regalloc/mod.rs`, `linear_scan.rs`) and consumed as
strings by the encoder via `int_concrete_physical_index`/`fp_physical_index`
(`analysis.rs:257,307`), which linear-scan `REG_ARRAY.position` to recover the
index.

### The two design premises this sub-plan must falsify or confirm

1. **Physical registers can be rendered faithfully from `{class, index}`.** The
   plan-78 note claims they cannot — "`x0`/`rax`/`zero` all = int index 0;
   `d3`/`v3` alias fp index 3". This is the load-bearing uncertainty and is
   scheduled first. The resolution: rendering is **per-consuming-arch**, and each
   arch's code layer only ever emits ITS OWN register vocabulary. The index→name
   map is the arch's existing `REG_ARRAY` / fp table read forward instead of
   `.position()` read backward. Confirm every physical name in every arch table
   round-trips `name → index → name`.
2. **Compound / addressing-mode operands embed register text.** Operands like
   `[%v5, #16]` or `[x9, x10]` are built by formatting registers into a bracketed
   string and stored as `Raw`. These are a real subset that cannot become a bare
   `VReg`/`Phys`. Measure how many and decide: they stay `Raw` (the register
   inside stays a string) OR gain a typed memory-operand arm. This sub-plan only
   **measures and decides**; B/C act on the decision.

### Measured populations

| What | Count | Command |
|---|---|---|
| Producer call sites (`allocate_register`/`allocate_fp_register`/`temporary_vreg`) | 1825 | `rg '\.allocate_register\(\|\.allocate_fp_register\(\|\.temporary_vreg\(' src/ \| wc -l` |
| …across files | 44 | `rg -l 'allocate_register\|allocate_fp_register\|temporary_vreg' src/ \| wc -l` |
| Register-string consumers (`parse_vreg`/`*_physical_index`/`vreg_name`/…) | 85 occ / 10 files | `rg -c 'parse_vreg\|parse_fp_vreg\|int_concrete_physical_index\|fp_physical_index\|vreg_name\|fp_vreg_name' src/` |
| Render surface (`.render()`/`.rendered()`/`Operand::`) | 125 occ / 21 files | `rg -c '\.rendered\(\)\|\.render\(\)\|Operand::' src/` |
| Compound/addressing-mode `Raw` operands (embedded register) | UNMEASURED | first task of Phase 1 |

### Verified properties

- **Producers bind to a local, then reuse it** (verified by reading
  `builder_collection_layout.rs:109-376`): `let s = self.temporary_vreg();` then
  `s` flows into `.field(...)` and, for addressing modes, into `format!`. So the
  return-type change in C is not purely `.field()`-mechanical — the `format!`
  sites are the compound-operand subset premise (2) above. UNVERIFIED: the exact
  fraction that only ever `.field()` the bare register — Phase 1 measures it.

## 3. Design Overview

Layering (each a separate sub-plan, landed in order, each byte-identical):

- **A (this):** add `Operand::Phys { class, index }` + `render_phys(arch)`;
  prove faithful rendering; measure the compound-operand subset; decide its
  representation. Zero hot-path change.
- **B:** the register allocator constructs typed operands on its hot path —
  reads each operand into a typed `VReg` once, writes the assignment as typed
  `Phys` — instead of parsing/formatting strings and cloning `Box<str>`. This is
  plan-78-C's deferred core and the single biggest allocation win (regalloc
  clones dominate the 566 M code_emit allocs).
- **C:** `allocate_register`/`allocate_fp_register`/`temporary_vreg` return a
  typed handle; the 1825 producer sites bind it; bare-register `.field()` stores
  `VReg` inline (no `Box`). Compound operands per A's decision. Removes
  production-time `Box<str>`.
- **D:** the encoder reads the typed `Phys` index directly, deleting the
  `REG_ARRAY.position` scans (`analysis.rs:280,300,357`) and the encode-side
  string render (215 M encode allocs). Realizes the final perf target.

Correctness risk concentrates in B (allocator rewrite — the emitted register
choice must not shift) and C (1825 sites). Design uncertainty concentrates in A
premise 1 (physical rendering) — hence A is first and cheap.

Rejected alternative: **string interning** (`Raw` holds an interned `u32`).
Rejected because it is a cheaper *substitute* for the deferred typing, not the
typing itself — it keeps the stringly model, still allocates each name once, and
leaves the `position` scans. plan-82 does the real representation change.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box is NOT
> DONE.

### Phase 1 — Measure the compound-operand subset & decide its representation

Falsify/confirm premise 2 before any type is added.

- [ ] Enumerate every production site that builds a `Raw` operand containing a
      register token that is NOT a bare register (addressing modes `[...]`,
      shifted/extended regs, reg lists). Command + count recorded in this file's
      Measured populations table (replace UNMEASURED).
- [ ] Decide and record here: compound operands stay `Raw` (register-in-string)
      **or** gain a typed arm. Recommendation: stay `Raw` for A–C; revisit in D
      only if they block the encode-side scan removal. Record the evidence.

Acceptance: the Measured-populations row is filled with its command, and the
decision is written with a one-line rationale.
Commit: —

### Phase 2 — Add the typed `Phys` arm + faithful per-arch rendering

- [ ] Add `Operand::Phys { class: RegClass, index: u32 }` to `operand.rs` with a
      render path that maps `{class, index}` to the arch's register token by
      reading `REG_ARRAY`/fp-table **forward** (index → name), for each arch that
      consumes the code layer.
- [ ] Route `render()`/`rendered()` for `Phys` through the arch renderer such
      that the produced token is identical to today's `Raw` string.
- [ ] Tests: in `operand.rs` (and/or `regalloc/analysis.rs` tests), for every
      name in each arch's integer `REG_ARRAY` and fp table, assert
      `render_phys(class, position_of(name)) == name` and
      `physical_index(name) == position_of(name)` — a full round-trip over the
      real tables, not a sample.

Acceptance: the round-trip test passes over every physical register name in
every consuming arch's tables; `cargo test --bin mfb` green. No `Phys` is
constructed on any production path yet (the arm may carry a scoped
`#[allow(dead_code)]` with a comment pointing at plan-82-B, removed in B).
Commit: —

### Phase 3 — Byte-identity gate

- [ ] Run `scripts/artifact-gate.sh … all` (the execution-free byte oracle) and
      confirm zero diffs vs the pre-plan tip — this sub-plan added only additive,
      unconstructed types + tests.

Acceptance: artifact-gate reports byte-identity across all four codegen targets;
`.ncodesum` goldens unchanged.
Commit: —

## Validation Plan

- Tests: the full-table round-trip test above (Phase 2), in the code/regalloc
  test modules (`cargo test --bin mfb` — compiler tests live in the bin target,
  not `--lib`).
- Coverage check: the round-trip test iterates the real `REG_ARRAY`/fp tables, so
  a new register added later is automatically in the denominator.
- Runtime proof: none needed — A changes no runtime behavior; byte-identity is
  the proof.
- Doc sync: none (no spec/man surface changes). If `Operand`'s doc comment claims
  physical registers "cannot render faithfully", correct it here with the
  round-trip evidence.
- Acceptance: `scripts/artifact-gate.sh … all` byte-identical; `cargo test`.

## Open Decisions

- Compound-operand representation (Phase 1) — **stay `Raw` through C**
  (recommended) vs. typed memory-operand arm now. Decide in Phase 1 with the
  measured count. (§Phase 1)

## Corrections

<Filled in during execution.>

## Summary

A is the cheap design gate for a huge feature: it proves the one premise plan-78
used to justify the deferral (physical registers "can't render faithfully") is
false, lands the typed `Phys` arm behind a full-table round-trip test, and
changes zero emitted bytes. The real allocation wins are in B (allocator) and
C (producers); D finalizes the encode side. Risk left untouched here: the hot
paths, deliberately — they are B/C/D.
