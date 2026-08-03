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
| Compound/addressing-mode `Raw` operands (embedded register) | **0** | `rg -n --glob '!**/tests*' '\.field\([^,]+,\s*&?format!' src/ \| wc -l` (0) — plus `rg -n 'format!\("\[' src/` (only 4, all in `ast/serialize.rs`, not operands) |

### Verified properties

- **Producers bind to a local, then reuse it**: `let s = self.temporary_vreg();`
  then `s` flows into `.field(...)`. **CORRECTED (Phase 1, 2026-08-02):** the plan
  premised that some producers format the register into a compound `format!`
  string (addressing modes). That is FALSE. Addressing modes store the register in
  a **separate `base` field** and the displacement in a **separate `offset`
  field** (`peephole.rs:385` `.field("base","sp").field("offset","1120")`; the
  aarch64 encoder reads them as two fields, `emitter.rs:224` `reg(field(.,"base"))`
  + `immediate(field(.,"offset"))`). Shift amounts are a separate `shift`
  immediate field (`abi.rs:681`), never fused with the register. There are **zero**
  production `.field(name, format!…)` sites (`rg -n --glob '!**/tests*'
  '\.field\([^,]+,\s*&?format!' src/` → 0); the only `.field(…, &format!("d{r}"))`
  is a regalloc **test** (`regalloc/tests.rs:237`). So **every** register operand
  is a bare register in its own field — the return-type change in C is purely
  `.field()`-mechanical, with no compound subset to hand-migrate.

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

- [x] Enumerate every production site that builds a `Raw` operand containing a
      register token that is NOT a bare register (addressing modes `[...]`,
      shifted/extended regs, reg lists). **Count = 0** (command in Measured
      populations table). Addressing uses separate `base`/`offset` fields; shifts
      use a separate `shift` field; no reg-list operand exists. See the corrected
      Verified-properties bullet.
- [x] Decide and record here: compound operands stay `Raw` **or** gain a typed
      arm. **DECISION: the question is moot — there are zero compound-register
      operands.** Every register operand is a bare register in its own field, so
      all of them become typed `VReg`/`Phys` in B/C/D with no `Raw`
      register-in-string survivor. No typed memory-operand arm is needed. Evidence:
      the 0-count census above + the encoder reading `base`/`offset` as two
      separate fields.

Acceptance: the Measured-populations row is filled with its command (count 0), and
the decision is recorded — no compound subset exists, so no register operand stays
`Raw`.
Commit: a40785f2c

### Phase 2 — Add the typed `Phys` arm + faithful rendering

> Design corrected (see Corrections): the arm carries the static name
> (`Phys { class, index, name: &'static str }`), so `render()` is byte-identical
> with no arch parameter and no heap allocation, and `index` is D's direct read.

- [x] Add `Operand::Phys { class: RegClass, index: u32, name: &'static str }` to
      `operand.rs`, with `render()`/`rendered()` returning `name` verbatim.
- [x] `rendered()` for `Phys` borrows `name` (`Cow::Borrowed`, zero-alloc), so a
      downstream reader sees exactly today's `Raw` string.
- [x] Tests: for every name in each arch's integer and fp register table, assert
      `Operand::Phys{class, index: position_of(name), name}.render() == name` and
      `physical_index(name) == position_of(name)` — a full round-trip over the
      real tables (`int_concrete_physical_index`/`fp_physical_index` +
      `riscv_int_index`/`riscv_fp_index` + the aarch64 `x{n}`/`d{n}`/`v{n}`
      spellings), not a sample. Guarantees `Phys.index` == the encoder's
      `.position()` result (plan-82-D's read).

Acceptance: the round-trip test passes over every physical register name in
every consuming arch's tables; `cargo test --bin mfb` green. No `Phys` is
constructed on any production path yet (the arm may carry a scoped
`#[allow(dead_code)]` with a comment pointing at plan-82-B, removed in B).
Commit: cf792e1b3

### Phase 3 — Byte-identity gate

- [x] Run `scripts/artifact-gate.sh … all` (the execution-free byte oracle) and
      confirm zero diffs vs the pre-plan tip — this sub-plan added only additive,
      unconstructed types + tests. **Result: `1146 tests, 1288 build(s), 1553
      golden(s) checked, 0 diff(s)`** (`target/release/mfb all`).

Acceptance: artifact-gate reports byte-identity across all four codegen targets;
`.ncodesum` goldens unchanged. ✓ 0 diffs.
Commit: (recorded next commit)
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

- ~~Compound-operand representation (Phase 1) — stay `Raw` through C vs. typed
  memory-operand arm now.~~ **RESOLVED (Phase 1): moot — zero compound-register
  operands exist. No register operand stays `Raw`; no memory-operand arm needed.**

## Corrections

- **Premise 2 (compound operands embed register text) is FALSIFIED.** The plan
  assumed a real subset of operands fuse a register into a bracketed/compound
  `Raw` string (`[%v5, #16]`) that could not become a bare typed register.
  Measurement (Phase 1): there are **zero** such production sites. Addressing
  modes carry the register in a separate `base` field and the displacement in a
  separate `offset` field; shift amounts are a separate `shift` immediate field;
  no register-list operand exists. Command: `rg -n --glob '!**/tests*'
  '\.field\([^,]+,\s*&?format!' src/` → 0. Consequence: C's migration is purely
  `.field()`-mechanical (no hand-migrated compound class), and D can delete the
  physical-index scans outright with no `Raw` inner-register fallback (plan-82-D
  Open Decision resolves to "no fallback needed").

- **Premise 1 resolution — CONFIRMED, but via a carried `&'static str` name, not a
  forward table scan.** The plan proposed rendering `Phys { class, index }` by
  reading each arch's register table *forward* (index → name) inside `render()`.
  That cannot work as written: `Operand::render()` / `Display` / `rendered()` /
  `PartialEq<str>` carry **no arch parameter**, and the same `{class, index}`
  renders to different tokens per arch (`index 0` = `x0`/`rax`/`zero`), so a
  parameter-less `render()` cannot disambiguate. Threading an arch through every
  `Display`/`format!` diagnostic call site is invasive and unwarranted, because
  every `RegisterModel` already exposes physical names as `&'static str`
  (`allocatable`/`caller_saved` → `&'static [&'static str]`). The faithful,
  **zero-allocation** representation is therefore
  `Phys { class: RegClass, index: u32, name: &'static str }`: `name` (a static
  pointer, no heap box) makes `render()` byte-identical with no arch context, and
  `index` is plan-82-D's direct encode read (== the register-table position,
  proven by the Phase 2 round-trip test). This is exactly the
  `Phys { class, index, name }` arm the plan-78-A module doc anticipated. It meets
  every plan-82 goal (byte-identity, zero heap allocation for physicals, direct
  index read) and confirms premise 1 (physical registers *can* render faithfully
  from a typed value).

## Summary

A is the cheap design gate for a huge feature: it proves the one premise plan-78
used to justify the deferral (physical registers "can't render faithfully") is
false, lands the typed `Phys` arm behind a full-table round-trip test, and
changes zero emitted bytes. The real allocation wins are in B (allocator) and
C (producers); D finalizes the encode side. Risk left untouched here: the hot
paths, deliberately — they are B/C/D.
