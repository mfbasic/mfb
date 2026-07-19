# bug-341: `src/arch/` cleanup cluster — five dead `RegisterModel` methods behind blanket allows, a triplicated `encode()`, the ISA-neutral image types living inside AArch64, and an x86 encoder test suite with 138 assertion-free calls

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup) / Dead-code

Status: Open
Regression Test: the three encoder test suites
(`src/arch/{aarch64,x86_64,riscv64}/encode/tests.rs`) plus
`scripts/artifact-gate.sh` at `diffs=0`.

A cluster of dead code, duplication, and drifted abstraction in `src/arch/` —
the layer that turns the neutral MIR stream into actual machine code. Every item
below was re-verified against the current worktree; where the original review's
numbers were wrong they are **corrected inline and labelled**, and one lead was
dropped after it failed verification (see *Dropped leads*).

This layer emits the bytes that end up in the binary, so "no behavior change"
here means *byte-identical machine code*. That constraint is what makes item
**C1 the mandatory first step**: the x86-64 encoder test suite contains 138
invocations that assert **no byte values at all**, so for the instructions those
calls nominally cover the suite cannot currently detect an encoding regression.
Every other item in this document is guarded by tests that, in x86's case, are
partly not guarding anything. Fix the suite first, then move the code.

The single correct outcome of a fix is that the dead trait surface is gone, each
duplicated encoder helper has exactly one implementation, the ISA-neutral image
types live outside the AArch64 module, and the emitted bytes for every fixture
are **identical** to today's committed goldens.

References:

- Found during the tree-wide cleanup review (Agent 11 — arch encoders), base
  `25c38ba1`.
- `src/docs/spec/architecture/15_x86_64-instruction-set.md` — the x86 register
  model that A6's `ZERO_REGISTER` doc contradicts.
- `src/docs/spec/linker/01_pipeline.md` — the encoder → linker seam B2 crosses.
- `bugs/completed-bugs/bug-85-x86-entry-runtime-arg-staging-tokens.md` — the
  revert that makes D2's comment misleading.
- `planning/old-plans/plan-34-B-role-named-registers.md` — the plan whose
  Phase 4 that comment promises.

### Overlaps with already-filed bugs

- **bug-300-E2** already owns `src/arch/ops.rs:548` (the neutral op vocabulary
  reporting errors as "aarch64"). Re-verified below as **D1** for completeness
  with a corrected test-line citation; the fix belongs to bug-300, not here.
- **bug-300-E5** already owns the x86 `ZERO_REGISTER` / `INT_ALLOCATABLE` doc
  drift. Re-verified as **A6**; land it wherever bug-300 lands, not twice.
- The `src/arch/x86_64/encode/emitter.rs` (2,217 lines) and
  `src/arch/x86_64/encode/tests.rs` (2,406 lines) **file splits** →
  **bug-327**. This document owns the duplication and the dead arms inside
  them, not the split.
- Repo-wide blanket `#![allow(dead_code)]` policy → **bug-326**; the three
  `regmodel.rs` allows are cited here because A1/A2 are what they hide.

## Current State

Measured in worktree `cleanup-review`, base `25c38ba1`.

| File | Lines |
| --- | --- |
| `src/arch/x86_64/encode/tests.rs` | 2,406 |
| `src/arch/x86_64/encode/emitter.rs` | 2,217 |
| `src/arch/aarch64/encode/tests.rs` | 1,270 |
| `src/arch/aarch64/encode/emitter.rs` | 1,226 |
| `src/arch/riscv64/select.rs` | 1,100 |
| `src/arch/x86_64/select.rs` | 1,085 |
| `src/arch/riscv64/encode/tests.rs` | 997 |
| `src/arch/riscv64/encode/emitter.rs` | 739 |
| `src/arch/ops.rs` | 727 |
| `src/arch/x86_64/regmodel.rs` | 275 |
| `src/arch/aarch64/regmodel.rs` | 272 |
| `src/arch/riscv64/regmodel.rs` | 255 |
| `src/arch/riscv64/encode/sizing.rs` | 197 |
| `src/arch/aarch64/encode/sizing.rs` | 165 |
| `src/arch/x86_64/encode/mod.rs` | 155 |
| `src/arch/riscv64/encode/mod.rs` | 143 |
| `src/arch/aarch64/encode/mod.rs` | 192 |
| `src/target/shared/regmodel.rs` | 110 |
| `src/arch/aarch64/select.rs` | 101 |
| **`src/arch/x86_64/encode/sizing.rs`** | **12** |

The last row is the shape the other two ISAs should have (see B3).

Assertion-free calls in the three encoder test suites, counted directly:

| Suite | `let _ = bytes(…)` | `assert!(!enc(…).is_empty())` | Total |
| --- | --- | --- | --- |
| `x86_64/encode/tests.rs` | 78 | 60 | **138** |
| `aarch64/encode/tests.rs` | 0 | 0 | 0 |
| `riscv64/encode/tests.rs` | 0 | 0 | 0 |

*(Correction to the original lead: the `assert!(!enc(…).is_empty())` count is
**60**, not 65, so the total is **138**, not 143. A naive one-line grep
undercounts to 41 because several calls wrap across lines; the 60 was confirmed
by a paren-aware scan of every `assert!(!enc(` site, each of which terminates in
`.is_empty());`. The aarch64 and riscv64 suites do not call `.is_empty()`
anywhere at all.)*

---

## Group A — dead trait surface hidden by blanket allows

`src/target/shared/regmodel.rs:10` and all three
`src/arch/*/regmodel.rs` carry `#![allow(dead_code)]`, so the compiler reports
nothing here. Every deadness claim below was established by grepping `src/` and
`tests/` and classifying each hit as declaration, impl, or test.

### A1 — Five `RegisterModel` methods are dead across all three implementations

Declarations in `src/target/shared/regmodel.rs`: `class_of` (`:31`),
`caller_saved` (`:38`), `emit_move` (`:47`), `closure_env` (`:77`),
`current_thread` (`:87`).

Fifteen implementations (5 methods × 3 arches):

| Method | aarch64 | x86_64 | riscv64 |
| --- | --- | --- | --- |
| `class_of` | `regmodel.rs:104` | `regmodel.rs:82` | `regmodel.rs:102` |
| `caller_saved` | `:131` | `:97` | `:116` |
| `emit_move` | `:165` | `:134` | `:152` |
| `closure_env` | `:176` | `:144` | `:168` |
| `current_thread` | `:183` | `:151` | `:175` |

For all five: **zero** non-declaration, non-impl, non-test call sites anywhere
in the repo. Every remaining hit is inside the three files' own `#[cfg(test)]`
modules (`aarch64/regmodel.rs:201`, `x86_64:167`, `riscv64:191`). 5 declarations
+ 15 implementations + their tests.

For contrast — the trait is not dead, these seven methods are load-bearing and
must stay:

- `allocatable` → `regalloc/linear_scan.rs:98`
- `is_callee_saved` → `linear_scan.rs:253,317,342`; `regalloc/tests.rs:86`
- `emit_spill` → `linear_scan.rs:295,307`
- `emit_reload` → `linear_scan.rs:298,310`
- `arena_base` → `function_lowering.rs:788,923`; `regalloc/mod.rs:264,270`;
  `x86_64/backend.rs:53`; `riscv64/backend.rs:52`
- `spill_slot_bytes` → `target/linux_gtk/mod.rs:611`;
  `builder_codegen_primitives.rs:154`; `regalloc/mod.rs:290`;
  `codegen_utils.rs:568`
- `math_pool_base` → `builder_simd_float_math.rs:2042`

Fix: delete the five declarations, their fifteen implementations, and the tests
that exist only to call them. Then re-evaluate whether the four blanket
`#![allow(dead_code)]` lines are still needed (A6 and B4 are the other two
things they hide).

### A2 — `caller_saved` is dead *because* the allocator hand-rolls the same ISA fact elsewhere

This is the item worth reading twice. `caller_saved` is not dead because nobody
needs the caller-saved set — it is dead because a different module states the
same fact by hand.

`call_clobber_mask` at
`src/target/shared/code/regalloc/analysis.rs:90-153` derives the partition
itself:

```
 97   let caller_saved_int = if is_riscv { RISCV_CALLER_SAVED_INT } else { CALLER_SAVED_INT };
102   let caller_saved_fp  = if is_riscv { RISCV_CALLER_SAVED_FP }  else { CALLER_SAVED_FP };
107   let all_int          = if is_riscv { RISCV_ALL_INT }          else { ALL_INT };
```

Two consequences, both verified:

1. The ISA's caller/callee-saved partition is stated **twice** — once in
   `RegisterModel::caller_saved` (authoritative-looking, three implementations,
   zero callers) and once here (unauthoritative-looking, hand-rolled, the only
   one that runs).
2. The branch is on `is_riscv` only. **x86-64 falls into the `else` arm and
   therefore uses the AArch64 caller-saved mask.** `is_riscv` itself is
   re-derived independently at `regalloc/mod.rs:263-270` from
   `model.arena_base()` — a third place the ISA identity is recovered by hand.

**Scope note, important:** point 2 is a *possible live miscompile*, not a
cleanup item, and this document does **not** fix it. Whether the AArch64 mask is
correct for x86-64 by accident or by luck must be triaged on its own, with its
own bug and its own runtime reproduction. The cleanup here is only: once that
triage lands, `call_clobber_mask` should consult `RegisterModel::caller_saved`
rather than re-branching, which makes `caller_saved` live and deletes the
duplicated fact. Sequencing matters — see *Open Decisions*.

### A6 — x86 `ZERO_REGISTER` is dead, and its own doc says the path is not taken

`src/arch/x86_64/regmodel.rs:45-49`:

```rust
/// The register the legacy `"x31"` zero spelling realizes as. The neutral zero
/// token (`abi::ZERO` = `xzr`) no longer needs it — `store xzr` encodes an
/// immediate zero and `r14` is now allocatable — but the constant is retained for
/// the residual-`x31` selection path (no shared producer emits `x31`).
pub(crate) const ZERO_REGISTER: &str = "r14";
```

Repo-wide grep: the definition, two `#[cfg(test)]` assertions in the same file
(`:209`, `:273`), and one prose mention in a doc comment at
`src/target/shared/abi.rs:111`. **Zero real uses.** And the one place that does
handle the literal `"x31"` — `src/arch/x86_64/select.rs:404-410` — maps it to
`abi::ZERO` (the neutral `"xzr"` token, later encoded as an immediate zero), not
to `ZERO_REGISTER`. So even the residual path the doc reserves it for bypasses
it.

**This is bug-300-E5.** Cited here because it is one of the three things the
x86 `regmodel.rs` blanket allow hides; land it with bug-300.

---

## Group B — duplication and misplacement in the encoders

### B1 — `encode()` is triplicated near-verbatim (~200 redundant lines)

- `src/arch/aarch64/encode/mod.rs:93-192` (100 lines)
- `src/arch/x86_64/encode/mod.rs:57-155` (99 lines)
- `src/arch/riscv64/encode/mod.rs:45-143` (99 lines)

Verified by extracting all three bodies and diffing them. With comments
stripped, the **only** difference anywhere across the three is the arch-name
literal in the duplicate-label panic: `"AArch64: duplicate label …"` /
`"x86_64: duplicate label …"` / `"rv64: duplicate label …"`.

*(Correction to the original lead, which said "+2 comments": there are **5-6**
distinct comment differences, not 2 — the rodata comment wording (lines 2-4);
a "First sub-pass" comment present in x86/riscv and absent in aarch64; the
duplicate-label comment (aarch64 and riscv64 share one wording citing
"bug-127; cf. x86 bug-15", x86 has another citing only "bug-15"); a "Second
sub-pass" comment absent from aarch64 and worded differently between x86 and
riscv; and a 3-line aarch64-only comment about built-in-surface import
defaulting. Only the panic string is a code difference.)*

Fix: hoist one `encode_plan(…, arch_name: &str)` into a shared module; the three
`encode()` fns become one-line delegations. The comment drift is itself the
argument — three copies of the same 100 lines have already accumulated five
divergent explanations of the same two sub-passes.

### B2 — `EncodedImage` and five siblings — the ISA-neutral image types — live inside the AArch64 module

All six defined in `src/arch/aarch64/encode/mod.rs`:

| Type | Lines |
| --- | --- |
| `EncodedImage` | `:17-48` |
| `ImportKind` | `:57-60` (with `#[allow(dead_code)]` at `:56`) |
| `EncodedSymbol` | `:62-66` |
| `EncodedSection` | `:69-72` |
| `EncodedRelocation` | `:74-80` |
| `EncodedImport` | `:82-91` |

Both siblings re-export them verbatim —
`src/arch/x86_64/encode/mod.rs:42-44` and
`src/arch/riscv64/encode/mod.rs:30-32`, both
`pub(crate) use crate::arch::aarch64::encode::{ EncodedImage, EncodedImport,
EncodedRelocation, EncodedSection, EncodedSymbol, ImportKind };` — and both say
in their own module docs that the types are not AArch64's:

- `src/arch/x86_64/encode/mod.rs:4-7`: "*The architecture-neutral container
  types (`EncodedImage`/…/`ImportKind`) are reused verbatim from the AArch64
  encoder — they describe a linkable image, not an ISA.*" and `:40`: "*The
  neutral image/symbol/relocation/import containers are ISA-independent; reuse
  them rather than redeclaring a parallel set.*"
- `src/arch/riscv64/encode/mod.rs:4-6` and `:28` carry the same language.

Eleven sites across eight files outside `src/arch/aarch64/` reach through
`crate::arch::aarch64::encode::`, including **both linkers**:
`src/os/linux/link/mod.rs:1`, `src/os/linux/link/tests.rs:2`,
`src/os/linux/mod.rs:13,93`, `src/os/macos/link/mod.rs:1`,
`src/os/macos/link/tests.rs:3,1146`, `src/os/macos/mod.rs:5,55`, plus the two
sibling re-exports.

Fix: move the six types to a new `src/arch/image.rs`; the two re-exports and the
linker imports point there instead. Pure path change — no type or field moves.

### B3 — aarch64 and riscv64 maintain a hand-written second size table; x86 solved it structurally

The x86 file is 12 lines and says exactly why —
`src/arch/x86_64/encode/sizing.rs:1-5`:

```
//! Instruction sizing. By construction this is exactly the byte count
//! [`super::emitter::emit_instruction`] produces, because both delegate to the
//! one [`super::emitter::encode_instruction`] function — sizing simply discards
//! the relocation/label side effect and returns the byte length. There is no
//! second, drift-prone size table to keep in sync.
```

It derives the size by calling the emitter and measuring (`sizing.rs:10-12`).
The other two do not:

- `src/arch/aarch64/encode/sizing.rs:4-78` — `instruction_size`, a hand-written
  `match instruction.op { … }` of hardcoded byte counts, independent of the
  emitter. (File is 165 lines.)
- `src/arch/riscv64/encode/sizing.rs:131-197` — `instruction_size` (`:131-168`)
  plus `sized_add_imm`/`sized_sub_imm`/`sized_memory` (`:170-197`), hardcoding
  counts such as `CodeOp::AddCarry | CodeOp::SubBorrow => 28` and
  `CodeOp::Rorv => 16`. (File is 197 lines.)

Both admit the sync obligation in comments, though less bluntly than x86's doc:
`aarch64/encode/sizing.rs:64-66` ("*Key the size on the resolved register
number, exactly as `emit_add_carry` does … so a spelling test would disagree
with the emitter*"); `riscv64/encode/sizing.rs:17` ("*so the two-pass sizes
always match*"), `:34` ("*so the two passes agree*"), `:160` ("*so they match
the emitter's `li` sequences exactly*").

This is ~250 lines **and a correctness hazard class**: a size table that
disagrees with the emitter by one byte produces branch displacements computed
against the wrong layout. Fix: adopt the x86 structure on both ISAs — sizing
calls the emitter and discards the side effect. This is the highest-value item
in the document and the one most dependent on the encoder tests actually
asserting bytes (C1).

### B4 — x86 and riscv64 import the neutral register-model trait through an AArch64 back-compat shim

- `src/arch/x86_64/regmodel.rs:14` and `src/arch/riscv64/regmodel.rs:18`:
  `use crate::arch::aarch64::regmodel::{RegClass, RegisterModel};`
- `src/arch/x86_64/backend.rs:8` and `src/arch/riscv64/backend.rs:8`:
  `use crate::arch::aarch64::regmodel::RegisterModel;`
- The shim, `src/arch/aarch64/regmodel.rs:19-23`:

```rust
// `RegClass` + the `RegisterModel` trait were hoisted to the neutral
// `crate::target::shared::regmodel` (plan-34-B Phase 2); re-export them so the
// AArch64 impl below and existing `crate::arch::aarch64::regmodel::…` callers are
// unchanged.
pub(crate) use crate::target::shared::regmodel::{RegClass, RegisterModel};
```

So both newer ISAs formally depend on the AArch64 module for a trait that lives
in `src/target/shared/`. *(Correction: the original lead also cited
`src/arch/aarch64/mod.rs:1-4` as part of this shim. That is a **different**
re-export — `pub(crate) use crate::target::shared::abi;` — unrelated to
`RegisterModel`. What makes the shim reachable is the plain
`pub(crate) mod regmodel;` at `src/arch/aarch64/mod.rs:7`.)*

Fix: point the four imports at `crate::target::shared::regmodel` and delete the
shim.

### B5 — Symbol-resolution / relocation-binding logic triplicated (~110 redundant lines)

- aarch64: `emit_bl` `src/arch/aarch64/encode/emitter.rs:1106-1135` (30 lines)
  and `emit_symbol_ref` `:1141-1181` (41 lines).
- riscv64: `emit_call` `src/arch/riscv64/encode/emitter.rs:624-651` (28 lines)
  and `emit_auipc_ref`/`emit_pageoff` `:655-707` (53 lines).
- x86_64: `record_reloc` `src/arch/x86_64/encode/emitter.rs:130-189` (60 lines).

All three implement the identical branching — `self.symbols.iter().any(…)` →
`"internal"`, `self.imports.get(…)` → `"external"`, else `"data"` — each
constructing an `EncodedRelocation { offset, target, kind, binding, library }`,
differing only in the per-arch error text. The x86 copy documents its own
duplication at `src/arch/x86_64/encode/emitter.rs:126-128`: "*Binding
distinguishes them for the linker, **exactly as the AArch64 encoder does**.*"

Fix: shared `resolve_call_binding` / `resolve_data_binding` taking the arch
label for the error string.

### B6 — ~32 one-line aarch64 emitter wrappers differing only in an opcode constant

**Correction to the original lead** (~40 wrappers / ~300 lines): four of the five
cited ranges hold genuine one-line wrappers of the shape
`self.emit_word(CONST | …)`; the fifth does not.

| Range | Wrappers | Lines | Shape |
| --- | --- | --- | --- |
| `emitter.rs:610-652` | 11 (`emit_add`…`emit_umulh`) | 43 | one-liners ✓ |
| `:707-729` | 5 (`emit_rorv`…`emit_asrv`) | 23 | one-liners ✓ |
| `:906-930` | 6 (`emit_fadd_d`…`emit_fmaxnm_d`) | 25 | one-liners ✓ |
| `:932-971` | 10 (`emit_fneg_d`…`emit_fcvtas_x_from_d`) | 40 | one-liners ✓ |
| `:973-1093` | 10 (`emit_ldr_u64`…`emit_str_u8`) | 121 | **not one-liners** — each has an alignment check, an imm12-range branch, and a scratch-register fallback (7-9 lines of real logic) |

So the mechanical target is **32 wrappers / 131 lines → ~60**, not 40/300. The
load/store range is real code and stays.

Fix: `emit_rrr` / `emit_rr` / `emit_fp2` helpers plus an opcode-constant table
for the 32.

### B7 — `field` / `immediate` / `shift` duplicated verbatim in all three `operand.rs`

Paths are `src/arch/<isa>/encode/operand.rs` (not `src/arch/<isa>/operand.rs`);
the cited line numbers are exact.

- `field()`: aarch64 `:3-15`, x86_64 `:5-17`, riscv64 `:3-15`.
- `immediate()` / `shift()`: aarch64 `:85-103`, x86_64 `:65-83`, riscv64
  `:96-114`.

Byte-for-byte identical between aarch64 and x86_64. The riscv64 copy differs
only by an inconsistent `"rv64 "` prefix on its error strings — e.g.
`"rv64 instruction '{}' missing field '{name}'"` vs the unlabelled
`"instruction '{}' missing field '{name}'"` in the other two. So the three
copies of one helper have already produced two different diagnostic
conventions.

Fix: one shared `operand` module; take the arch label as a parameter and settle
on one convention (labelled or not) for all three.

---

## Group C — the x86 encoder test suite

### C1 — 2,406 lines, two parallel coverage waves, 138 calls that assert nothing

**This is the first thing to fix in this document.** `src/arch/x86_64/encode/`
produces real machine code; its test suite is the guard for every other item
here. For the instructions covered only by an assertion-free call, that guard
does not exist.

Two waves with two helpers:

- Wave 1 — helper `fn bytes(op, fields) -> Vec<u8>` at
  `src/arch/x86_64/encode/tests.rs:28`; tests run `:44` (`mov_reg_reg`) through
  `:1677` (`unsupported_op_errors`), with a full-plan/`encode`-image block at
  `:1453-1677` belonging to neither wave.
- Wave 2 — helper `fn enc(op, fields) -> Vec<u8>` at `:1696`; tests run `:1705`
  (`label_and_add_pageoff_are_empty`) through `:2391`
  (`encode_unresolved_call_and_label_error`).

The two waves cover overlapping ground under near-identical names:

| Wave 1 | Wave 2 |
| --- | --- |
| `label_and_pageoff_are_empty` `:646` | `label_and_add_pageoff_are_empty` `:1705` |
| `rev_word_and_quad` `:670` | `rev_w_rev_x` `:1718` |
| `rbit_bit_reverse_sequence` `:697` | `rbit_reverse_bits` `:1744` |
| `shifts_var` `:323` | `shifts_var_32bit` `:1783` |
| `msub_extended_registers` `:1275` | `msub_disjoint_and_dst_aliases_lhs` `:1756` |
| `div_aliasing_and_preservation` `:1265` | `div_aliasing_and_dividend_preservation` `:1771` |
| `float_int_conversions` `:910` | `int_float_conversions` `:1978` |
| `v128_unary_and_neg_abs` `:1039` | `v128_unary_and_negabs` `:2052` |

And 138 of the invocations assert nothing about the bytes: **78** of the form
`let _ = bytes(…)` and **60** of the form `assert!(!enc(…).is_empty())`. The
latter asserts only that *some* bytes came back — any wrong encoding of the
right length, or of any non-zero length, passes.

The aarch64 (1,270 lines) and riscv64 (997 lines) suites have **zero** such
calls; neither file calls `.is_empty()` anywhere.

Fix, in order: (1) replace every `let _ = bytes(…)` and
`assert!(!enc(…).is_empty())` with an assertion on the expected byte sequence,
taking the expected bytes from the current output only after independently
confirming each against the Intel encoding — a snapshot of possibly-wrong bytes
is not a fix; (2) merge the two waves into one taxonomy with one helper;
(3) hand the resulting file to bug-327 for the split.

Until (1) lands, treat any x86 encoder change in this document — B1, B3, B5,
B7, D3 — as unguarded.

### C2 — `fresh_encoder` duplicated verbatim in all three test files

`src/arch/aarch64/encode/tests.rs:268-278`, `x86_64:14-23`, `riscv64:8-48`;
aarch64's helpers additionally sit mid-file (`:489-508`) rather than at the top.

Fix: hoist one test helper; move aarch64's to the top of its file. Do this after
C1, not before — it touches the same file.

---

## Group D — stale documentation, dead arms, and one abstraction that has drifted

### D1 — `src/arch/ops.rs` reports errors as "aarch64" in the arch-neutral vocabulary

`src/arch/ops.rs:548`, inside `CodeOp::from_mnemonic`:

```rust
other => Err(format!("aarch64 code op '{other}' is not encodable")),
```

`src/arch/ops.rs` is the neutral MIR vocabulary for all backends, so a bad
mnemonic on the x86 or riscv path misreports "aarch64". Nothing catches it: the
test `unknown_mnemonic_errors` (`:723`) asserts only
`err.contains("not encodable")` at **`:725`** *(corrected from the lead's
`:722`, which is a closing brace)*.

**This is bug-300-E2.** Recorded here as verified-still-present; the fix belongs
to bug-300.

### D2 — `select_aarch64` cites a plan phase that was attempted and reverted

`src/arch/aarch64/select.rs:87-91`:

```
// Realize the plan-34-B role tokens (`%arg`/`%ret`/`%sysnr`/…) to their
// AArch64 register spellings — the temporary Phase-3b seam that keeps the
// encoder on today's `xN` input (byte-identical); Phase 4 deletes this and
// realizes tokens directly. Then realize `arena_base` back to its pinned
// register (plan-00-D §2, plan-34-A).
```

Phase 4 landed as `c098504f`, broke every x86-64 program, and was reverted at
`a23aee06` — see `bugs/completed-bugs/bug-85-x86-entry-runtime-arg-staging-tokens.md`,
which leaves the follow-up explicitly OPEN. The revert is visible in current
code: `src/arch/x86_64/select.rs` still contains `abi_boundary_of` (`:23`),
`map_abi_register` (`:80`), and `remap_x86_abi` (`:107`) — precisely the
inference seam Phase 4 was meant to delete. Meanwhile
`planning/old-plans/plan-34-B-role-named-registers.md` still reads
"STATUS: COMPLETE (2026-07-10)" with Phase 4 listed as landed.

So the comment tells a reader to expect a deletion that is not coming, and the
archived plan agrees with the comment rather than with the code. Fix: reword to
state that the seam is permanent (both other backends are built on it) and that
the Phase 4 attempt was reverted per bug-85; annotate the archived plan.

### D3 — Seven unreachable neutral-MIR alias arms in the x86 dispatch (eight dead alias tokens)

**Correction to the original lead** ("eight arms at `:293,708,714,811,817,824,848,866` plus `:827`"):

- `src/arch/x86_64/encode/emitter.rs:293` is **not** a match arm — it is
  `let m = instruction.op.mnemonic();`, the line above `match m {`.
- `:827` is **not** a separate arm — it is an `if` inside the body of `:824`.

The genuine arms, each with a dead first alternative:

| Line | Arm | Dead alias |
| --- | --- | --- |
| `:708` | `"fmov_i2f" \| "fmov_d_from_x"` | `fmov_i2f` |
| `:714` | `"fmov_f2i" \| "fmov_x_from_d"` | `fmov_f2i` |
| `:811` | `"i2f" \| "scvtf_d_from_x"` | `i2f` |
| `:817` | `"f2i_trunc" \| "fcvtzs_x_from_d"` | `f2i_trunc` |
| `:824` | `"f2i_floor" \| "fcvtms_x_from_d" \| "f2i_ceil" \| "fcvtps_x_from_d"` | `f2i_floor`, `f2i_ceil` (two) |
| `:848` | `"f2i_nearest" \| "fcvtas_x_from_d"` | `f2i_nearest` |
| `:866` | `"rorv_w" \| "rotr_w"` | `rotr_w` |

**7 arms, 8 dead alias tokens.**

Mechanism confirmed: the `mir_ops!` macro's `renamed` block at
`src/target/shared/code/mir.rs:65-184` gives these ops a **display-only** neutral
mnemonic used solely by `MirOp::mnemonic()` for the `-mir` dump.
`MirOp::to_code()` (`mir.rs:101-109`) converts back to the canonical `CodeOp`
before any `CodeInstruction` is built, and `src/arch/x86_64/select.rs:683-692`
performs exactly that conversion, commenting: "*Non-fused MIR ops map 1:1 to a
CodeOp via `to_code` (which applies the neutral→concrete renames, e.g.
`call`→`bl`)*". A repo-wide grep confirms the eight alias strings appear nowhere
outside `mir.rs` (as display strings) and these emitter arms — no
`CodeInstruction` is ever constructed with them. This is x86-only residue; the
other two encoders have no such arms.

Fix: delete the eight alias tokens; better, match on `CodeOp` rather than on the
mnemonic string so the compiler enforces exhaustiveness and this class cannot
recur.

### D4 — riscv64 `FT0` is dead, kept alive by a `const _` for phases that are complete

- `src/arch/riscv64/encode/emitter.rs:40`: `const FT0: u8 = 0;`
- `src/arch/riscv64/encode/emitter.rs:738-739` (the file is exactly 739 lines):

```rust
// Scratch registers reserved for later phases (referenced to keep them named).
const _: (u8, u8, u8) = (T1, T2, FT0);
```

Repo-wide grep for `FT0` finds only this definition, this scaffolding line, and
`src/arch/riscv64/v128.rs:44` `const FT0: &str = "ft0";` — a **separate,
unrelated** constant (a register-name string used at selection time, not the
numeric encoding-level `u8`). The "later phases" are complete: `v128.rs` is a
full 1,164-line scalarized-v128 implementation with no `TODO`/`unimplemented!`,
and it uses its own string constant rather than the emitter's numeric one.

`T1` and `T2`, folded into the same tuple, are **not** dead — both have dozens
of real uses (`emitter.rs:172-174`, `:362-433`, `:468-498`), so they need no
artificial reference.

Fix: delete `FT0` and the `const _` line entirely.

### D5 — The largest drifted abstraction in `src/arch` — context, not a near-term fix

Recorded here as rationale for the rest of the document, **not** as an
actionable item. Do not attempt this as part of this bug.

The "neutral" MIR stream is not neutral: it still carries AArch64 physical
register names (`xN`, `dN`, `sp`, `lr`, `xzr`), so both non-AArch64 backends
carry a hand-tuned layer whose job is to un-AArch64 the stream before encoding.

- `src/arch/x86_64/select.rs:34-101` — doc comment plus
  `fn map_scratch_register(n: usize) -> &'static str` (`:36`) through the
  `map_abi_register` block; `remap_x86_abi` at `:107`.
- `src/arch/riscv64/select.rs:534-651` — doc comment (`:534-548`),
  `fn map_scratch_register(n: usize) -> String` (`:551`), `map_fp_register`,
  `remap_riscv_abi`, `remap_register` (ending `:651`).

Two symptoms of the drift, both verified:

1. The two `map_scratch_register` functions have **divergent signatures** —
   `-> &'static str` on x86, `-> String` on riscv64 — for the same conceptual
   operation.
2. `remap_x86_abi` runs a genuine **forward fixpoint dataflow analysis**
   (`src/arch/x86_64/select.rs:208-244`) to *recover* the call/syscall/ret ABI
   role context that the neutral stream lost:

```rust
let mut boundary_before: Vec<Option<AbiBoundary>> = vec![None; count];
let mut changed = true;
while changed {
    changed = false;
    for i in 0..count {
        …
        if new_val != boundary_before[i] { boundary_before[i] = new_val; changed = true; }
    }
}
```

An abstraction that requires a fixpoint analysis downstream to reconstruct
information it discarded upstream is the definition of a leaky one. Both
backends' remap logic operates on literal `xN`/`dN`/`sp`/`lr`/`xzr` strings
(`src/arch/riscv64/select.rs:599-651`, `src/arch/x86_64/select.rs:401-410`).

The fix is a real design change (role tokens realized per-ISA at selection, no
AArch64 spelling in the stream) — which is precisely what plan-34-B Phase 4
attempted and bug-85 reverted (D2). It needs its own plan, not a cleanup bug.
It is stated here so that a reader of D2's reworded comment understands *why*
the seam is permanent for now.

---

## Dropped leads

- *"The `#[allow(dead_code)]` on `ImportKind` (`src/arch/aarch64/encode/mod.rs:56`)
  is no longer needed because linker tests construct `Data`."* — **refuted, do
  not act on it.** The attribute and the cited construction sites are real
  (`src/os/linux/link/tests.rs:150,436`; `elf.rs:678-719` matches on `Data` in
  four arms), but `tests.rs` compiles only under `#[cfg(test)]`
  (`src/os/linux/link/mod.rs:49-50`), and `elf.rs` only *matches* the variant —
  matching does not satisfy rustc's `variant is never constructed` lint. In a
  non-test build `Data` is still never constructed, so removing the attribute
  reintroduces a warning. The enum's own doc at `:52-54` already says so
  ("*`Data` is produced by a `tls`/app-mode consumer (and the linker tests) once
  one exists*"). Leave it.

## Goal

- Zero dead `RegisterModel` surface: the five methods and their fifteen
  implementations are gone, and each remaining blanket `#![allow(dead_code)]` in
  `src/arch/*/regmodel.rs` is either justified in a comment or deleted.
- One `encode()`, one relocation-binding helper, one `operand` module, one
  `fresh_encoder` — not three each.
- The six ISA-neutral image types live in `src/arch/image.rs`; nothing outside
  `src/arch/aarch64/` names `crate::arch::aarch64::encode::`.
- No ISA imports a neutral trait through the AArch64 module.
- aarch64 and riscv64 derive instruction sizes from the emitter, as x86 does;
  no second size table exists in the tree.
- The x86 encoder test suite asserts concrete byte values for every instruction
  it covers — zero `let _ = bytes(…)` and zero
  `assert!(!enc(…).is_empty())` remain.
- Emitted machine code for every fixture is **byte-identical** to today's
  committed goldens.

### Non-goals (must NOT change)

- **Any emitted byte.** This layer produces the machine code; a single changed
  instruction encoding is a failed change, not an improvement.
- The `EncodedImage` field set, the relocation `kind`/`binding` string values,
  and the linker's view of them (B2 moves the types, not their contents).
- Instruction selection: which `CodeOp` is chosen for a given MIR op. B1/B3/B5
  move *encoding* code only.
- The neutral-stream design itself (D5) — explicitly out of scope, needs a plan.
- The `is_riscv`-only branch in `call_clobber_mask` (A2) — its **correctness for
  x86-64** is a separate triage with its own bug; this document must not
  "clean it up" into a behavior change.
- The tempting wrong fix, named and forbidden: **do not close C1 by snapshotting
  current output into `assert_eq!`.** Recording whatever bytes the encoder
  happens to emit today converts an untested path into a *pinned* path without
  ever checking it is correct — and if any of those 138 encodings is wrong, the
  snapshot makes the bug permanent. Each expected byte sequence must be confirmed
  against the ISA reference independently.

## Blast Radius

- `src/target/shared/regmodel.rs` + the three `src/arch/*/regmodel.rs` — fixed
  by A1/B4/A6.
- `src/target/shared/code/regalloc/analysis.rs:90-153` and
  `regalloc/mod.rs:263-270` — the duplicated ISA fact from A2. **Latent, out of
  scope**: the x86-64 mask question is a correctness triage, not a cleanup.
- `src/arch/{aarch64,x86_64,riscv64}/encode/mod.rs` — B1, B2.
- `src/arch/{aarch64,x86_64,riscv64}/encode/emitter.rs` — B5, B6, D3, D4.
- `src/arch/{aarch64,x86_64,riscv64}/encode/operand.rs` — B7.
- `src/arch/{aarch64,riscv64}/encode/sizing.rs` — B3.
  `src/arch/x86_64/encode/sizing.rs` is the reference and is unaffected.
- `src/os/linux/{mod,link/mod,link/tests}.rs`,
  `src/os/macos/{mod,link/mod,link/tests}.rs` — import-path updates from B2
  only; no linker logic changes.
- `src/arch/ops.rs:548` — bug-300-E2, **not fixed here**.
- `src/arch/x86_64/regmodel.rs:45-49` — bug-300-E5, **not fixed here**.
- `src/target/shared/code/mir.rs:65-184` — the rename table that makes D3's arms
  unreachable; unaffected, it is the *reason* they are dead.
- `src/arch/{x86_64,riscv64}/select.rs` — D5 context only, **unaffected**.

## Fix Design

The encoders emit the actual machine code, so the encoder test suites — not the
type checker and not the acceptance goldens alone — are the real guard for every
item here. That drives the ordering:

**C1 comes first, before any code moves.** With 138 assertion-free calls, the
x86 suite currently cannot detect an encoding regression it is nominally
covering. Doing B1 (hoisting `encode()`), B3 (deleting x86's size derivation's
counterparts on the other ISAs), B5, B7, or D3 while the suite is in this state
means moving encoder code with a partial safety net. Land C1's byte assertions,
confirm them independently, then move code.

Ordering after C1: A1 and D4 (pure deletion, nothing can move); B4 and B2 (path
changes, no logic); B1, B5, B6, B7 (dedup, one arch pair at a time); D3 (dead
arms, then the `CodeOp` match that prevents recurrence); B3 last, as it is the
only item that changes *how a size is computed* and therefore the only one where
a mistake produces wrong branch displacements rather than a compile error. D2 is
a comment edit and can land any time.

Rejected alternative for B3: keep the hand-written tables and add a test
asserting `instruction_size(op) == emit(op).len()` for every `CodeOp`. Rejected
— it makes the drift detectable rather than impossible, and requires enumerating
every op/operand shape, which is the same maintenance burden in a new place.
x86 already demonstrates the structural answer in 12 lines.

Rejected alternative for B2: leave the types where they are and add a neutral
`pub use` alias elsewhere. Rejected — that adds a third path to the same types
rather than removing the inversion, which is exactly how B4's shim arose.

Expected output shift: **none**, for every item. Any diff in emitted bytes is a
defect in the change, not an intended consequence.

## Phases

### Phase 1 — make the x86 encoder suite a real guard (C1)

- [ ] Record a clean `scripts/artifact-gate.sh <exe>` baseline at `diffs=0`.
- [ ] Replace all 78 `let _ = bytes(…)` and all 60
      `assert!(!enc(…).is_empty())` with concrete byte assertions. Confirm each
      expected sequence against the Intel encoding independently — do **not**
      snapshot current output.
- [ ] Record any encoding found to be wrong during that confirmation as its own
      bug; do not fix it inside this cleanup.
- [ ] Merge the two coverage waves into one taxonomy with one helper; resolve
      the eight near-duplicate name pairs.
- [ ] C2: hoist `fresh_encoder`; move the aarch64 mid-file helpers to the top.

Acceptance: zero assertion-free calls remain in any of the three encoder
suites; `cargo test` green; `artifact-gate.sh` still `diffs=0` (tests do not
affect output).
Commit: —

### Phase 2 — deletions and path moves (no logic change)

- [ ] A1: delete the 5 declarations, 15 implementations, and their tests;
      re-evaluate each `#![allow(dead_code)]`.
- [ ] D4: delete riscv64 `FT0` and the `const _` scaffolding.
- [ ] D3: delete the 8 dead alias tokens across the 7 x86 arms.
- [ ] B4: repoint the four imports at `crate::target::shared::regmodel`; delete
      the shim at `src/arch/aarch64/regmodel.rs:19-23`.
- [ ] B2: move the six image types to `src/arch/image.rs`; update the two
      sibling re-exports and the six linker/OS import sites.
- [ ] D2: reword the `select_aarch64` comment; annotate the archived plan-34-B.

Acceptance: `artifact-gate.sh` at `diffs=0` after **each** commit; all three
encoder suites green; `cargo build` warning-free.
Commit: —

### Phase 3 — deduplication

- [ ] B1: one `encode_plan(…, arch_name)`; three one-line delegations.
      Reconcile the five divergent comments into one explanation.
- [ ] B5: shared `resolve_call_binding` / `resolve_data_binding`.
- [ ] B7: one shared `operand` module; settle the `"rv64 "` prefix convention.
- [ ] B6: `emit_rrr`/`emit_rr`/`emit_fp2` + opcode table for the 32 aarch64
      one-liners; leave `:973-1093` alone.
- [ ] D3 follow-up: match on `CodeOp` instead of the mnemonic string so
      exhaustiveness is compiler-enforced.

Acceptance: `artifact-gate.sh` at `diffs=0` after each commit; encoder suites
green on all three ISAs.
Commit: —

### Phase 4 — B3, the size tables

- [ ] aarch64: replace `src/arch/aarch64/encode/sizing.rs:4-78` with the x86
      derivation (call the emitter, discard the side effect, return the length).
- [ ] riscv64: same for `:131-197` including the three `sized_*` helpers.
- [ ] Before deleting either table, add a temporary test asserting the old
      table and the new derivation agree for every `CodeOp` and operand shape.
      **If they disagree anywhere, that disagreement is a real bug — stop, file
      it, and do not proceed until it is understood.**
- [ ] Delete the temporary test with the tables.

Acceptance: `artifact-gate.sh` at `diffs=0`; the agreement test passed before
deletion; branch displacements unchanged in every golden.
Commit: —

### Phase 5 — full validation

- [ ] `scripts/test-accept.sh` full run on macOS and Linux, both arches.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`.
- [ ] Confirm every committed binary golden is byte-identical.

Acceptance: full suite green on every target; no golden moved.
Commit: —

## Validation Plan

- Regression tests: the three encoder suites are the primary guard, and Phase 1
  is what makes the x86 one trustworthy. Phase 4 adds a temporary
  table-vs-emitter agreement test whose *purpose is to be deleted*.
- Runtime proof: `scripts/artifact-gate.sh` at `diffs=0` on every commit — for
  an output-preserving change in the encoder this *is* the proof — plus one full
  `scripts/test-accept.sh` run per target before merge. Because this layer emits
  machine code, `test-accept.sh` (which actually runs the produced binaries) is
  not optional here the way it might be for a front-end cleanup.
- Byte-identity guard: `git status` must show **zero** modified files under any
  `tests/**/golden/` directory, and every emitted binary must be byte-identical.
  If a golden moves, the change is out of scope and must be reverted, not
  re-baselined.
- Doc sync: `src/docs/spec/linker/01_pipeline.md` if B2 changes the cited
  encoder path; `planning/old-plans/plan-34-B-role-named-registers.md` for D2's
  Phase 4 annotation. Otherwise none expected.
- Full suite: `cargo test`, `scripts/test-accept.sh`, `cargo clippy`.

## Open Decisions

- **A2 sequencing.** The x86-64-uses-the-AArch64-mask question must be triaged
  as its own bug **before** `call_clobber_mask` is refactored to consult
  `RegisterModel::caller_saved`. Recommended: delete the five dead methods now
  (A1) *including* `caller_saved`, and reintroduce a single caller-saved
  accessor as part of that triage; alternative is to keep `caller_saved` alive
  as a placeholder, which leaves the duplicated fact in place indefinitely.
- **C1 scope.** Recommended: confirm all 138 encodings independently even though
  it is the slowest part of this bug — that confirmation is the entire value of
  the item. Alternative (snapshot now, verify later) is explicitly forbidden in
  Non-goals.
- **B7 error-string convention.** Recommended: no arch prefix (matching aarch64
  and x86_64, the majority) and drop riscv64's `"rv64 "`; alternative is to add
  the label everywhere. Either way, one convention.
- **B3 on riscv64 vs aarch64 first.** Recommended: aarch64 first (smaller table,
  4-78 vs 131-197, and the reference backend); alternative is riscv64 first
  since its hardcoded multi-instruction counts (`AddCarry => 28`) are the more
  fragile ones.
- **D1 / A6 ownership.** Recommended: leave both in bug-300 and delete these
  entries when bug-300 lands; alternative is to move them here. Either way, one
  owner — do not fix them twice.

## Summary

Twenty-one verified cleanup items in `src/arch/`. The engineering risk is
concentrated in exactly two places. **C1**, because the x86-64 encoder test
suite has 138 invocations that assert no byte values, which means the suite
nominally covering those instructions cannot detect an encoding regression —
so it must be repaired *before* any encoder code moves, and repaired by
confirming the expected bytes, not by snapshotting them. And **B3**, because
replacing two hand-written size tables with an emitter-derived length is the one
change here that can silently produce wrong branch displacements rather than a
compile error; it lands last, behind a temporary agreement test.

Everything else — five dead trait methods with fifteen implementations, a
triplicated 100-line `encode()`, the six ISA-neutral image types stranded inside
AArch64, two ISAs importing a neutral trait through an AArch64 shim, 32
copy-paste emitter wrappers, three copies of `field`/`immediate`/`shift`, eight
unreachable alias tokens, and a dead `FT0` propped up by a `const _` — is
mechanical and provable by `scripts/artifact-gate.sh` staying at `diffs=0` with
byte-identical binaries.

Left untouched and deliberately so: the neutral-stream design itself (D5 — the
largest drifted abstraction in the layer, needing its own plan, not a cleanup
bug), the `call_clobber_mask` x86-64 correctness question (A2 — its own triage),
`src/arch/ops.rs:548` and x86 `ZERO_REGISTER` (bug-300-E2/E5), the encoder file
splits (bug-327), and the `ImportKind` allow (verified still necessary).
