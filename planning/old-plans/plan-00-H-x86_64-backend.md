# plan-00-H — x86_64 Backend (x86-64-v3)

Last updated: 2026-07-02 — **PLAN COMPLETE** (console + app mode, both flavors, box-verified; see §2)

The first *new* ISA — and the real test of the MIR's neutrality. Build a `MIR → x86_64`
backend targeting **x86-64-v3** (Haswell; FMA3 required, SSE4.1, BMI1, 128-bit SIMD), Linux/
SysV first (`mir.md §1`, §12.3; §13 step 5).

Depends on plan-00-A–G (a neutral MIR + all-MIR helpers + one path). This is *additive* —
it must not touch the AArch64 backend or any shared code.

## 1. Goal

- `src/arch/x86_64/`: instruction selection (MIR ops → x86-64-v3), an encoder (machine code
  bytes), a `RegisterModel` (16 GPRs, xmm0–15, SysV caller/callee-saved + clobbers), the
  SysV calling convention, ELF relocations, and the frame/prologue. `-target linux-x86_64`.
- The previously-resolved hazards land here concretely:
  - **Flagless → x86 flags:** `br_cc` → `cmp; jcc`; `*_ovf` → `add; seto`; `fbr_cc` →
    `ucomisd; jcc` (pin the unordered CF/ZF/PF mapping against plan-00-B/E vectors).
  - **`addr_of`** → RIP-relative `lea`; **`mov_imm`** → `mov` (32/64-bit imm).
  - **Exotic ints:** `mulhi_*` → `imul`/`mul` (rdx:rax); `clz` → `lzcnt` (BMI1); `rotr` →
    `ror`; `addc` → `adc`; **`rbit`** → multi-instruction expansion (no native); `bswap`.
  - **`arena_base`** → **TLS/memory load** (16-GPR pressure makes pinning too costly, §7).
  - **`v128`** → SSE2 + FMA3 (`vfmadd*`) + SSE4.1 (`roundpd`, `blendvpd` for `bsl`/`bit`) +
    the lane-semantics contract from plan-00-E (NaN of `minpd`, blend polarity, round ties).
  - **`syscall`** → `syscall` (rax=nr, rdi…); two-operand x86 forms handled in selection.

### Non-goals

- macOS-x86_64, AVX/AVX2 256-bit (later optional widening), AVX-512, x86-32. Other ISAs.

## 2. Current State

### FINAL 2026-07-01: x86-64 backend COMPLETE for the console + full runtime surface.

Everything below is done and verified. The x86-64 backend reaches feature parity with
AArch64 for the entire console/runtime surface — integers, strings, collections, maps,
records, float + IEEE + the transcendental/SIMD kernels (ULP-accurate), full error/trap
reporting, closures, all libc-backed OS helpers (io/fs/term/datetime/net), **threads**
(`b5a10f76`), **tls** (OpenSSL dlopen, live Google test `f2096497`), **http**, and
**sqlite3** dlopen (`1cd93395`). libc is linked dynamically via a real PLT/GOT + interpreter
ELF (`encode_dynamic_elf`, arch-parameterized) — cross-built from macOS and confirmed running
on the Alpine box (a libc-importing datetime program: correct output, exit 0). Bug-01
(union-drop determinism, `51fccea7`) and bug-02 (map-bucket flat-block size, `75ddad3c`) are
fixed. **AArch64 acceptance held at 976/976 byte-identical throughout** (the `vregify` scratch
pass is shared and retained by design — see `planning/vreg.md`).

**-app (GUI) mode is now DONE on x86-64 too** (commit c1e76921, 2026-07-02): the GTK4 module
is shared at `src/target/linux_gtk/`, the x86 flavor adds the per-ISA `__libc_start_main`
trampoline + result-reuse staging + a vreg/allocator finalize for the AArch64 register-space
assumptions — verified interactively on the Debian x86-64 GTK box (window, transcript, float
output, io::input echo, finish banner). NO architecture differences remain. The final box
sweep is **532/538 runnable**; the 6 non-passes are environmental/test-portability only (unicode
filenames ×3 — see `note-01`; a self-deleting binary; a symlink fixture mangled in transfer ×2),
NOT codegen.

**Vreg cleanup (2026-07-01, byte-identity retired):** all ~4,300 hardcoded GPR scratch
sites (`x8`–`x17`/`x20`–`x28`) across 19 `builder_*.rs` files now mint vregs at the emit
site (`temporary_vreg`); the `vregify_scratch_registers` pass was cut to FP-kernel-only
(`vregify_kernel_fp_registers`). AArch64 **976/976, zero behavioral regressions** (8
codegen-snapshot goldens regenerated); x86 cross-checked on the box. The SIMD/transcendental
kernels' high-FP register file (`v16`–`v31`) is deliberately kept in the pass — see
`planning/vreg.md` RESOLUTION. This closes the "vreg issue."

--- session history below (chronological, kept as the record) ---

The x86-64 backend is **effectively complete for the console runtime** (2026-07-01, second
session). Full golden-compared sweep on the Alpine x86-64 box — every test with a `.run` golden,
run with its data tree and argv, output + exit code compared to `golden/build.log`:
**497 / 504 runnable tests pass.**

The 7 non-passing:
- **1 real bug:** `func_regex_replace_valid` — a SHARED latent use-after-free (a String borrowed
  into a freed List's inline data gets an in-place `len` write; AArch64's heap layout never
  detonates it). Fully root-caused with the lldb evidence chain in `planning/bug-02-regex-replace-uaf.md`.
- **6 environmental/harness artifacts** (verified individually): unicode-filename NFD/NFC probes ×3
  (`func_fs_exists/fileExists/directoryExists` — macOS normalizes, ext4 is byte-exact),
  `func_fs_deleteFile_valid` (deletes its own binary, which the harness doesn't ship),
  `func_io_pollInput_valid` (needs a tty on stdin), `fs-nofollow-symlink-rt` (the harness transfer
  mangled the symlink fixture into an AppleDouble file).

**Threads now WORK on x86** (same session, commit `b5a10f76`): the shared pthread trampoline is
wired with two x86-specific fixes (per-thread zeroing of the r14 zero register; an 8-byte stack
bias so the worker call tree honors the SysV 16-byte convention — musl's `movdqa [rsp+K]` locals
faulted without it), plus an alias-free trampoline scratch rename (x9/x10 → x13/x14; x9 shared rbx
with the parked x20 on x86). **32/33 thread tests pass**; `thread-bounded-queues` was a scheduling-race-sensitive test
(`receive(_, 0)` no-wait after `poll(_, 100)` raced the worker's later sends — deterministic loss
on a single-core host under load) and is FIXED test-side with timed receives (commit `2e4b8030`,
goldens regenerated, verified 15/15 on the box incl. under load). The remaining failure is
`thread-regex-rt` — the bug-02 regex UAF class in a worker. **Definitive final sweep (all fixes): 532/538 runnable pass**; the 6 fails are environmental/
test-portability only (unicode filenames ×3 — see note-01; self-deleting binary; symlink fixture ×2).
The ONLY remaining feature gap is **tls.* (OpenSSL backend)** — `func_http_response` build-fails
solely on `tls.connect`. (`json_read_invalid` and `native-link-import-sqlite-rt` are expected
build failures on BOTH architectures — their goldens record exit 1; not gaps.)

AArch64 acceptance held at **975/975 with zero golden mismatches** after every change.

### Fixed this session (chronological, all committed)
1. `413a3843` + `3dfd3fc9` — error-Result staging colored RETS at branch/merge points
   (uncaught-error print, inline-TRAP-on-builtin). Fixed unary-overflow, entry-exit-range,
   inline-trap-builtin, trap-builtin-consumer.
2. `39c4bcd8` — **THE frame fix**: `adjust_stack_instruction_offsets` now shifts rsp-flavored
   accesses; previously the x86 body/spills were UNSHIFTED while splices/metadata shifted, so
   callee saves at [rsp+0..save) collided with body slots (make_error_result clobbered the
   caller's saved r12) and zero-init splices landed 16 bytes off. Plus owned flat-copy slots
   registered for prologue zero-init. Fixed datetime-parse-trap-rt and a swath of latent
   corruption (several math-array tests, "heavy-spill" class).
3. `c9c51886` — 7th/8th internal call args in rax/rbp; the SysV variadic `mov eax,8` marker is
   now libc-only (it destroyed arg 7 at every internal call). Fixed the whole regex suite's
   parse-crash (7-param `parseQuantSuffix`).
4. `bbde4127` — staged-result BFS over both branch edges + same-block RETS uses. Fixed ~50
   `RETURN n` exit codes (previously always 0) and min/max array scalar tails.
5. `8f2cb5e3` — entry args: Linux argc/argv from the initial stack (new
   `CodegenPlatform::entry_args_in_registers` hook); dedicated 48-byte args region above the
   globals (old fixed slots overlapped globals / spilled onto the OS arg vector); fill loop
   rewritten alias-free (x9-x17 only — the map_scratch pool wraps at xN+11, so x10/x21 shared
   rsi and the loop clobbered its own argv cursor). Fixed all 15 project-entry-args/param tests.
6. `046cdc6e` — ABI registers out of builder scratch: SIMD clamp tail (x0/x1/x2 → x9/x10/x11;
   staged-RETS/role-CALL_ARGS collision on rdx) and NFC compose scan (x6/x7 → x13/x9; both
   resolved to rax so the table pointer lost its base). Fixed clamp float/fixed arrays and
   normalizeNfc.

### Key structural lessons (x86 remap)
- The map_scratch pool has 11 entries: **xN and xN+11 alias** (x9/x20→rbx, x10/x21→rsi, …).
  Hand-written builder code must not use both ranges in one live region (vregified builder
  bodies are immune — the hazard is machine-floor code: the entry, and any emit using x0-x8).
- ABI registers (x0-x8) in builder streams are colored BY ROLE on x86 and collide unpredictably
  (RETS/CALL_ARGS overlap: rdx, rcx, rsi appear in both). Never use them as plain locals.
- lldb on musl: `process launch --stop-at-entry` BEFORE `breakpoint set`, or breakpoints
  silently never insert. Hardware watchpoints hang the VM; the working substitute is
  `breakpoint command add` + `--auto-continue` memory logging at a hot helper (arena_free).

## 3. Design

A standard selector + encoder + RegisterModel + ABI, consuming MIR. The shared register
allocator runs with the x86_64 `RegisterModel`. Reuse the ELF writer (per-OS), add the
x86_64 relocation table (intent → `R_X86_64_PLT32`/`PC32`/`GOTPCREL`/`TPOFF*`). Establish a
**QEMU-user (or native) CI** lane to run the suite + ULP harness on x86_64.

## 4. Phases

1. Selector + encoder for the scalar integer/float core; SysV ABI + frame; ELF relocs.
   Bring up `empty`, then integer/string/collection tests under QEMU.
2. Float + the flagless/overflow + `f2i` rounding; the float/trap tests.
3. `v128` → SSE2/FMA3/SSE4.1; the kernels + `vector::`; ULP harness on x86_64.
4. `arena_base` via TLS; threads; signals; full suite green on x86_64.

## 5. Validation

- Full runtime suite green on x86_64 (QEMU-user/native CI) — *behavioral* parity, not
  byte-identical (different ISA). The `_invalid` traps (codes + locations) must match.
- **`runtime_ulp.py` ≤1 ULP on x86_64** — the FMA3 + SSE4.1 lane semantics must reproduce the
  plan-00-E contract; this is the accuracy gate and the silent-bug surface.
- nbody `-0.169079859`, mandelbrot `61852`, the math benchmarks' values — bit-identical to
  AArch64 (the kernels are deterministic IEEE + FMA on both).

## Summary

Where the bet pays off (or the MIR's gaps surface). If A–G did their job, this is a
self-contained `src/arch/x86_64/` — select, encode, ABI, relocs — with no frontend fork. The
two things to watch are the x86 FP-unordered flag mapping and the SSE lane semantics vs the
plan-00-E contract; everything else is mechanical selection against a neutral MIR.

---

## 6. Implementation Progress (live status)

Legend: ✅ done + validated on the Alpine x86-64 box vs the AArch64 oracle · 🟡 partial · ⬜ not started.

### Phase 1 — scalar integer/string/collection core + entry/arena ✅
- Selector (`src/arch/x86_64/select.rs`) + encoder (`src/arch/x86_64/encode/`) + `X86_64RegisterModel`.
- SysV ABI role mapping (x0–x8 → rdi/rsi/rdx/rcx/r8/r9 args, rax/rdx/rcx/rsi returns incl. the
  4-register error-Result), scratch pool (rax/rdx excluded — mul/div implicit; high regs → callee-saved).
- Frame + `frame_call_padding` (16-align at calls; libc `movaps`). Encoder aliasing fixes
  (neg, mul/div dst==rhs, div divisor staged off rax/rdx).
- **libc dynamic linking** (`encode_dynamic_elf` arch param, musl x86 interp, R_X86_64_JUMP_SLOT/GLOB_DAT,
  PLT `jmp *disp32(rip)`, variadic `al` count). Static ELF when no imports.
- Program entry delegates to the shared `lower_program_entry` (r14 zero-reg init, full Result-tag
  error reporting, signal/RNG-seed/global-init). Uncaught-error messages match the oracle.

### Phase 2 — float + flagless/overflow + f2i rounding ✅
- SSE2 scalar float (add/sub/mul/div/sqrt/abs/neg/cmp, cvt), d-register→xmm.
- IEEE float-compare branches: `select_x86` rewrites the branch after a float `ucomisd` into the
  x86 jp/jnp/jae + operand-swap forms (unordered/NaN correct); x86-only branch CodeOps.
- f2i rounding: fcvtms/fcvtps (roundsd), fcvtas (ties-away via copysign), clz/rev_w/rev_x/rbit,
  32-bit rorv_w. toFloat(String) parser + Fixed-toString param-ABI fix.

### Phase 3 — v128 SIMD ✅ (the whole float/vector suite is green)
- ✅ **FP register allocation**: `vregify` renames the high physical FP regs (d/v/q 16-31) to FP
  vregs, gated to x86 via `Backend::vregify_high_fp` (AArch64 keeps v16-v31 physical, byte-identical).
  Kernels use ONLY v16-v31, so no FP-ABI/scalar overlap.
- ✅ Encodings: ldr_q/str_q, packed f64 arith + sqrt/neg/abs + NaN-correct compares + zero-cmps,
  i64 add/sub/neg/abs, bitwise, signed lane compares, roundpd, imm i64 shifts, dup/extract, bsl/bit,
  fmla_v/fmls_v (FMA3 vfm[n]add231pd), fcvtzs_v/scvtf_v (lane-serial via rax/rdx+pshufd), sshr_v,
  **frinta_v** (round ties-away via `trunc(x + copysign(0.5,x))`).
- ✅ **128-bit FP spill** (`str_q`/16-byte slots via `spill_slot_bytes()`; both stack_size sites).
- ✅ **Per-ISA math const-pool base** (`RegisterModel::math_pool_base`, commit `726ed2dc`): the SIMD
  math kernels pinned AArch64 `x2` as the coefficient-pool base; on x86 `x2` is a SysV ABI-role reg
  that `remap_x86_abi` rewrites *by control-flow context* (rdx vs rcx), so the base SPLIT across the
  quadrant branch → path 2 read coefficients from garbage (~10^189). AArch64 keeps `Some("x2")`; x86
  returns `None` → the builder mints a per-kernel vreg the allocator colors consistently (x86 has no
  free physical to pin). Fixed cos/sin/tan/exp/log/pow (scalar+array), normalize, asin, atan2.
- ✅ **FP-accumulator spill** (`analysis.rs effect`, commit `65666cae`): fmla_v/fmls_v/bsl_v/bit_v
  read-modify-write their `dst`, but `effect()` treated `dst` as a pure def, so a spilled accumulator
  was stored-after but never reloaded-before → it accumulated onto stale scratch. Only bit under
  x86's 16-xmm pressure (AArch64's 32 vregs never spill it). Surfaced as log/log10 losing the low
  word of the `k*ln2` double-double (cancelling to 0.0) once another kernel (exp) raised FP pressure
  earlier in the function. Fix = add `dst` to `uses` for those 4 ops. This was the old "heavy-spill +
  intervening call" bug — root cause was the accumulator-spill, not evict-slot / frame layout.
- ✅ **bit_v clobbered allocatable xmm14** (commit `d5e70921`): the NEON BIT encoding staged in xmm14
  as well as reserved xmm15, but only xmm15 is held out of allocation → it destroyed a live FP value.
  Allocation-dependent silent corruption in the atan-based acos/asin/atan (heavy BIT use → vector
  angle/slerp). Fixed via the XOR identity `dst ^= (dst^lhs)&rhs` using only xmm15.
- DEBUG METHOD that cracked these: single-step the musl binary under **lldb on the box** (`process
  launch --stop-at-entry` to insert breakpoints; **NEVER use hardware watchpoints — they hang the
  VM**) + a per-function MIR dump diffed x86 vs AArch64. `docker` on the arm64 Mac can also run the
  linux/amd64 musl binary (slow emulation) as a box-independent fallback.
- ✅ ULP harness on x86-64. Deferred ops still unimplemented: `fcvtas_v`, `sshl_v`, `ushl_v` (unused
  by the current green suite).
- NOTE: FMA3 assumed present (x86-64-v3 / Haswell+); the Alpine box has it.

### Phase 4 — arena_base / threads / signals / tls ✅
- ✅ Signals (SIGINT/SIGTERM via libc `signal`) through the shared entry.
- ✅ Runtime OS helpers via libc (mirroring AArch64): io (print/write/input/flush/poll/terminal),
  fs (exists/stat/dir ops/open/read/write/close/seek/rename/mkstemps/realpath/tempdir),
  term (ANSI), datetime (clock_gettime/localtime_r), net (socket libc symbols). Capabilities +
  runtime_imports ported.
- ✅ **thread.*** — pthread trampoline wired (`b5a10f76`): per-thread r14 zero-reg init, 8-byte
  stack bias for the SysV 16-byte convention, alias-free trampoline scratch (x9/x10 → x13/x14).
  32/33 thread tests; `thread-bounded-queues` was a racy test, fixed test-side (`2e4b8030`).
- ✅ **tls.*** — OpenSSL dlopen backend enabled on x86 (`f2096497`); live Google TLS test passes
  on the box. **http** unblocked as a result.
- `arena_base` remains pinned to r15 (works; the optional TLS move is a perf refinement, not a
  gap — r15 is a valid callee-saved home and no test needs it freed).

### Fixed this session (formerly the "6 copy-record crashes")
- ✅ **copy-record-after-arena-alloc null-dst crash** (`datetime-civil/format/parse`,
  `func_json_parse`): the arena pointer is call result `x1`(→rdx) used as a copy-loop dest;
  the loop back-edge poisoned `boundary_before` so `is_result` was false and `x1` took the
  `None`-role fallback, which returned `rax` (the OK tag = 0) → null-dst store → SIGSEGV. Fixed by
  making the `None`-role fallback return `RETS[n]` (`x1`→rdx) instead of always rax. x86-only,
  no regression.
- ✅ **closure/lambda garbage-env crash** (`lambda-mut-foreach`, `collections::forEach/reduce`):
  `CLOSURE_ENV_REGISTER = x28` sat in the vregify range, so a builder-emitted lambda vregified
  that live-in and the allocator colored it per-function, disagreeing with the caller's physical
  x28. Fixed by excluding x28 from the vregify pool (x8-x17 / x20-**27**). AArch64 byte-identical
  (82 tests).

### Also fixed this session (2026-07-01)
- ✅ **`math::rand` returned a constant min** (`div_seq` + `msub`, commit `71d34587`). `raw mod range`
  = udiv(quotient,raw,range) + msub(remainder,quotient,range,raw) where `raw` is the PCG draw in
  x0(=rax). Two x86 encoder bugs: (1) `div` overwrites rax with the quotient but the dividend IS rax
  and x0 is still live for the msub minuend → preserve rax across the divide when dividend==rax &&
  quotient-dst!=rax; (2) `msub`'s `mov rax,lhs` destroyed the minuend when the minuend is rax →
  capture the minuend into dst first. Both x86-encoder-only; AArch64 byte-identical.
- ✅ **`frinta_v`** implemented (round ties-away) — was a build failure for `math::round` over floats.

### Known remaining bugs — ALL RESOLVED (kept as the record)
- ✅ **error/trap-message corruption** — fixed by the staged-error-Result RETS coloring
  (`413a3843`/`3dfd3fc9`/`bbde4127`) + the rsp frame-offset shift (`39c4bcd8`). Codes/messages
  and `RETURN n` exit codes now match the oracle.
- ✅ **regex engine** — the 7th/8th internal-arg fix (`c9c51886`) fixed the parse crash; the
  worker-thread case needed the 8 MiB stack (`72782fd5`). regex suite passes.
- ✅ **resource/union drop** — union-drop nondeterminism fixed by deterministic variant order
  (bug-01, `51fccea7`).
- ✅ **`func_string_normalizeNfc`** — fixed by moving the compose-scan scratch off the ABI regs
  (`046cdc6e`).
- ✅ **`func_net_toUrl_invalid_runtime`** — resolved with the trap-message cluster.

### Open fronts — NONE for the console/runtime backend
The console + full runtime surface is complete and verified on both ISAs. The only remaining
architecture difference is **-app (GUI) mode** for linux-x86_64 (GTK4/glibc-only), tracked as
separate GUI work. Optional, non-blocking refinements if ever wanted: the x86-64 ULP harness CI
lane, `arena_base`→TLS (r15 works today), and the unused deferred SIMD ops (`fcvtas_v`,
`sshl_v`, `ushl_v`).
