# x86_64 Backend — Scoping Document

Last updated: 2026-06-29

Pre-plan scoping for adding a Linux x86_64 native backend. This is **not** a
committed implementation plan; it exists to choose the instruction-layer
strategy (A vs B below) and size the work before a `planning/plan-NN-*.md` is
written. Decisions captured from the kickoff discussion:

- **First target OS:** Linux x86_64 (ELF / SysV ABI / `syscall`). Closest sibling
  to the existing `linux_aarch64` backend; Apple silicon makes macOS-x86 the less
  natural pick.
- **Math kernels:** port NEON → SSE/AVX to preserve the zero-libm,
  bit-identical-accuracy property on x86 (not a libm fallback).
- **Strategy:** undecided — this doc compares A and B and recommends.
- **Future ISA:** RV64 (RISC-V 64-bit) is planned after x86. This is a second
  future backend, which materially shifts the strategy choice — see §5a.

It complements:

- `./mfb spec memory` (value/record/collection layout the backend must preserve byte-for-byte)
- `planning/plan-03-transcendental-kernels.md` and the `plan-01-libm-kernels` work (the NEON math kernels in scope for the SSE/AVX port)

---

## 1. Where the architecture boundary sits today

```
source → lexer → AST → resolver → typecheck → monomorph → IR
   → NIR              src/target/shared/nir/      ISA-neutral, OS/target-tagged
   → shared/code      ~72k lines                  instruction selection → CodeOp
   → CodeInstruction { op: CodeOp, fields: Vec<(&'static str, String)> }
   → peephole         shared/code/peephole.rs
   → regalloc         shared/code/regalloc/       ISA-neutral core + per-ISA RegisterModel
   → encode/emitter   src/arch/aarch64/encode/    CodeOp → machine bytes
   → plan.rs          target/<os>_<arch>/         Mach-O / ELF image
```

### Already abstracted for a second ISA (the leverage)

- **`RegisterModel` trait** — `src/arch/aarch64/regmodel.rs`. The linear-scan
  allocator core (`shared/code/regalloc/`) queries register facts (allocatable
  sets, caller/callee partition, call-clobber masks, spill/reload/move emitters)
  through this trait. Its doc comment explicitly anticipates
  `src/arch/x86_64/regmodel.rs`. **Reusable as-is**; x86 supplies a new impl.
- **`CodegenPlatform` trait** — `shared/code/types.rs`. OS-level seam:
  `emit_program_exit`, `emit_write`, `emit_poll_input`, termios layout, syscalls.
  Per-OS, but the seam exists and `linux_*` already implements it.
- **Encoder is isolated** — `src/arch/aarch64/encode/` (~1.8k lines) is a clean
  `CodeOp → bytes` layer. A `src/arch/x86_64/encode/` sibling is mechanical,
  well-bounded work.
- **ELF image writer is largely shared** — `linux_aarch64/plan.rs` delegates to
  `shared/plan`. Needs an x86 `e_machine` (EM_X86_64 = 62) and the `R_X86_64_*`
  relocation kinds, but the section/segment/PLT/GOT machinery is reusable.

### NOT abstracted (the cost center)

- **`CodeOp` is the AArch64 instruction set, not a virtual ISA.** 131 variants
  (~42 FP/SIMD). The entire ~72k-line `shared/code` layer constructs `CodeOp::*`
  via 103 `abi::*` helpers (`abi::store_u64`, `abi::compare_immediate`, …).
  "Shared" means *shared across OSes that are both aarch64* — it is **not**
  ISA-neutral. Operands are strings (`"x9"`, `"d0"`) on a load/store RISC model.
- **No fixed-register / precolor constraint mechanism** in the allocator. AArch64
  never needed one. x86 `div`/`idiv`/`mul` pin `rdx:rax`; variable shifts pin
  `cl`. This is a genuine allocator extension regardless of strategy.
- **NEON math kernels** — `builder_simd_math.rs` (775), `builder_simd_float_math.rs`
  (1239), `builder_simd_fixed_math.rs` (295), `builder_pow.rs` (737),
  `builder_math.rs` (1146), `simd_kernel_coeffs.rs` (101) ≈ **4,300 lines** emit
  NEON directly. SSE/AVX is a different register file and encoding.
- **AAPCS in `abi.rs`** (759 lines, 103 fns): x0–x7 args / x0 ret vs SysV
  rdi,rsi,rdx,rcx,r8,r9 / rax ret. Needs an x86 sibling.
- **Syscall model:** `Svc` (`svc #0`) vs x86 `syscall`; different numbers and the
  `rcx`/`r11` clobber by `syscall` itself.

---

## 2. The two strategies

### Strategy A — Reinterpret `CodeOp` as a virtual RISC ISA

Keep all 72k lines of builders untouched. Treat the emitted `CodeOp` stream as a
quasi-RISC virtual ISA and write an **x86 encoder** that pattern-matches it onto
x86-64, using a reserved scratch register or two to emulate the load/store model
(x86 has memory operands, so most `LdrU64`/`StrU64` collapse to a single `mov`).

- **Pro:** fastest path to a running Linux/x86_64 binary. Reuses the most
  expensive, most-tuned asset (instruction selection) verbatim.
- **Pro:** `CodeOp` is *already* close to a virtual RISC ISA — most integer ops
  (`Add`, `Sub`, `And`, `Mul`, `LdrU64`, `Cmp`, `Branch*`) map cleanly.
- **Con:** `CodeOp` carries aarch64-specific *semantics* that don't map 1:1:
  explicit flag-setting variants (`Adds`/`Subs` vs `Add`), the FP-domain
  condition codes `b.vs`/`b.mi`/`b.ls` (NaN/ordered-compare encodings from
  plan-16/17), `MSub`, `SMulH`/`UMulH`, `Clz`/`Rbit`. Each needs a faithful x86
  emulation sequence, sometimes multi-instruction.
- **Con:** you're encoding x86 *under* an arm-shaped IR. Two-address x86, fixed
  registers, and flag liveness are fought rather than modeled → leaves
  performance on the table and makes the fixed-register problem (div/shift)
  harder because selection already happened.
- **Con:** the NEON math kernels still must be ported (they emit FP/SIMD
  `CodeOp`s the x86 encoder can't sanely reinterpret 1:1) — see §3.

### Strategy B — Generalize `CodeOp` into a target-neutral MIR

Lift `CodeOp` into a real MIR (virtual registers already exist in the regalloc
vreg layer per `plan-03-register-allocator`) and write **two instruction-selection
backends** from it (aarch64 reproducing today's output byte-for-byte; x86 new).

- **Pro:** architecturally correct; the third ISA (Windows, RISC-V) becomes
  cheap. Fixed-register constraints and flag modeling live in the MIR where the
  allocator can honor them.
- **Pro:** removes the "shared means aarch64" foot-gun permanently.
- **Con:** large refactor of the layer that took the most effort to build *and*
  the most effort to keep golden-stable. Every one of the 103 `abi::*` helpers
  and 72k lines of builders must be re-expressed against the MIR without changing
  aarch64 output — the acceptance suite is byte-exact, so the bar is "prove
  identical Mach-O/ELF for every golden."
- **Con:** long time-to-first-x86-binary; high regression risk on the *existing*
  shipping backend.

---

## 3. SIMD / transcendental kernels (decision: parity port)

~4,300 lines across the `builder_simd_*` / `builder_pow` / `builder_math` files
emit NEON to implement `pow`/`atan2`/`tan`/`fmod`/vector ops with no libm imports,
minimax polys tuned to the **1-ULP boundary** (per the memory note — there is no
degree headroom; the accuracy is load-bearing). Porting to SSE/AVX:

- The *math* (coefficients, range reduction, log2-space `pow`) is ISA-neutral and
  carries over. `simd_kernel_coeffs.rs` is data, not code.
- The *emission* (NEON `fmla`/`fcvt`/`tbl`/lane ops, `d8`–`d15` callee-saved
  discipline) must be rewritten in SSE2/AVX. Register file differs (`xmm0`–`xmm15`,
  all caller-saved in SysV — no callee-saved FP, unlike AArch64's `d8`–`d15`).
- **Verification is the hard part, not the porting:** `tools/math-kernels/runtime_ulp.py`
  (the existing ULP harness) must show every x86 kernel stays ≤1 ULP vs the truth
  oracle, matching the aarch64 result. FMA availability differs (NEON `fmla` is
  baseline; x86 needs AVX2/FMA3 — gate on `-mfma` or provide an SSE2 mul+add path
  that may shift the last ULP). **Open decision D3.**
- This work is **independent of the A/B choice** and is the single largest
  fixed cost. It is also independently landable and testable (a kernel at a time,
  against the ULP harness) before any x86 program links.

---

## 4. Rough effort (engineer-weeks, ±50%; assumes one engineer fluent in the codebase)

| Workstream | A | B | Notes |
|---|---|---|---|
| x86 encoder (`src/arch/x86_64/encode/`) | 3–4 | 3–4 | 131 ops; x86 variable-length encoding is fiddlier than fixed-width arm |
| `abi.rs` SysV sibling (103 fns) | 2–3 | 1–2 (B: against MIR) | calling convention, prologue/epilogue, syscall |
| `RegisterModel` x86 impl | 0.5 | 0.5 | trait already exists |
| Fixed-register constraints in allocator | 2–3 | 2–3 | div/mul→rdx:rax, shift→cl; new mechanism either way |
| ELF: EM_X86_64 + R_X86_64 relocs | 1–2 | 1–2 | writer mostly shared |
| `CodegenPlatform` Linux/x86 (syscalls, termios) | 1 | 1 | mostly numbers + `syscall` clobbers |
| Instruction-layer work | ~2 (encoder pattern-match glue) | **8–14** (MIR lift + aarch64 re-host golden-stable) | the strategic fork |
| NEON→SSE/AVX kernel port + ULP verify | 4–6 | 4–6 | independent of A/B; the big shared cost |
| Integration, acceptance, golden expansion | 2–3 | 3–4 | new target multiplies the golden matrix |
| **Total (very rough)** | **~18–26 wk** | **~24–38 wk** | A reaches first-binary much sooner |

The kernels (~4–6 wk) and the allocator constraint work (~2–3 wk) are paid under
*both* strategies. The real fork is the instruction layer: ~2 weeks of glue (A)
vs ~8–14 weeks of careful refactor (B).

---

## 5. Recommendation

**Sequence: constrained A first, then evaluate B.**

1. Land the strategy-independent foundations: x86 encoder, SysV `abi`,
   `RegisterModel` impl, allocator fixed-register constraints, ELF x86 relocs,
   Linux/x86 `CodegenPlatform`. These are pure additions — no risk to the
   shipping aarch64 backend.
2. Bring up Strategy A (CodeOp-as-virtual-RISC) for **console mode, integer +
   scalar-float programs only**, deferring SIMD. Prove the pipeline end-to-end
   with a real Linux/x86_64 binary that runs the acceptance subset.
3. Port the NEON kernels to SSE/AVX against the ULP harness, kernel by kernel,
   until x86 reaches accuracy parity.
4. *Then* decide B. With A shipping and the seams exercised, the cost/benefit of
   lifting `CodeOp` into a true MIR is concrete rather than speculative — and B,
   if pursued, is a refactor behind a passing x86 test suite instead of a leap of
   faith.

Rationale: B is the "correct" end state, but committing to it first means months
before any x86 code runs and maximal regression risk on the existing backend. A
de-risks the OS/ABI/encoder/reloc unknowns quickly and produces a shippable
result; the arm-shaped-IR debt it incurs is exactly what a later B pays down,
informed by real data.

---

## 5a. Impact of a planned RV64 backend

A second future ISA changes both the economics and, more importantly, the
*design constraints* on the abstraction.

**Three ISAs crosses the threshold where a real MIR pays off.** With aarch64
shipping + x86 + RV64, Strategy A's "x86 encoder pattern-matches the CodeOp
stream" is a per-ISA hack written twice against an IR shaped like neither target.
A composes badly across multiple backends; B amortizes across all three.

**RV64 fits CodeOp's *shape* but exposes its one real wart — the flag model.**
RV64GC is a clean three-address load-store RISC: 32 GPR + 32 FPR, **no
fixed-register constraints** for mul/div (so the x86 precolor work in D2 does
**not** carry over to RV64), and **no condition-flags register at all** —
RISC-V branches on a register compare (`beq`/`blt`/`bltu`) or materializes a
boolean (`slt`/`sltu`/`feq.d`/`flt.d`). That collides with CodeOp's most
aarch64-specific feature: the flag-based conditionals (`Adds`/`Subs`/`Cmp` +
`b.cond`) and especially the plan-16/17 FP-domain codes `b.vs` (NaN via overflow
flag), `b.mi`/`b.ls` (IEEE ordered compares). x86 has flags and can emulate
these; RV64 has nothing to map them onto and must synthesize each from
`fclass.d`/`feq.d`/`flt.d` into a GPR, then branch. **The flag model is the one
piece of CodeOp that fits neither future ISA cleanly, and RV64 proves it.**

**Math kernels become three SIMD targets, and the third is awkward.** RVV (the
RISC-V Vector extension) is **not** in baseline RV64GC and is vector-length-
agnostic — a different programming model, not a fixed-width port like
NEON→SSE/AVX. Recommendation: keep the algorithm/coefficients
(`simd_kernel_coeffs.rs`, range reduction, log2-space `pow`) rigorously split
from emission (already true), and ship a **portable scalar kernel path** for RV64
v1 (still zero-libm, still ≤1 ULP via `runtime_ulp.py`, just unvectorized), with
RVV as a later optimization rather than a bring-up blocker. **Open decision D6.**

### Revised recommendation given RV64

Not pure-A (throwaway, written twice) and not speculative full-B before any
second ISA exists (only aarch64 to validate against → risks re-baking arm
assumptions). Instead: **co-design the MIR during x86 bring-up, with a flag-free
condition model from day one.** Make the MIR comparison a value-producing or
compare-and-branch op (RISC-V-shaped); lower it *to* the flags hardware on
aarch64/x86 (`cmp`+`b.cond`/`jcc`) and directly to `blt`/`slt` on RV64. x86
(a genuinely different ISA) forces the abstraction to be real and validates it;
RV64 then arrives cheap because the load-bearing design decision — kill the flag
model — was made up front instead of discovered late.

This supersedes §5's "constrained A, decide B later" *if RV64 is firmly on the
roadmap*: the second backend is where to bite the MIR bullet, using x86 to
discover the MIR shape and RV64's constraints (flag-free, no fixed regs) to keep
it honest.

---

## 6. Open Decisions

- **D1 — Strategy.** Given RV64 is on the roadmap: co-design a flag-free MIR
  during x86 bring-up (recommended, §5a) vs constrained A now / decide B later
  (§5, weaker once a second future ISA exists) vs commit to full B up front
  (risks re-baking aarch64 assumptions with only one ISA to validate against).
- **D6 — RV64 SIMD.** Portable scalar kernel path for RV64 v1, RVV later
  (recommended) vs block RV64 bring-up on an RVV port. §5a.
- **D2 — Allocator fixed-register support.** Add a precolor/constraint pass to
  the existing linear-scan (recommended; localized) vs special-case div/shift in
  the x86 encoder by spilling around them (simpler, slower code). §1/§4.
- **D3 — x86 FMA baseline.** Require AVX2/FMA3 for the math kernels (matches
  NEON `fmla` precision, recommended) vs SSE2-only mul+add (wider CPU support,
  risks the last ULP and would fork the ULP goldens by ISA). §3.
- **D4 — libc vs raw syscalls on Linux/x86.** Mirror `linux_aarch64`'s current
  choice (recommended, for consistency) — confirm whether it uses `_exit`/`write`
  via libc imports or raw `svc`; x86 should match the same policy with `syscall`.
- **D5 — Golden matrix.** Whether x86 goldens are full parity with aarch64 from
  day one or a curated subset until A stabilizes. §4.

---

## 7. What stays untouched (guardrails)

No change to language surface, value/copy/move/freeze semantics, record/collection
**layout/ABI** (`mfb spec memory`), thread-transfer rules, or the NIR contract.
x86 must produce byte-identical *observable behavior*; it does **not** need
byte-identical machine code (only aarch64 goldens are byte-exact). Strategy B's
hard constraint is that the aarch64 backend's existing golden output stays
byte-for-byte identical through the MIR lift.

---

## 8. Summary

The compiler was *partly* built for a second ISA: the register model, platform
trait, encoder isolation, and ELF writer are reusable seams. The unbuilt half is
the instruction-selection layer — `CodeOp` is aarch64's instruction set wearing a
"shared" label — plus a missing fixed-register constraint in the allocator and
~4,300 lines of NEON math that need an SSE/AVX parity port. **With RV64 also on
the roadmap (§5a), the recommendation flips toward co-designing a flag-free MIR
during x86 bring-up** rather than a throwaway Strategy A: three ISAs cross the
threshold where a real IR pays off, and RV64's flag-free, fixed-register-free
shape pinpoints exactly what the MIR must abstract (the flag-based condition
model) — so x86 forces the abstraction and RV64 then comes cheap.
