# plan-85 census — ABI-token emission-site classification (decides the A→E split)

Last updated: 2026-08-03

This is the split-deciding census the write-plan rule ("measure before you estimate")
requires: the gross distribution of every `%arg`/`%ret`/`%sysarg` emission site across
the six target conventions, measured on `main` (aaabffc33 / post-plan-71 f3b62d29a).
Every count carries its command. The **precise per-operand** C-vs-MFB assignment of the
single-role sites is refined via the `@src` `MFB_BUG387_AUDIT` sweep as each site
converts (plan-85-B/C execution) — that refinement does not change the split; this
gross distribution does.

## Measured distribution (current `main`)

| Convention bucket | Sites | Command | Sub-plan |
|---|---|---|---|
| **Syscall** (`%argSys` / `%retSys`) | **16** | `grep -rohE 'abi::SYSARG\[\|%sysarg\|%sysret\|%sysnr' src/target/shared/code/ \| wc -l` | B (folded) |
| **MFB error-Result convention** (`RESULT_*_REGISTER`) | **884** | `grep -rohE 'RESULT_(TAG\|VALUE\|ERROR_MESSAGE\|ERROR_SOURCE)_REGISTER' src/target/shared/code/ \| wc -l` (across **56** files) | **C** |
| **Single-role arg emissions** (`abi::ARG[k]` + `argument_register`) | **1,609** | `grep -rohE 'abi::ARG\[[0-9]+\]' … \| wc -l` = 1590; `… 'argument_register\('` = 19 | B |
| **Single-role result emissions** (`abi::RET[k]` + `return_register()`) | **2,383** | `grep -rohE 'abi::RET\[[0-9]+\]' …` = 572; `… 'return_register\(\)'` = 1811 | B |
| **Total emission sites** | **~4,892** | sum of the above | — |

Sub-count of the MFB Result convention (the dual-role tail):

| Register | Sites | Command |
|---|---|---|
| `RESULT_TAG_REGISTER` (= `RET[0]`) | 403 | `grep -roh 'RESULT_TAG_REGISTER' src/target/shared/code/ \| wc -l` |
| `RESULT_VALUE_REGISTER` (= `RET[1]`) | 395 | `grep -roh 'RESULT_VALUE_REGISTER' …` |
| `RESULT_ERROR_MESSAGE_REGISTER` (= `RET[2]`) | 61 | `grep -roh 'RESULT_ERROR_MESSAGE_REGISTER' …` |
| `RESULT_ERROR_SOURCE_REGISTER` (= `RET[3]`) | 25 | `grep -roh 'RESULT_ERROR_SOURCE_REGISTER' …` |

C-boundary call sites the single-role args feed (context for `%argC`):

| Callee | Sites | Command |
|---|---|---|
| `emit_arena_alloc_call` | 60 | `grep -roh 'emit_arena_alloc_call' src/target/shared/code/ \| wc -l` |
| `emit_arena_free_call` | 12 | `grep -roh 'emit_arena_free_call' …` |
| `emit_symbol_call` | 12 | `grep -roh 'emit_symbol_call' …` |
| other `_mfb_*` helper refs | ~711 | `grep -rohE 'emit_.*_call\(\|_mfb_[a-z_]+' … \| grep -v arena \| wc -l` (fuzzy — includes non-call refs) |

## What the census decides (the split)

Two facts drive the A→E letter split:

1. **The MFB error-Result convention (884 sites, 56 files) is a structurally distinct,
   dual-role tail.** These `RESULT_*_REGISTER` operands are a `Result`'s
   {tag, value, message, source} that are a *result* at production and an *argument* at
   the error-builder call — the exact values plan-71 re-tokenization was falsified on
   (`planning/completed/plan-71-C…` FINAL STATUS). They require the `%retMFB` +
   explicit `%argC`-staging treatment, which is a different reasoning discipline than
   the single-role bulk. → **its own sub-plan, plan-85-C.**

2. **Everything else (~3,992 single-role arg/ret/sys emissions) is the mechanical
   bulk.** A single-role value (an arena size → `%argC`; a genuine function result →
   `%retMFB`; a syscall arg → `%argSys`) converts to exactly one explicit token,
   byte-identically, keyed on the register the fixpoint already gives it. Uniform
   transform, high volume. → **plan-85-B** (syscalls folded in — only 16).

So: **B = the ~4,008 single-role sites** (x-large — split B/B2 by subsystem only if
execution proves the file volume unwieldy; the transform is uniform). **C = the 884
error-Result dual-role sites** (large). **D = ARM/RISC-V staging + self-move elision +
fixpoint deletion + reconcile** (large).

The MFB convention is **aligned from the start** (byte-CHANGING on SysV-x86; the byte-
identical intermediate was considered and dropped — plan-85-A §3). So there is **no
separate alignment letter**: the aligned realization lands in plan-85-A and takes effect
as B/C convert. SysV-x86 goldens regenerate and correctness is gated on rt-behavior;
Win64/AArch64/RISC-V stay byte-identical (already aligned) — that is the cross-target gate.

## Precise per-operand refinement (execution-time, does not change the split)

The single-role C-vs-MFB distinction (is this arg a `%argC` to a `_mfb_*` helper or a
`%argMFB` to an MFB function?) is read per-site from the `@src`-tagged
`MFB_BUG387_AUDIT` sweep during B: the callee at the emission site names the
convention. Recorded per `file:line` as each file converts, appended below during
execution. This is refinement, not the split decision.

_(Per-file work-list appended during plan-85-B/C execution.)_
