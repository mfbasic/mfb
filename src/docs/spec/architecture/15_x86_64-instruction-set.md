# x86-64 Instruction Set

The `linux-x86_64` backend is a sibling of the AArch64 backend. It consumes the
**same neutral MIR** the shared builders and helpers produce and plugs in through
the `mir::Backend` trait, with no edits to the AArch64 path. Where the AArch64
backend turns each abstract op into one or more fixed 32-bit words, the x86-64
backend turns each into a variable-length x86-64 encoding (REX / ModRM / SIB /
opcode bytes). [[src/arch/x86_64/mod.rs:1]] [[src/arch/x86_64/backend.rs:X86_64Backend]]
This topic specifies the x86-64 op repertoire, its encodings, and how it differs
from AArch64. The register set and ABI are canonical in
`./mfb spec memory native-calling-convention`; the relocations are canonical in
`./mfb spec linker symbols-and-relocations`; the shared neutral MIR is
`./mfb spec architecture mir-instruction-set`.

## The op vocabulary is shared, not separate

x86-64 reuses the **same closed `CodeOp` enum** defined in `src/arch/ops.rs`
(imported as `crate::arch::ops::CodeOp` throughout the x86 backend). There
is one neutral op set for both ISAs; the x86 encoder dispatches on
`instruction.op.mnemonic()`. The only x86-specific additions to the shared enum
are the branch variants `X86Jae`, `X86Jp`, `X86Jnp`, `X86Ja`, `X86Jb`, `X86Jbe`,
`X86Je`, `X86Jne` (never emitted for AArch64 — see *Float branches* below).
[[src/arch/x86_64/select.rs:1]] [[src/arch/ops.rs:CodeOp]] The encoder
rejects any op it cannot yet encode with a clear `Err`. [[src/arch/x86_64/encode/emitter.rs:encode_instruction]]

The backend lives under `src/arch/x86_64/`: `backend.rs` (the `X86_64_BACKEND`
singleton), `select.rs` (instruction selection), `regmodel.rs` (the register
model), `reloc.rs` (relocation-intent mapping), and `encode/` (the machine-code
encoder). The platform crate `src/target/linux_x86_64/` supplies the program
entry, thread trampoline, arena mmap/munmap, and app-mode hooks. [[src/arch/x86_64/mod.rs:14]] [[src/target/linux_x86_64/code.rs:backend]]

## Instruction selection

`select_x86(&[MirInstruction]) -> Vec<CodeInstruction>` mirrors `select_aarch64`'s
structural conversion, reached through `Backend::select`. [[src/arch/x86_64/select.rs:select_x86]] [[src/arch/x86_64/backend.rs:select]]

- `MirOp::AddrOf` → a single `Adrp` (the x86 encoder turns it into a RIP-relative
  `lea`; AArch64's page-offset second instruction becomes zero bytes here).
- A fused flagless op is split via the shared `fused_setter_codeop` into its
  setter op plus the flag-reading branch (`cmp; jcc`), exactly the AArch64 shape.
- Non-fused MIR ops map 1:1 to a `CodeOp` via `op.to_code()`.
- After conversion, `ARENA_BASE` is renamed to `r15` and `remap_x86_abi` runs.

[[src/arch/x86_64/select.rs:select_x86]]

**The hard part of selection is ABI remapping.** Shared lowering names its
call-boundary registers by role token (`%arg`/`%ret`/`%sysnr`/`%sysarg`/`%sysret`/
`%closure_env`), which a seam (`abi::realize_abi_token`)
translates back to the AArch64 spelling (`%arg3` → `x3`, `%sysnr` → `x8`, …) before
selection. A selected stream then still carries residual AArch64 physical registers
(the ABI regs `x0`–`x8`, `sp`, `x31`/`xzr`, link reg `x30`, and leftover scratch
`x9`–`x30`). `remap_x86_abi` rewrites them to System V homes. This is materially
more complex than AArch64 selection because AArch64 has 8 argument registers while
System V has 6, and because a register's role (argument vs return vs staged result)
must be resolved by the nearest call / syscall / `ret` boundary along the
control-flow graph:

- `AbiBoundary` classifies each Call / Syscall / Ret boundary.
- `map_abi_register` maps `x0`–`x8` to a home from `CALL_ARGS` / `SYS_ARGS` / `RETS`.
- `map_scratch_register` maps residual `x9`+ scratch, deliberately avoiding `rax`
  and `rdx` (implicit in `mul`/`div`) and arranging AArch64 callee-saved `x19`–`x28`
  onto x86 callee-saved `rbx`/`rbp`/`r12`/`r13`.
- The link register `x30` has no x86 equivalent (`call` pushes / `ret` pops), so
  its frame save/restore is dropped entirely; `x31`/`xzr` is the zero register.
- A forward CFG dataflow distinguishes call arguments from call results and
  handles the runtime's 4-register error-`Result` convention.

> **Why the inference is needed.** Mapping role tokens straight to System V homes
> is not sufficient on x86: the entry stub and runtime-helper bodies stage
> call/syscall arguments through result-accessor registers (`%ret0`, and `x1`/`x2`
> for string data/length) rather than `%arg` tokens. On AArch64 that is
> byte-identical (both `%ret0` and `%arg0` realize `x0`), but on x86 the roles
> diverge (`%ret0` → `rax`, an argument needs `rdi`). The forward-CFG inference is
> what distinguishes an argument from a result by its nearest call/syscall/`ret`
> boundary, so it cannot be dropped without those sites naming `%arg`/`%sysarg`
> tokens explicitly.

[[src/arch/x86_64/select.rs:remap_x86_abi]] [[src/arch/x86_64/select.rs:map_abi_register]] [[src/arch/x86_64/select.rs:map_scratch_register]]

## Encoding (REX / ModRM / SIB)

Bytes are produced by `encode_instruction` — the single source of truth for both
the emitted bytes and the reported size, so `emit_instruction` and
`instruction_size` cannot drift. Every encoding is fixed-size and
distance-independent (rel32, imm32/imm64, disp32). [[src/arch/x86_64/encode/emitter.rs:encode_instruction]] [[src/arch/x86_64/encode/sizing.rs:instruction_size]]

Encoding primitives: [[src/arch/x86_64/encode/emitter.rs:rex]]

- `rex(w,r,x,b)` — `0x40 | w<<3 | r<<2 | x<<1 | b`.
- `modrm(md,reg,rm)` and `sib(scale,index,base)`.
- `mem_disp32(reg,base,disp)` — `[base+disp32]` (mod=10), handling the rsp/r12 SIB
  requirement (base low bits == 4).
- `mem_rip(reg)` — `[rip+disp32]` (mod=00, rm=101); the disp32 placeholder is
  patched by a relocation.
- `alu_rr(opcode,dst,src)` — the MR reg-form (`rm := rm OP reg`).

Register numbering is architectural: `rax=0, rcx=1, rdx=2, rbx=3, rsp=4, rbp=5,
rsi=6, rdi=7, r8..r15=8..15` (with `r8`–`r15` setting the REX.B/R/X bit). A
synthetic zero token (`xzr`/`x31`) decodes to the sentinel `16` ("no register"),
used by explicit-carry ops for a "no carry-in" operand. `xmm0`–`xmm15` are
decoded by `fp_reg`. [[src/arch/x86_64/encode/operand.rs:reg]] [[src/arch/x86_64/encode/operand.rs:fp_reg]]

**Encoding correctness gate.** `src/arch/x86_64/encode/tests.rs` asserts each op's
exact byte sequence (for example `mov rax,rbx` = `48 89 D8`). Unlike AArch64's
`encodes_neon_vector_ops`, these are **hand-verified against the x86-64 instruction
reference, not asserted against a system assembler.** [[src/arch/x86_64/encode/tests.rs:1]]

## Register model

`X86_64RegisterModel` answers the same questions the ISA-neutral linear-scan
allocator asks. [[src/arch/x86_64/regmodel.rs:X86_64RegisterModel]]

- **16 GPRs** in the integer class: `rax rbx rcx rdx rsi rdi rbp rsp r8..r15`.
- **The allocatable integer set is tight — only four:** `r10, r11, r12, r14`
  (AArch64 has 19). The allocator spills freely under this pressure.
- **Reserved / non-allocatable, and why:** `rax`/`rdx` (mul/div implicit, plus
  return), `rcx` (variable shift/rotate count), `rsi`/`rdi`/`r8`/`r9` (System V
  argument registers placed by selection at ABI boundaries), `rsp` (stack
  pointer), `rbp` (frame register), and `r15` (pinned `arena_base` — the analog of
  AArch64's pinned `x19`). x86 has no `xzr`, so the residual `xzr`/`x31` selection
  path realizes as `r14`; but a zero *store* encodes an immediate `0` directly, so
  `r14` needs no pinning and stays allocatable.
- **Caller-saved (volatile):** `rax rcx rdx rsi rdi r8 r9 r10 r11`.
  **Callee-saved:** `rbx rbp r12 r13 r14 r15`.

[[src/arch/x86_64/regmodel.rs:INT_ALLOCATABLE]] [[src/arch/x86_64/regmodel.rs:arena_base]] [[src/arch/x86_64/regmodel.rs:ZERO_REGISTER]]

Floating point / SIMD uses `xmm0`–`xmm15`. `xmm0`–`xmm14` are allocatable; **`xmm15`
is a reserved fixed FP scratch** (the SSE encoder needs one for the non-commutative
`dst==rhs` `subsd`/`divsd` staging that has no in-place form), mirroring `r14`/`r15`
for the GPR bank. There is **no callee-saved xmm bank** under System V, so a float
live across a `call` must spill. Spill slots are 16 bytes; integer spills use the
low 8 bytes, FP spills use a 128-bit `movups` (a 64-bit `movsd` would drop a
spilled vector's high lane). [[src/arch/x86_64/regmodel.rs:FP_REGS]] [[src/arch/x86_64/regmodel.rs:spill_slot_bytes]]

## Calling convention (System V AMD64)

Arguments in `rdi rsi rdx rcx r8 r9`; return in `rax`; `syscall` (`0F 05`) with
the number in `rax` and arguments in `rdi rsi rdx r10 r8 r9`. The internal
`CALL_ARGS` table extends the six arg registers with `rax` (7th) and `rbp` (8th)
for internal 8-parameter MFBASIC calls only. The error-`Result` convention returns
its four fields in `RETS = [rax, rdx, rcx, rsi]`. [[src/arch/x86_64/select.rs:CALL_ARGS]] [[src/arch/x86_64/select.rs:RETS]]

**16-byte stack alignment is the key ABI difference from AArch64.**
`frame_call_padding()` returns 8 for x86-64 (AArch64 returns 0). The `call`
instruction pushes the 8-byte return address, so a callee must add 8 to its
16-byte-aligned frame to keep `rsp` 16-byte-aligned at *its own* call sites —
otherwise libc's variadic `movaps` register-save faults. AArch64 needs none of
this because the link register is a register and nothing is pushed. The thread
trampoline applies the same re-bias (an extra `subtract_stack(8)`) since pthread
enters a C callee with `rsp ≡ 8 (mod 16)`. [[src/arch/x86_64/backend.rs:frame_call_padding]] [[src/target/linux_x86_64/code.rs:emit_thread_trampoline]]

**External-call vector marker.** An external (libc) call emits `mov eax, 8` before
`call rel32` — the System V variadic ABI's "number of vector registers used"
marker (8 is a safe superset). An internal `_mfb_*` call must **not** emit it
(internal functions may pass a 7th argument in `rax`, which the marker would
clobber). The choice is `target.starts_with("_mfb_")`. [[src/arch/x86_64/encode/emitter.rs:encode_instruction]]

Program entry reuses the shared `lower_program_entry` (Result-tag error reporting,
signal setup, RNG-seed, global init), with `select_x86` mapping neutral registers
to System V homes plus the one-time `xor r14,r14` zero-register init. Program exit
uses `exit_group`. The `__libc_start_main` trampoline (7th argument on the stack)
applies to **app (GTK) mode only**, not console mode. [[src/target/linux_x86_64/code.rs:emit_program_entry]]

## Op-specific lowerings and hazards

- **Float branches (the `X86J*` family).** After a float compare, `ucomisd` sets
  CF/ZF/PF (unordered ⇒ CF=ZF=PF=1), which differs from AArch64 `fcmp`'s NZCV, so
  the integer `b.cc → jcc` mapping would mishandle every NaN case. `x86_float_branch`
  rewrites each AArch64 float-relation branch into `x86.*` jumps reproducing the
  exact ordered-only / unordered truth set (e.g. `<` → `jp skip; jb target; skip:`;
  `!=` → `jp target; jne target`). These `x86.*` ops are inert to the shared
  `lower_to_mir`, so the stream is a fixed point on re-lowering. Integer `b.cc`
  map to `jcc` directly. [[src/arch/x86_64/select.rs:x86_float_branch]]
- **Flags are implicit.** x86 `add`/`sub` always set EFLAGS, so the flag-setting
  `adds`/`subs` share the same encoding as `add`/`sub`. [[src/arch/x86_64/encode/emitter.rs:alu3]]
- **Three-operand → two-operand.** The neutral `CodeOp` set is three-operand
  (`dst,lhs,rhs`); x86 ALU is two-operand (`dst OP= src`). `alu3` synthesizes the
  three-operand form with `dst==lhs` in place, `dst==rhs` by commutative swap (or,
  for `sub`, negate-then-add), else a `mov dst,lhs` first. It also handles a
  zero-token `lhs` (AArch64 freely sources `xzr`). `dst==src1` aliasing hazards are
  handled per op (mul/imul commutativity, `msub` minuend capture, SSE staging in
  `xmm15`, div staging the divisor when it aliases rax/rdx). [[src/arch/x86_64/encode/emitter.rs:alu3]]
- **Synthesized instructions with no single-op x86 form:** `rbit` (bit-reverse via
  stride swaps + `bswap`), `clz` (`lzcnt`, ABM / x86-64-v2), `rev_w`/`rev_x`
  (`bswap`), variable shifts/rotate through CL, `mulhi` (`mul`/`imul` → rdx),
  `div`/`idiv` (rax/rdx + `cqo` / `xor rdx,rdx`). Explicit-carry `add_carry`/
  `sub_borrow` set CF from the carry-in (`add carry_in,-1`), then `adc`/`sbb`, then
  `setc`+`movzx` for the carry-out. [[src/arch/x86_64/encode/emitter.rs:enc_add_carry]]

## Floating point and SIMD → xmm

- **Scalar doubles (SSE2).** The AArch64 `dN` bank maps 1:1 to `xmmN` (and the
  128-bit `vN`/`qN` aliases map to the same `xmmN`). Ops: `addsd/subsd/mulsd/divsd`,
  `sqrtsd`, `ucomisd` (`fcmp_d`), `movq` reinterprets, `movaps` copy, `cvtsi2sd`/
  `cvttsd2si` conversions, SSE4.1 `roundsd` for directed rounding, and ties-away
  emulated as `trunc(x + copysign(0.5, x))`. `fneg`/`fabs` build the sign mask in
  `xmm15` — no memory-mask constant. `fminnm_d`/`fmaxnm_d` (`math::min`/`max(Float)`)
  use `minsd`/`maxsd` — order-preserving `min/max(lhs, rhs)`, correct for the finite
  operands MFBASIC produces; they differ from AArch64 `fminnm`/`fmaxnm` only on a NaN
  input or a ±0 tie (neither reachable through `math::min`/`max`).
  [[src/arch/x86_64/encode/emitter.rs:sse_arith]]
- **v128 SIMD (SSE2 / SSE4.1 / SSE4.2).** 128-bit load/store is `movups`. Packed
  f64×2 arithmetic uses the `66 0F` forms (`addpd`/`subpd`/`mulpd`/`divpd`/`minpd`/
  `maxpd`), packed i64×2 uses `paddq`/`psubq`, bitwise uses `pand`/`por`/`pxor`,
  plus `sqrtpd`, `roundpd`, and NaN-correct `cmppd`/`pcmpgtq`/`pcmpeqq`. Lane
  extract is `movq`/`pextrq`; broadcast is `movq`+`punpcklqdq`. [[src/arch/x86_64/encode/emitter.rs:encode_instruction]]
- **FMA3.** `fmla_v`/`fmls_v` use `vfmadd231pd`/`vfnmadd231pd` (3-byte VEX,
  x86-64-v3) for single-rounding parity with AArch64 `fmla`/`fmls`. The four scalar
  fused ops `fmadd_d`/`fmsub_d`/`fnmsub_d`/`fnmadd_d` use the matching 231-form
  `sd` opcodes (`vfmadd231sd` B9 / `vfmsub231sd` BB / `vfnmadd231sd` BD /
  `vfnmsub231sd` BF) — same VEX prefix as the packed form, opcode LSB odd. The
  `addend` is staged in the reserved `xmm15` so `dst` need not alias a source; as
  on AArch64 the neutral mnemonic names the result, so `fnmsub_d`→`vfnmadd231sd`
  and `fnmadd_d`→`vfnmsub231sd`. [[src/arch/x86_64/encode/emitter.rs:enc_vfma231sd]]
- **Emulated where SSE has no form:** packed i64↔f64 conversions (`fcvtzs_v`/
  `scvtf_v`) run lane-serial through rax/rdx + `xmm15` (`pshufd 0xEE` brings lane 1
  to lane 0); `sshr_v` (arithmetic i64 lane shift-right) uses `psrlq` plus a
  sign-fill from `pcmpgtq`. At the AArch64-legal boundary `sshr_v #64` the fill is
  kept unshifted and `psrlq dst, 64` zeroes the lane, so the `por` yields the sign
  fill; `shl_v`/`ushr_v` take the same boundary as the zero an x86 count > 63
  already produces. A lane shift above 64 is rejected.
  [[src/arch/x86_64/encode/emitter.rs:encode_instruction]]

## Unsupported ops on x86-64

The encoder rejects ops it does not encode with an explicit `Err`
(`x86 encode: unsupported op {op}`) rather than miscompiling.
[[src/arch/x86_64/encode/emitter.rs:encode_instruction]]

- **Unsupported SIMD ops:** `fcvtas_v` (nearest ties-away vector), `sshl_v`,
  `ushl_v` (per-lane variable shifts). The `sshl_v` rejection is pinned by the
  `unsupported_op_errors` test. [[src/arch/x86_64/encode/tests.rs:unsupported_op_errors]]
- **`arena_base` is pinned on `r15`** (the analog of AArch64's pinned `x19`),
  narrowing the integer allocatable set accordingly.

There is no `str_u16` op in the shared op vocabulary — a U16 store is never
emitted — so the encoder's `str_u16` error arm is unreachable through normal
dispatch. [[src/arch/ops.rs:CodeOp]]

## See Also

* ./mfb spec architecture aarch64-instruction-set — the sibling AArch64 op catalog
* ./mfb spec architecture mir-instruction-set — the shared neutral MIR both ISAs select from
* ./mfb spec memory native-calling-convention — registers, ABI, and clobber sets
* ./mfb spec linker symbols-and-relocations — the relocation kinds that patch RIP-relative references
* ./mfb spec linker linux-x86_64 — the x86-64 ELF container and dynamic linking
