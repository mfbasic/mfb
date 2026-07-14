# plan-00-F — Runtime Helpers → MIR

Last updated: 2026-06-29

> **Status: DONE (byte-identical scope).** Two parts, both AArch64-byte-identical
> under `-codegen mir`.
>
> 1. **Vocabulary completion.** The last AArch64-named machine-y ops `bl`/`blr`/
>    `svc` → neutral `call`/`call_indirect`/`syscall` (renamed group, 1:1). The
>    macOS `svc; b.<carry>` syscall error idiom — the one flag branch plan-00-B
>    deferred here — fuses into a new flagless `syscall_br` op. The helper MIR is
>    now **fully neutral and flagless**: a `-mir` dump over an entry+arena+RNG+
>    thread+error program has zero `svc`/`bl`/`blr`/`adrp`/`x19`/NEON/standalone
>    `b.cc` (only neutral ops + the universal `b`/`ret`/`branch_self`/`add_sp`/
>    `sub_sp` + the deferred `adds;addc` carry chain, §4).
> 2. **Helpers through the seam.** The hand-written helpers bypass the builder's
>    pre-allocation seam, so under `-codegen mir` they are routed through
>    `lower_to_mir → select_aarch64` (the identity) at plan assembly
>    (`route_function_through_mir`) — bringing the entry sequence, arena
>    allocator, error path, PCG64 RNG, math kernels, and thread trampoline under
>    the byte-identical gate.
>
> **Deliberately NOT done** (honors the "byte-identical" constraint): the helper
> *bodies* still emit via the AArch64 `abi::*` builders with fixed physical
> registers (`x9`/`x10`/`v22`…) — neutral *ops*, pinned *registers*. Rewriting
> them to **vreg** MIR (so the shared allocator places registers per-ISA, which is
> what makes H/I additive) re-pins the AArch64 registers and is therefore **NOT
> byte-identical** — it cannot live in byte-identical F. It is scheduled as
> **plan-00-G Phase 2** (the helper vreg migration), validated by the runtime
> suite once the self-diff gate is retired. plan-00-F delivers the byte-identical
> half: every helper *op* is neutral, every helper stream is proven
> MIR-representable, and the helpers flow through the MIR seam.
>
> Validation: 36→39-fixture op-family sweep (+ call/call_indirect/syscall) + a
> dedicated `syscall_br` fusion test; full bin+integration tests green;
> codegen-selfdiff byte-identical **with helpers now routed through MIR** (only
> the pre-existing `bug-01` union-drop non-determinism fails, confirmed
> non-deterministic direct-vs-direct); RNG + threads run correctly under
> `-codegen mir`; acceptance 975/975; the two `.mir` goldens regenerated.

Port the hand-written AArch64 runtime helpers to MIR so they are written once and portable
(`mir.md §9`, §12.5 — resolved: long-term *all* helpers are MIR; the only per-ISA code is the
backend itself).

Depends on plan-00-A–E (needs the neutral op set, incl. `v128` for the kernels, `syscall`/
`call`/`arena_base` for the machine-y ones). Stays AArch64-**byte-identical** under
`-codegen mir`.

## 1. Goal

Re-express every `lower_*` helper `CodeFunction` as a MIR function:

- **Pure compute + memory (first):** `arena_alloc`/`arena_free`/`arena_insert_free`,
  `build_error_loc`, `make_error_result`, the PCG64 RNG fill (`arena_fill_*`), `fmod`,
  `simd_alloc_list`, and the transcendental kernels (already `v128` after plan-00-E).
- **Machine-y (last, but still MIR):** the entry/`_start` shim, `_mfb_shutdown` + signal
  setup, the syscall stubs, the thread trampoline register setup — via the MIR `syscall` op,
  the ABI-abstract `call`, and `arena_base`.
- After this plan there are **no hand-written AArch64 helper bodies** — only MIR.

### Non-goals

- No new ISA. The AArch64 backend still encodes everything; output stays byte-identical. The
  per-ISA *backend* (encoder/RegisterModel/ABI/relocs/frame) is **not** a "helper" and is
  untouched here.

## 2. Current State

Helpers are `CodeFunction`s built by `lower_*` in `entry_and_arena.rs` etc., emitting AArch64
`CodeOp` by hand (`lower_arena_alloc`, `lower_build_error_loc`, `lower_make_error_result`,
`lower_arena_fill_*`, `lower_shutdown`, the entry sequence, …). They assume `x19`, the AArch64
syscall (`svc`), and the AArch64 ABI — all now neutralized by plans B–E + `arena_base`/
`syscall`/`call`.

## 3. Design

Rewrite each `lower_*` to build a **MIR** function (neutral ops, vregs, `arena_base`,
`syscall`, `call`). The AArch64 selection from plans A–E lowers them to the same instructions
they emit today — proven by the byte-identical gate. The entry/syscall/signal helpers use
the `syscall` op (nr + args) and `call` (ABI-abstract); their per-ISA register placement is a
backend detail, not in the helper.

## 4. Phases

1. Port the pure-compute helpers (arena family, error builders, RNG, `fmod`, `simd_alloc_list`).
2. Confirm the kernels are MIR (`v128`) end-to-end (overlaps plan-00-E).
3. Port the machine-y helpers (entry, shutdown/signal, syscall stubs, thread trampoline) via
   `syscall`/`call`/`arena_base`.
4. Byte-identical gate; delete the hand-AArch64 helper bodies (the `lower_*` now produce MIR).

## 5. Validation

- Suite **byte-identical** under `-codegen mir` after each helper ported (the helpers are in
  every binary — the entry sequence, the arena, the error path, threads).
- The RNG reproducibility, thread tests, signal/shutdown behavior, and the ULP harness
  (kernels) are the load-bearing runtime checks.

## Summary

The plan that makes "the backend is tiny per ISA" true: once the helpers are MIR, adding
x86_64/rv64 means writing a *selector + encoder + ABI*, not re-hand-coding the arena
allocator, the error path, the RNG, and the math kernels three times. The machine-y helpers
fall out via the `syscall`/`call`/`arena_base` ops — no hand asm survives.
