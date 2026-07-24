# bug-384: Win64 external (IAT) calls with >4 args don't spill args 5+ to the stack

Status: WORKED AROUND for net recvfrom/sendto (plan-47-I); general ABI gap OPEN.

## Claim

A hand-written runtime helper that issues an external DLL/libc call with more than
four integer arguments by setting `abi::ARG[0..N]` and calling
`platform.emit_libc_call` passes args 5+ in the WRONG place on Win64. `emit_libc_call`
is a bare `branch_link` (it assumes args are already placed), and the selection pass
maps `abi::ARG[4]`/`ARG[5]` to `rdi`/`rsi` — the *internal* 8-register model
(`CALL_ARGS_WIN64 = [rcx, rdx, r8, r9, rdi, rsi, rax, rbp]`, `select.rs`). But the Win64
C ABI passes only args 1–4 in `rcx/rdx/r8/r9` and args 5+ as STACK arguments above the
32-byte shadow space. So the callee reads garbage 5th/6th args from `[rsp+0x20]`/
`[rsp+0x28]`.

Symptom seen: `net::receiveFrom`/`receiveTextFrom` returned WSAEFAULT (10014) because
`recvfrom`'s `from`/`fromlen` (args 5/6) were in `rdi`/`rsi`, not on the stack; and
`net::sendTo` silently sent each datagram to a garbage destination (its `to`/`tolen`
were likewise mis-passed), so the receiver blocked forever. TCP is unaffected — every
socket call in the TCP path has ≤4 args.

## Why this isn't caught elsewhere

The compiler's OWN generated calls go through the arg-passing infrastructure, which
(per `Win64RegisterModel::external_int_argument_registers`) caps external calls at 4
register args and spills the rest. `emit_write` (5-arg `WriteFile`) works because it
hand-places its 5th arg at `[sp+0x20]` in a self-carved frame. Only the hand-written
runtime helpers that use raw `abi::ARG[4+]` tokens for a >4-arg external call are
affected: `net/io.rs` `recvfrom`/`sendto`. Any future >4-arg IAT call (some TLS/SChannel
entry points) will hit the same trap.

## Workaround applied (plan-47-I I2)

In `net/io.rs`, the Windows arm of the `recvfrom`/`sendto` sites carves a `0x30` frame,
stores `ARG[4]`/`ARG[5]` to `[sp+0x20]`/`[sp+0x28]`, calls, then restores — exactly the
`emit_write` pattern. POSIX passes all six in registers, unchanged (byte-identical).

## Proper fix

Teach the Win64 arg lowering to spill external-call args 5+ to the stack tail
automatically, so `abi::ARG[4+]` on an `AbiBoundary::Call` to an external symbol lands
at `[sp+0x20 + 8*(n-4)]` instead of `rdi/rsi/rax/rbp`. Then the hand-written helpers need
no per-site frame carving and future >4-arg IAT calls are correct by construction.
Reserve the outgoing-args area in the frame accordingly (the shadow space is already
reserved; extend it by `8 * max(0, max_external_stack_args)`).
