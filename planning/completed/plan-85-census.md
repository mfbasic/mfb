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

### MFB_BUG387_AUDIT @src sweep results (plan-85-A Phase 3, on the A-only release build)

Ran `MFB_BUG387_AUDIT=1 target/release/mfb build -target linux-x86_64 <fixture>` over
**135 error-path fixtures** (`tests/rt-error/**` + `tests/syntax/app/**`) on the byte-identical
A-only release binary. Result: **14,748 `BUG387-MISMATCH` lines across 273 distinct `@src`
sites** — the sites where the fixpoint's role inference *disagrees* with the context-free
`map_token_direct`, i.e. the genuinely **byte-changing MFB-result** conversion targets
(the `%argC`/incoming-arg sites do NOT diverge — they already agree, confirming Correction
C1's "args are byte-identical"). Top divergence sites saved to `plan-85-audit-src.txt`;
aggregated by file (top-40 by fixture frequency):

- **`builder_error_emission.rs` (19 of the top sites)** — the error-Result production/park/
  spill path → **plan-85-C** (`RESULT_*_REGISTER` → `%retMFB[0..3]`). By far the dominant
  divergence (the `:88`/`:104`/`:106`/`:90` spill-register moves alone are ~4,000 lines).
- **`entry.rs` / `process_lifecycle.rs` (7 sites)** — entry/exit result staging (the
  `main` return → exit-arg boundary) → **plan-85-B**.
- **`io_stdout.rs` (5), `builder_collection_layout.rs` (3), `builder_inplace_assign.rs` (2),
  `builder_strings.rs`, `error_result.rs`, `crypto_ec/openssl.rs`, `arena.rs`** — MFB-result
  reads (arena_alloc `RET[1]`, helper own-returns) → **plan-85-B** (the cross-file atomic
  result move, Correction C1).

This confirms the split: the divergences (byte-changing sites) are the MFB-result
convention — error-Result (C) + result staging/reads (B) — while the arg sites are
byte-identical (no divergence), so the args batch converts freely and the result move is
the byte-changing core. The `@src` sweep is blind to Category-2 (memory
`bug387-divergence-audit-blind-to-category2`), so it under-reports same-register result→arg
reuse; the grep work-list (`plan-85-worklist.md`) remains the complete site enumeration.

_(Per-file conversion progress + the per-file audit callee refinement are appended
here as plan-85-B/C convert each file.)_

## plan-85-B Phase 1 — `%retC` boundary inventory (gathered during plan-85-A)

Enumerated the MFB↔C value crossings (read-only source analysis; the conversion +
per-boundary fix land in plan-85-B). **204** external/libc/variadic call sites across
**37** files (`grep -rnE '\.emit_libc_call\(|emit_external_int_call\(|\.emit_variadic_call\('
src/target/shared/code/ | wc -l` = 204; the 37 files are those calling `emit_libc_call`/
`emit_external_int_call`/`emit_variadic_call`/`emit_errno`).

**Key finding (confirms the plan's central uncertainty).** `abi::return_register()`
(= `RET[0]`, AArch64 `x0`) is **dual-role at a C boundary**: e.g. `net/io.rs:203`
stages the `poll()` **argument** into it *before* `emit_libc_call`, and `net/io.rs:217`
reads the C **result** from it *after*. On AArch64 both are `x0` so one token sufficed;
on SysV arg0=`rdi` but the C return=`rax`, which is exactly why `remap_x86_abi` infers
the role from the call boundary. Under the aligned convention B must split each use:

- `return_register()`/`ARG[k]` **staged before** a C call (an argument) → **`%argC[k]`**
  (SysV `rdi,rsi,…`).
- `return_register()`/`RET[0]` **read after** a C call (the C result) → **`%retC0`**
  (SysV `rax`) — NOT `%retMFB0` (which aligns to `rdi`). A C `int` return is
  `sign_extend_word`'d in place (`net/io.rs:217`), so the extend's dst/src is `%retC0`.
- A C result then **moved into MFB's result** (`RESULT_VALUE_REGISTER`/`%retMFB`) needs
  the explicit `%retC0`→`%retMFB`/`%argMFB` staging move (SysV `mov rdi,rax`).

C-boundary files (the plan-85-B conversion set): `audio/{alsa,macos,windows,windows_io,windows_open}`,
`crypto`, `crypto_ec/{cng,macos,openssl}`, `datetime`, `entry`, `fs/{atomic,io,mod,paths}`,
`link_thunk`, `native_helpers`, `net/{io,mod,poll}`, `os/{env,introspect,paths}`, `perf`,
`stdin_broadcast`, `tls/{macos/*,mod,openssl,schannel*}`, `types`.

> The audit sweep (`MFB_BUG387_AUDIT=1` release build) confirms per-site which `RET[0]`
> use is the C result (post-call) vs an MFB result; it is **blind to Category-2** same-
> register reuse, so B reads the pre/post-call position at each site, not the audit's
> silence (memory `bug387-divergence-audit-blind-to-category2`).

### B Phase 1 audit CONCLUSION — the C boundary is cleanly separable (no old-`RETS` dependency)

The plan's central uncertainty was: *does any FFI / entry / runtime-helper / 2-register
C-return path silently rely on MFB returning in the old `RETS = [rax,rdx,rcx,rsi]`?*
**Answer: no.** Auditing every multi-register `RET[k]` read (`grep -rohE 'abi::RET\[[123]\]'
src/target/shared/code/ | sort | uniq -c` → `RET[1]` ×546, `RET[2]` ×2, `RET[3]` ×1):

- **`RET[1]` (×546)** is overwhelmingly the `_mfb_arena_alloc` result — an **MFB** 2-value
  `{tag@RET[0], ptr@RET[1]}` result (comments: "alloc result → vreg base") → `%retMFB[0..1]`
  (aligned). `arena_alloc` is an MFB-internal helper, not a genuine C function; its 2-value
  result is exactly what the aligned bank carries. No C 2-register return reads `RET[1]`.
- **`RET[2]`/`RET[3]`** are `RESULT_ERROR_MESSAGE_REGISTER`/`RESULT_ERROR_SOURCE_REGISTER`
  (`error_constants.rs:27/31`), the error-Result convention → `%retMFB[2..3]` (plan-85-C);
  plus one read of an **incoming argument** via a `RET` token at a helper entry
  (`fs/io.rs:901` reads `mode` = incoming arg2 as `RET[2]`), which is byte-safe under
  alignment because `%retMFB[k] == %argMFB[k]` on SysV.
- **Genuine C returns are single-value in `rax` (`%retC0`)**, read via `return_register()` /
  `RET[0]` immediately after a libc `bl` (e.g. `net/io.rs:217`). A 2-value C return (rare)
  uses `rax:rdx = %retC[0..1]`. **Nothing reads a C result from `RET[1..3]`.**

Therefore the aligned convention cannot silently corrupt a C return: every multi-register
`RET` read is an MFB result (aligned) or the error-Result convention (plan-85-C), never a
C 2+-register return. plan-85-B's byte-changing conversion rests on solid ground. (The
"still byte-identical, no conversion yet" half of B Phase 1's acceptance is the plan-85-A
finalization gate, since B Phase 1 converts nothing.)

## plan-85-C Phase 1 — Result-returning `_mfb_*` helper inventory (gathered during plan-85-A)

The `_mfb_*` helpers that return a `Result` / a single C-style value, whose return
convention moves to aligned `%retMFB` in plan-85-C (read-only source analysis; the
body edits + call-site shims land in C):

1. **`_mfb_make_error_result`** (`error_result.rs:97` `lower_make_error_result`) —
   inputs `ARG[0..4]` (filename, line, char, code, message\*); **returns the 4-register
   error `Result` in `RESULT_{TAG,VALUE,ERROR_MESSAGE,ERROR_SOURCE}_REGISTER` =
   `RET[0..3]`** → becomes **`%retMFB[0..3]`**. Body reads `_mfb_build_error_loc`'s `x0`
   return (`return_register()`, `:112`) into `RESULT_ERROR_SOURCE_REGISTER` — a helper-
   `%retC0`→`%retMFB3` consumption. Called from `builder_error_emission.rs:556`.
2. **`_mfb_build_error_loc`** (`error_result.rs:13` `lower_build_error_loc`) — inputs
   `ARG[0..2]`; **returns a single `ErrorLoc*` (Pointer) in `x0`** (`return_register()`,
   `:75`; `0` on OOM). Its body reads `_mfb_arena_alloc`'s `RET[0]` (tag, `:34`) / `RET[1]`
   (ptr, `:49-53`). Per plan-85-C §3 a single C-value-in-`rax` return is **`%retC0`**;
   its caller (`make_error_result:112`, `builder_error_emission.rs:178`) then reads
   `%retC0`. **Open for C:** whether `build_error_loc`/`arena_alloc` returns are `%retC`
   (rax-exact single value) or `%retMFB` (aligned) — decide in C from whether each is a
   genuine 1-value C-style return or an MFB multi-register result (`arena_alloc` returns
   a 2-value tag/ptr → `%retMFB[0..1]`; `build_error_loc` returns 1 pointer → `%retC0`).

Both helper **bodies** (in `error_result.rs`) and the trap-site emitters
(`builder_error_emission.rs`) are hand-emitted MIR, so C edits both sides in lockstep.
