# bug-125 — x86 encoder LOW cluster: carry-in destroyed, allocatable-pool doc drift, implicit-register clobbers

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G6). Three latent/docs
findings in the x86-64 encoder, batched per goal-02.

## 1. add_carry/sub_borrow destroy the carry-in register (divergent from AArch64/riscv)

`src/arch/x86_64/encode/emitter.rs:1963` (`add carry_in, -1`), :1994 (borrow_in).
The CF-staging `add carry_in, -1` decrements the carry register in place
(0→-1, 1→0). AArch64 (`cmp carry_in, #1`, emitter.rs:660) and riscv (sltu) leave
carry_in intact. On x86: (a) a stream that reads the carry value again sees
garbage; (b) if `carry_in` aliases `lhs` (the `add` runs before `mov dst,
lhs`), `dst` is computed from `lhs-1`. The allocator models `carry_in` as a pure
USE (analysis.rs USE_FIELDS), so a vreg whose interval extends past the op is
silently corrupted. No current site reuses/aliases carry_in; latent. Fix:
preserve carry_in (use a scratch or a non-destructive CF materialization).

## 2. Allocatable-pool doc drift — comments say 5 registers, pool has 4

`src/arch/x86_64/regmodel.rs:32-43` ("Tight (5)" vs `INT_ALLOCATABLE =
["r10","r11","r12","r14"]`); `src/target/shared/regmodel.rs:100-104`
("five-register allocatable pool"); regalloc/analysis.rs comment. plan-34-C
removed `rbx` (now `%thread`) and added `r14`, leaving 4 allocatable GPRs; three
comments still claim 5 — misleads pressure/spill reasoning (see bug-127 item 2's
eviction bound). Docs-only fix.

## 3. Multi-instruction expansions clobber implicit registers without preservation

`src/arch/x86_64/encode/emitter.rs:363-408` (`rbit` uses rax+rdx unsaved),
:1780-1793 (`var_shift`: `mov rcx, amount` before checking dst/value==rcx),
:430-447 (umulh/smulh rax/rdx) — contrast :1125/:1139/:1012 where sibling
expansions push/pop rax/rdx precisely because "the kernels keep live values
there". (a) `rbit` clobbers rax/rdx with no save (rax = internal 7th call arg &
every call result; rdx = 2nd result/3rd arg); (b) `var_shift` writes rcx first,
so a residual scratch operand mapped to rcx as dst/value computes wrong; (c)
umulh/smulh's `mov rax, lhs` destroys an rhs mapped to rax. All require an
ABI/staged/residual-scratch value live in the implicit register at the
expansion — not constructible from current vreg streams (compute-then-stage
ordering), hence latent. Same class bug-17 fixed for f2i_nearest. Fix: push/pop
the implicit registers as the protected siblings do.
