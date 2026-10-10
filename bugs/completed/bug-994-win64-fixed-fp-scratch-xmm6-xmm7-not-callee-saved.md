# bug-994: Win64 functions write `xmm6`/`xmm7` through the fixed FP scratch tokens without saving them, so a caller's Float is silently replaced

Last updated: 2026-10-06
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Fixed (2026-10-10, bs18-agent-i)
Regression Test: `tests/codegen/codegen_win64_fp_callee_saved.rs`; Win64 xmm rows in `tests/runtime/rt_runtime_helper_callee_saved.rs` and `rt_foreign_entry_callee_saved.rs`

On `windows-x86_64`, a function can overwrite `xmm6` or `xmm7` without saving and restoring
them. Win64 makes `xmm6`–`xmm15` callee-saved, and the register allocator relies on that: it
keeps a caller's Float in `xmm6`–`xmm14` across a call. When the callee is one of these
functions, the caller's Float comes back holding whatever the callee left in the register.
Nothing traps and nothing reports an error. The value is just wrong.

The callees that do this are any compiled function whose body inlines a kernel that stages
values in the fixed scratch tokens `abi::FP_SCRATCH[6..7]` / `abi::VEC_SCRATCH[6..7]`. That
includes `math::sin`/`cos`/`tan` and the other SIMD float kernels, `math::pow`, and
`toFloat(Fixed)`/`toFloat(Money)`. On x86-64 these tokens become `xmm6`/`xmm7`. The frame's
save set only covers registers the allocator chose itself, so these writes are never saved.
The bug was found in `examples/canvas-demo` (`src/svg.mfb`, `FUNC parseSvg`). For an SVG with
a single `<circle>` and `viewBox="0 0 200 200"`, the `MUT Float` local `vw` was 200.00 before
`shapeRings(tag)` and 0.00 after it, but only on Windows.

**Correct behavior:** on every target, a call leaves every callee-saved register (Win64:
`rbx rbp rdi rsi r12–r15`, `xmm6–xmm15`) exactly as it was. So a Float the caller holds
across any call reads back unchanged. The reduced repro below must print
`10.00 20.00 30.00 40.00` three times on `windows-x86_64`, the same as on macOS and Linux.

References:

- `src/docs/spec/memory/06_native-calling-convention.md`: the callee-saved contract and the
  frame save set.
- `src/docs/spec/architecture/15_x86_64-instruction-set.md`: the xmm file, with `xmm15` as
  the reserved SSE scratch.
- `.ai/arch-abi.md` § Win64, and `.ai/codegen-invariants.md` (the bug-54/bug-550 save-set
  invariant).
- `bugs/completed/bug-350-x86-caller-saved-clobber-mask.md`: the per-ISA caller-saved mask.
  It is why SysV is immune here.
- `tests/runtime/rt_runtime_helper_callee_saved.rs` (bug-774): the sibling frame audit. Its
  Win64 table lists no xmm register.
- Found while testing `examples/canvas-demo` on box 206. The test agent's probe is
  `/tmp/svgprobe` (`win6.txt`: `svg/xcirc.svg vb=0.00,0.00,0.00,200.00`, printed in the
  order vx,vy,vw,vh).

## Failing Reproduction

A self-contained project. `mfb init`, then replace `src/main.mfb` with:

```
IMPORT io
IMPORT math
IMPORT os

FUNC viaSin(x AS Float, n AS Integer) AS Float
  MUT s AS Float = 0.0
  FOR i = 0 TO n - 1
    s = s + math::sin(x + toFloat(i))
  NEXT
  RETURN s
END FUNC

FUNC viaPow(x AS Float, n AS Integer) AS Float
  MUT s AS Float = 0.0
  FOR i = 0 TO n - 1
    s = s + math::pow(x + 1.5, toFloat(i) + 0.5)
  NEXT
  RETURN s
END FUNC

FUNC viaFixed(x AS Fixed, n AS Integer) AS Float
  MUT s AS Float = 0.0
  FOR i = 0 TO n - 1
    s = s + toFloat(x) * 3.0
  NEXT
  RETURN s
END FUNC

FUNC probe(which AS Integer) AS String
  LET k AS Float = toFloat(len(os::args()))
  MUT a AS Float = 0.0
  MUT b AS Float = 0.0
  MUT c AS Float = 0.0
  MUT d AS Float = 0.0
  MUT w AS Float = 0.0
  MUT i AS Integer = 0
  DO WHILE i < 2
    IF i = 0 THEN
      a = k + 10.0
      b = k + 20.0
      c = k + 30.0
      d = k + 40.0
    ELSEIF which = 1 THEN
      w = viaSin(k, 3)
    ELSEIF which = 2 THEN
      w = viaPow(k, 3)
    ELSE
      w = viaFixed(toFixed(k + 0.25), 3)
    END IF
    i = i + 1
  LOOP
  RETURN toString(which) & ": " & toString(a) & " " & toString(b) & " " & toString(c) & " " & toString(d)
END FUNC

FUNC main AS Integer
  FOR m = 1 TO 3
    io::print(probe(m))
  NEXT
  RETURN 0
END FUNC
```

```
mfb build --target windows-x86_64
scp build/<name>.exe test@192.168.1.206:C:/Users/test/b994/b994.exe
ssh test@192.168.1.206 'powershell -NoProfile -Command "& C:\Users\test\b994\b994.exe; $LASTEXITCODE"'
```

- Observed (box 206, compiler built from `90a91ab4a`):
  ```
  1: 10.00 -0.09 30.00 40.00
  2: 10.00 20.00 30.00 40.00
  3: 10.00 4294967296.00 30.00 40.00
  0
  ```
- Expected: `N: 10.00 20.00 30.00 40.00` for N = 1, 2, 3, and exit 0.

Only `b` is corrupted, and it is the value `probe` keeps in `xmm7`. In the
`mfb build --target windows-x86_64 --ncode` dump, `_mfb_fn_probe`'s frame saves
`["r12","r14","xmm6","xmm7","xmm8","xmm9","lr"]`, and the loop head loads a/b/c/d into
`xmm6`/`xmm7`/`xmm8`/`xmm9`. Case 3 is the clearest evidence: `4294967296.00` is exactly
2^32, the constant `load_numeric_as_double` puts in `FP_SCRATCH[7]` to divide a Fixed.
Case 1's `-0.09` is a sin-kernel intermediate. Case 2 is correct only because of how
`viaPow` happened to be allocated: its allocator homed values in `xmm6`/`xmm7` itself, so
both are already in its save set (`["r12","r14","xmm6","xmm7","xmm8","xmm9","lr"]`). That
is a latent case, not a safe one.

In the original demo, `parseSvg` keeps vx→`xmm8`, vy→`xmm9`, vw→`xmm7`, vh→`xmm6`. Its
`--ncode` loads `vb[0..3]` into those registers, and the loop head reloads them from
slots 1288/1296/1304/1312. On the circle path it calls `_mfb_fn_ovalRing`, which inlines
the `math::cos`/`math::sin` kernels and saves
`["r12","r14","xmm10".."xmm14","xmm6","xmm8","xmm9","lr"]`. `xmm7` is missing from that
list, so vw is lost while vh (`xmm6`, saved) survives. A `<path>` goes through
`parsePath`, which does not reach a trig kernel for M/L/Z, so it does not clobber vw.
The test agent's standalone `hold` probe did not reproduce because `hold` itself inlines
`math::cos`. The allocator therefore treated `xmm7` as busy there and never homed `vw` in it.

| Environment | build | Result |
| --- | --- | --- |
| box 206 Win11 x86_64 (real HW) | `--target windows-x86_64` | fails ✗ (cases 1 and 3) |
| box 2227 Alpine x86_64 musl | `--target linux-x86_64` | works ✓ |
| macOS aarch64 host | native | works ✓ |

On SysV, `_mfb_fn_probe`'s frame is `["r12","r14","lr"]` (linux-x86_64 `--ncode`). SysV
treats every xmm register as caller-saved (`X86_64RegisterModel::caller_saved(Fp)` =
`FP_REGS`), so floats are spilled around every call, and a callee is free to write any xmm
register. On AArch64 and RISC-V the scratch tokens become `d0`–`d7`/`v0`–`v7` and
`fa0`–`fa7` (`src/arch/riscv64/select.rs:map_fp_register`), which are all caller-saved.

## Root Cause

Two correct-looking pieces don't fit together on Win64:

1. **The allocator relies on xmm6–xmm14 surviving a call.**
   `src/arch/x86_64/regmodel.rs:Win64RegisterModel` sets `caller_saved(Fp)` =
   `WIN64_FP_CALLER_SAVED` (`xmm0`–`xmm5`) and `is_callee_saved` true for `xmm6`–`xmm15`.
   `src/codegen/engine/regalloc/analysis.rs:call_clobber_mask` therefore lets a value live
   across any `bl` (an `_mfb_fn_*`/`_mfb_ifn_*` call *and* every runtime helper, whose FP
   clobber is assumed to follow the PCS) sit in `xmm6`–`xmm14`.

2. **Fixed-register writes are never added to the save set.** The SIMD and math kernels
   stage lanes in the fixed tokens `src/target/shared/abi.rs:FP_SCRATCH` /
   `VEC_SCRATCH` (`%fscratch0..7`, `%vscratch0..7` → `d0..d7`/`v0..v7`).
   `src/arch/x86_64/select.rs:realize_x86_residual` maps `d/v/q{n}` to `xmm{n}` the same
   way on SysV and Win64, so indices 6 and 7 become the callee-saved `xmm6`/`xmm7`.
   `src/codegen/engine/regalloc/linear_scan.rs:run` builds `extra_callee_saved` from only
   two sources: `every_home` (colored homes) and `scratch_callee_saved` (free reload
   scratch). Its bug-54/bug-550 assertion checks only those same two sets, so a
   precolored physical write is invisible to it. The builder-side tracker
   `src/codegen/engine/regalloc/builder_registers.rs:mark_register_used` uses
   `abi::is_callee_saved`, which is hard-coded to AArch64 `x19`–`x28`. It never sees an xmm
   register either. `fp_physical_index` does model the tokens' *occupancy*, so the
   allocator never colors a value onto a live `%fscratch7`. Nothing models their
   *callee-saved write*.

A function writes `xmm6` or `xmm7` without saving it whenever it uses one of those tokens
and its allocator did not also pick that register as a home. Writers found by
`grep -nE "(FP|VEC)_SCRATCH\[(6|7)\]|v\[(6|7)\]" src`:

- `src/codegen/builtins/vector/builder_simd_float_math.rs`: `emit_sincos_reduce` and the
  other `lower_simd_float_unary`/`lower_simd_float_scalar` kernels (sin/cos/tan, exp,
  log, atan, ...), 147 hits. These are the scalar `math::` calls in ordinary code.
- `src/codegen/builtins/vector/builder_simd_math.rs:lower_simd_unary`, and
  `src/codegen/builtins/vector/builder_simd_fixed_math.rs:emit_fixed_sqrt_vector`.
- `src/codegen/builtins/math/gen_pow.rs:emit_pow_scalar`: `FP_SCRATCH[6]`/`[7]` (`sh`/`sl`).
- `src/codegen/engine/operators/builder_numeric.rs:load_numeric_as_double`:
  `FP_SCRATCH[7]` for every Fixed and Money → Float conversion.
- `src/codegen/runtime/canvas/vulkan.rs:emit_store_f32_from_integer`: `FP_SCRATCH[7]`,
  inside hand-built `_mfb_rt_*` helpers.

A frame audit of the demo's Windows `--ncode` (each function's xmm6–xmm15 `dst` writes
minus its `frame.calleeSaved`; the script was `/tmp/xmmaudit.py`) found **60 functions**
that write xmm6/7 without saving them. 57 miss `xmm7` only, among them `ovalRing`,
`arcPts`, `hold`, `owl` and `audio::render`. 3 miss both `xmm6` and `xmm7`: `approach`,
`_mfb_rt_canvas_canvas_vulkanPresentScene` and `_mfb_rt_canvas_canvas_vulkanDrawScene`.

## Goal

- Every Win64 function and runtime-helper frame saves every callee-saved xmm register
  (`xmm6`–`xmm14`, and any later-allocatable one) that its final instruction stream writes.
  The planned `codegen_win64_fp_callee_saved` audit checks this for every function in the
  dump.
- The reduced repro prints `N: 10.00 20.00 30.00 40.00` for N = 1..3 on box 206, and
  `examples/canvas-demo`'s `svg/xcirc.svg` parses to `vb=0.00,0.00,200.00,200.00` there.

### Non-goals (must NOT change)

- SysV, AArch64 and RISC-V code must stay byte-identical. Their scratch realizations are
  caller-saved, so their save sets must not change. Their `.ncodesum` goldens must not move.
- Do not make `xmm6`–`xmm14` caller-saved in `Win64RegisterModel`. That would hide the
  MFB→MFB symptom but still break the ABI toward Windows callers of MFB code, and it would
  force every Float to spill around every call.
- Do not change the kernels' math or their register choreography (≤1 ULP validated).
- Do not change the integer save set. `rbp` doubles as Win64 internal argument 8, and
  integer precolors are a separate audit (bug-698/bug-774).

## Blast Radius

`grep -rnE "(FP|VEC)_SCRATCH\[(6|7)\]" src` plus the `--ncode` frame audit above:

- Every compiled `_mfb_fn_*`/`_mfb_ifn_*` that inlines one of the writers listed above is
  **fixed by this bug**. An observed failure needs two things: the callee doesn't home
  that register itself, *and* the caller keeps a Float in `xmm6`/`xmm7` across the call.
  Seen: `parseSvg`→`ovalRing`, and the repro's `viaSin`/`viaFixed`.
- Hand-built runtime helpers (`_mfb_rt_canvas_canvas_vulkanPresentScene`,
  `_mfb_rt_canvas_canvas_vulkanDrawScene`) write xmm6/7 with an empty FP save set. They are
  **fixed by this bug**, since the fix must cover `finalize_vreg_body_with_locals` bodies as
  well. Their only caller, `_mfb_ifn_canvas_5FrenderVulkan`, holds no xmm across calls today
  (frame `["r12","r14","lr"]`), so this is latent.
- `xmm15`, the encoder's reserved SSE scratch (`src/arch/x86_64/encode/emitter.rs`: fcmp,
  fneg, fma, roundsd, ...), is written inside instruction encodings and never saved. It is
  **latent and out of scope**. The allocator never homes a value in `xmm15`, so no MFB caller
  is affected. Only a *foreign* caller of MFB code could see it (a Win32 callback whose
  body reaches FP code). `src/target/win_x86_64/app/mod.rs:win64_nonvolatile` notes that its
  wndproc arms use no xmm. File it separately if a callback ever runs FP code.
- `tests/runtime/rt_runtime_helper_callee_saved.rs:abi("windows-x86_64")` lists only
  integer registers, so its audit cannot see this class. It is **extended by this bug**
  (Phase 1).
- SysV / AArch64 / RISC-V: **unaffected**. Every scratch realization (`xmm0–7` SysV,
  `d0–d7`/`v0–v7`, `fa0–fa7`) is caller-saved there.

## Fix Design

Extend the bug-54 save-set rule from "every callee-saved register the allocator *chose*"
to "every callee-saved register the final stream *writes*". After the FP pass in
`src/codegen/engine/regalloc/mod.rs` (where `int`/`fp.extra_callee_saved` are merged),
scan the output instructions. For every FP register a definition writes (or, more simply,
every FP operand), whether a concrete name (`xmm7`) or a still-unrealized token
(`%fscratch7` in a hand-built helper body), add it to `extra_callee_saved` if
`model.is_callee_saved` is true. Resolve a token through the backend's realization, not
by index arithmetic: index 6 is `ft6` on RISC-V and `d6` on AArch64, both caller-saved, so
those targets stay byte-identical. `finalize_frame` already saves an `xmm` save-set entry
with a 128-bit `movups` (`vreg_frame.rs:is_simd128_callee_saved`), so no frame code changes.
Promote the bug-550 assertion to check this wider set.

Most of the risk is in the realization lookup: the tokens reach the allocator realized for
builder bodies and unrealized for hand-built helpers. The expected golden shift is
`windows-x86_64` `.ncodesum`/`.ncode` only, for fixtures that use a math kernel or a
Fixed/Money→Float conversion: one 16-byte save and restore of `xmm7` (and sometimes `xmm6`)
per affected frame. A diff on any other target is a bug in the fix.

Rejected alternatives:

- Remap `FP_SCRATCH`/`VEC_SCRATCH[6..7]` to volatile registers on Win64. Only `xmm0`–`xmm5`
  are volatile, and the kernels need all 8 slots live at once. `xmm15` is already the
  encoder's scratch.
- Mark the kernel-using functions' calls as clobbering every xmm (a caller-side fix). This
  is wrong toward Windows callers and penalizes every caller.

## Phases

### Phase 1: failing test + audit (no behavior change)

- [ ] Add `tests/codegen/codegen_win64_fp_callee_saved.rs` (with a `[[test]]` entry in
      `Cargo.toml`). Modeled on `rt_runtime_helper_callee_saved.rs`, it uses
      `common::build_ncode(&project, "windows-x86_64", ...)` on the repro program and asserts
      that every function in `ncode["functions"]` lists every `xmm6`–`xmm14` it names in an
      instruction field in `frame.calleeSaved`. Also add a targeted assertion that
      `_mfb_fn_viaSin` and `_mfb_fn_viaFixed` save `xmm7`. The test needs no Windows box.
      Confirm it fails today (`viaSin`/`viaFixed` are missing `xmm7`).
- [ ] Add `xmm6`–`xmm15` to `rt_runtime_helper_callee_saved.rs:abi("windows-x86_64")`, and
      confirm it fails only for the reason documented here.

Acceptance: both tests fail for the documented reason (`xmm7`, and in helpers `xmm6`,
written but not saved). The blast-radius list above has a verdict for every site.
Commit: —

### Phase 2: the fix

- [ ] Widen the save set in `src/codegen/engine/regalloc/mod.rs` / `linear_scan.rs:run` as in
      Fix Design (concrete and token FP operands, realized per backend), and widen the
      bug-550 assertion to match.
- [ ] Check that the hand-built helper path (`util/vreg_frame.rs`, which goes through
      `finalize_frame` with `outcome.extra_callee_saved`) picks up the vulkan helpers' xmm6/7.

Acceptance: the Phase 1 tests pass. linux-x86_64 / AArch64 / RISC-V `--ncode` are
byte-identical. Nothing listed in Non-goals changed.
Commit: —

### Phase 3: regenerate expected outputs + full validation

- [ ] Regenerate the `windows-x86_64` goldens that move, and inspect one fixture's diff to
      confirm it is only a prologue/epilogue `xmm6`/`xmm7` save/restore. Run the full suite
      before re-baselining.
- [ ] Run the full suite and the artifact/byte-identity gates (`.ai/testing-gates.md`).
- [ ] Re-run the reduced repro on box 206 (all three lines correct, exit 0), and
      `examples/canvas-demo` with `svg/xcirc.svg` headless (`MFB_WINAPP_HEADLESS=1`), where
      vw must read 200.00.

Acceptance: the full suite is green, only the intended Win64 golden deltas appear, and the
repro passes on box 206.
Commit: —

## Validation Plan

- Regression tests: `tests/codegen/codegen_win64_fp_callee_saved.rs` (new) and the extended
  `tests/runtime/rt_runtime_helper_callee_saved.rs` Win64 row.
- Runtime proof: the reduced repro on box 206, run through PowerShell so `$LASTEXITCODE` is
  read, plus the canvas-demo `xcirc.svg` viewBox check.
- Doc sync: `src/docs/spec/memory/06_native-calling-convention.md` (the save-set rule now
  covers fixed-register writes on Win64), and a Win64 note in `.ai/arch-abi.md` saying the
  FP scratch tokens 6/7 realize to callee-saved xmm registers.
- Full suite: `cargo test` plus the golden/byte-identity gates.

## Open Decisions

- Scope of the stream scan: FP class only (recommended; it stays inside this bug and leaves
  the integer `rbp`-as-arg-8 convention alone), or every class.
- `xmm15` for foreign callers: leave it latent (recommended; no FP code runs in a callback
  today), or save it in every Win64 function that the encoder lets write it.

## Summary

The risk is in the fix's realization lookup (tokens reach the allocator realized for
builder bodies but raw for hand-built helpers) and in confirming the Win64-only golden
delta. The kernels, the allocator's coloring, and every non-Win64 target are untouched.

## Resolution

`linear_scan::run` (FP class only) now adds to `extra_callee_saved` every callee-saved
register the INPUT stream writes by fixed name (`fixed_callee_saved_writes`), and the
bug-54/bug-550 assertion covers that set. Two spellings are resolved: a selected builder
body's concrete name (`xmm7`), asked of the target model directly; and a hand-built
helper's unrealized `%fscratchN`/`%vscratchN` token, mapped through its occupancy index to
the target's allocatable register. `finalize_frame` already saved an `xmm` entry with a
16-byte slot, so nothing else moved.

Correction to the Fix Design: mapping EVERY physical def by index (the first cut) was
wrong on RISC-V. The artifact gate showed `audio`/`vector` `linux-riscv64` sums moving:
13 frames gained `fs0`–`fs4` saves although the final stream names none of them. The
RISC-V pre-allocation stream still carries AArch64-numbered `vN` lane names its
scalarizer rewrites later, so an index is not a register there. Only tokens are mapped
by index now; a concrete name must be the target's own. After that the gate's new diffs
are Windows only.

`xmm15` stays unsaved (Open Decision, as recommended). Open Decision "scope": FP only.

Other ABIs audited: `sysv_frames_save_no_xmm` (no SysV frame saves an xmm), and the
artifact gate shows no AArch64/RISC-V/SysV byte change, so no AArch64 `d8`–`d15` or
RISC-V `fs*` register is written by fixed name without a save in the covered corpus.

### Proof

- RED (ee9d31beb): `cargo test --release --test codegen_win64_fp_callee_saved` failed 2 of
  3 (`_mfb_fn_viaSin`/`_mfb_fn_viaFixed`: `xmm7`; `_mfb_rt_canvas_canvas_vulkan{Present,Draw}Scene`:
  `xmm6`,`xmm7`). After the fix: 3 passed.
- Box 206 (Windows 11), the reduced repro, exit code read through `$LASTEXITCODE`:
  main f6b4099c9 printed `1: 10.00 -0.09 30.00 40.00` / `2: 10.00 20.00 30.00 40.00` /
  `3: 10.00 4294967296.00 30.00 40.00`, exit 0; the fix prints `N: 10.00 20.00 30.00 40.00`
  for N = 1..3, exit 0. Same output on the macOS host.
- Not run: `examples/canvas-demo` `svg/xcirc.svg` on 206.

## Golden impact

`scripts/artifact-gate.sh target/release/mfb all`: beyond the bug-982/983 diffs, exactly
`byte-identity/{audio,general,vector}` `.windows-x86_64.ncodesum` (frames that inline a
math kernel or a Fixed/Money→Float conversion gain an `xmm7`, sometimes `xmm6`, save).
`syntax/app/app-mouse-surface` `.windows-x86_64.app.ncodesum` (already moved by bug-982)
also gains the Vulkan helpers' saves.
