# plan-85 census — ABI-token emission-site classification (decides the A→E split)

Last updated: 2026-08-03

This is the split-deciding census the write-plan rule ("measure before you estimate")
requires: the gross distribution of every `%arg`/`%ret`/`%sysarg` emission site across
the six target conventions, measured on `main` (aaabffc33 / post-plan-71 f3b62d29a).
Every count carries its command. The **precise per-operand** C-vs-MFB assignment of the
single-role sites is refined via the `@src` `MFB_BUG387_AUDIT` sweep as each site
converts (plan-85-B/C execution) — that refinement does not change the split; this
gross distribution does.

## Measured distribution (re-measured on base `c0c30e70a`, plan-85-A Phase 3)

> **Correction (plan-85-A Phase 3).** The original table was measured at
> `f3b62d29a`; `main` has since advanced to `c0c30e70a`, so the counts drifted a
> few sites. Re-measured below with the same commands. Two precision fixes: (1) the
> single-role arg/result counts here are `-oh` **occurrence** counts (the raw
> per-`file:line` snapshot in `plan-85-worklist.md` is by `grep -n` **line**, which
> is fewer where a line names two tokens); (2) the syscall bucket's original "16"
> was the **fuzzy** grep including `%sysnr`/`%sysret` mentions in doc comments — the
> precise single-role syscall-arg emission count (`abi::SYSARG[`) is **5**.

| Convention bucket | Sites | Command | Sub-plan |
|---|---|---|---|
| **Syscall** (`%argSys`) | **5** | `grep -rohE 'abi::SYSARG\[[0-9]+\]' src/target/shared/code/ \| wc -l` | B (folded) |
| **MFB error-Result convention** (`RESULT_*_REGISTER`) | **886** | `grep -rohE 'RESULT_(TAG\|VALUE\|ERROR_MESSAGE\|ERROR_SOURCE)_REGISTER' src/target/shared/code/ \| wc -l` (across **56** files) | **C** |
| **Single-role arg emissions** (`abi::ARG[k]` + `argument_register`) | **1,611** | `grep -rohE 'abi::ARG\[[0-9]+\]' … \| wc -l` = 1592; `… 'argument_register\('` = 19 | B |
| **Single-role result emissions** (`abi::RET[k]` + `return_register()`) | **2,387** | `grep -rohE 'abi::RET\[[0-9]+\]' …` = 574; `… 'return_register\(\)'` = 1813 | B |
| **Total emission sites** | **~4,889** | sum of the above | — |

The split is unchanged by the drift: error-Result (886) → plan-85-C; the ~4,003
single-role arg/result/sys sites → plan-85-B.

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

## Per-`file:line` work-list generation (plan-85-A Phase 3)

The complete per-`file:line` enumeration of every emission site — with its **default**
target token by the deterministic rule below — is generated into
**`planning/plan-85-worklist.md`** (a reproducible snapshot on base `c0c30e70a`;
`plan-85-worklist-raw.txt` is the header-less concatenation). Regenerate it with:

```
grep -rnE 'abi::ARG\[[0-9]+\]|argument_register\(' src/target/shared/code/   # B: arg sites
grep -rnE 'abi::RET\[[0-9]+\]|return_register\(\)' src/target/shared/code/    # B: ret sites
grep -rnE 'abi::SYSARG\[[0-9]+\]' src/target/shared/code/                     # B: syscall arg sites
grep -rnE 'RESULT_(TAG|VALUE|ERROR_MESSAGE|ERROR_SOURCE)_REGISTER' src/target/shared/code/  # C
```

**Default target-token rule (per site, justified by the callee/boundary):**

- `abi::RET[k]` / `return_register()` → **`%retMFB[k]`** (a genuine MFB result), or
  **`%retC[k]`** where the value is returned *straight from a C call* (`rax`-exact).
- `RESULT_{TAG,VALUE,ERROR_MESSAGE,ERROR_SOURCE}_REGISTER` → **`%retMFB[0..3]`** (C).
- `abi::SYSARG[k]` → **`%argSys[k]`**; a syscall result → **`%retSys`**.
- `abi::ARG[k]` / `argument_register(k)` → **`%argC[k]`** when the arg feeds a `_mfb_*`
  / arena / `emit_symbol_call` / syscall C boundary, else **`%argMFB[k]`** (an MFB call).

The **only** distinction that needs runtime callee info is the single-role arg
`%argC`-vs-`%argMFB` split. plan-85-B resolves it per-file from the `@src`-tagged
`MFB_BUG387_AUDIT=1` sweep on a **release** build — the callee at the emission site
names the convention. Run per converting file (serially — the fixture build dirs are
shared; see memory `exe-oracle-concurrent-clobber`):

```
MFB_BUG387_AUDIT=1 MFB_TARGET=linux-x86_64 target/release/mfb build -q -target linux-x86_64 <fixture> 2>&1 \
  | grep -E 'BUG387-MISMATCH|@src='
```

> **Audit blind spot (memory `bug387-divergence-audit-blind-to-category2`).** The
> `MFB_BUG387_AUDIT` cross-check reports only *re-tokenizable* divergences; a genuine
> same-register result→arg reuse (Category 2) is invisible (its staging move agrees),
> so "0 mismatches" ≠ "every site is `%argMFB`". B must read the callee at each arg
> site, not trust the audit's silence.

### Per-file distribution (top files — the B and C decomposition)

Single-role **arg** (`abi::ARG[`/`argument_register`), `grep -rEc … | sort -rn` on
base `c0c30e70a` — top files: `audio/macos.rs` 105, `tls/schannel_server.rs` 90,
`tls/openssl.rs` 79, `runtime_helpers.rs` 78, `runtime_helpers_thread.rs` 74,
`fs/io.rs` 66, `net/io.rs` 61, `audio/alsa.rs` 59, `tls/macos/client.rs` 54, … .

Single-role **ret** (`abi::RET[`/`return_register()`) — top files: `tls/openssl.rs`
185, `fs/io.rs` 146, `fs/atomic.rs` 125, `audio/macos.rs` 118, `tls/macos/client.rs`
110, `audio/alsa.rs` 105, `tls/macos/server.rs` 103, `fs/paths.rs` 101, … .

**error-Result** (`RESULT_*_REGISTER`, plan-85-C), 56 files — top: `runtime_helpers_thread.rs`
90, `fs/io.rs` 89, `fs/paths.rs` 68, `link_thunk.rs` 55, `io_stdin.rs` 48, `fs/atomic.rs`
42, `builder_error_emission.rs` 36, `term.rs` 32, `runtime_helpers.rs` 30, … (full
list: `grep -rEc 'RESULT_(TAG|VALUE|ERROR_MESSAGE|ERROR_SOURCE)_REGISTER' src/target/shared/code/ | grep -v ':0$' | sort -t: -k2 -rn`).

_(Per-file conversion progress + the per-file audit callee refinement are appended
here as plan-85-B/C convert each file.)_
