# bug-384: Win64 external (IAT) calls with >4 args don't spill args 5+ to the stack

Status: FIXED (2026-07-24, commit 1a079a519)

## Resolution

Added `native_helpers::emit_external_int_call`, which spills every integer arg
at or beyond `register_model().external_int_argument_registers()` to the
`OUTGOING_ARGS_BASE` sentinel that `finalize_frame` already reserves and
resolves (Win64 arg 5 → `[rsp+0x20]`, above the shadow; SysV/AAPCS/riscv → frame
bottom). Because the threshold comes from the register model, the spill loop is
empty for any call within a target's register-arg limit (SysV 6, AAPCS64/riscv64
8), so the emitted bytes are byte-identical to a bare `emit_libc_call` on every
non-Win64 target — only Win64 (4 register args) actually spills.

The four `net` sites were converted to it and their manual `sub_sp` brackets
removed: `recvfrom`/`sendto`/`setReadTimeout` (previously worked around) plus
the two sites that had **no** guard and passed a garbage 5th arg on Win64 —
`setsockopt(SO_REUSEADDR)` on `listenTcp` (best-effort, so latent) and
`getsockopt(SO_ERROR)` on a non-blocking connect (reached only when the initial
`connect()` returns EINPROGRESS, so latent on fast loopback). `schannel`'s
`sspi_call`/`sspi_call_ext` and the CNG helpers already solved this locally and
were left unchanged; the LINK-thunk refusal for >N-arg native calls
(`link_thunk.rs`) is a separate feature and still stands (it is safe — it
rejects rather than miscompiles).

Verified byte-identical `net` `.ncodesum` on linux-x86_64, linux-aarch64,
linux-riscv64, macos-aarch64; the Win64 ncode shows the getsockopt 5th arg
spilled to `[rsp+0x20]`; and on 2230 (Win11) `func_net_receiveFrom_valid`
(sendto/recvfrom/SO_RCVTIMEO) prints `4`/`77070007` and `func_net_accept_valid`
(SO_REUSEADDR + non-blocking connect getsockopt) prints `accepted two`.

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
