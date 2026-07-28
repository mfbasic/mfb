# bug-123 — x86 add_carry/sub_borrow with a discarded (xzr) carry-out silently writes r8

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G6).
**Severity:** MED — currently harmless (r8 dead at the sites), but a real
mis-encoding of the documented "discard carry" convention.
**Class:** correctness (latent miscompile).

## Finding

`src/arch/x86_64/encode/emitter.rs:1948-1972` (`enc_add_carry`), :1981-2003
(`enc_sub_borrow`), :1919-1935 (`enc_setcc_to`); zero-token parse at
`src/arch/x86_64/encode/operand.rs:45` (`"xzr" => 16`).

`enc_add_carry`/`enc_sub_borrow` check `is_zero_token` only for
`carry_in`/`borrow_in`. A `carry_out`/`borrow_out` of `abi::ZERO` ("discard the
carry", the documented last-limb convention in abi.rs:463-476) reaches
`enc_setcc_to(16, 0x92)`: `16 & 7 == 0` with REX.B set encodes `setc r8b; movzx
r8, r8b` — instead of discarding the carry it stores 0/1 into **r8**, a SysV
argument register (`CALL_ARGS[4]`) outside the allocatable pool and invisible to
the allocator.

## Trigger

Emitted today at `src/target/shared/code/entry_and_arena.rs:1858-1859` and
:1939-1940 (PCG64 RNG 128-bit adds, `carry_out = abi::ZERO` on the high limb).
At those sites r8 is dead (vreg-allocated helper bodies, r8 caller-saved at the
call boundary), so presently harmless — but any future inline
`add_carry`/`sub_borrow` with a discarded carry between staging the 5th call
argument (r8) and its `bl`/`call` silently corrupts that argument.

## Fix

In `enc_add_carry`/`enc_sub_borrow`, detect `is_zero_token(carry_out)` and emit
no `setcc` at all (truly discard), matching the carry_in zero-token handling.
