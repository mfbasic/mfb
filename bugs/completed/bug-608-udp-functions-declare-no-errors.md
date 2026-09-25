# bug-608: `udp` functions raise errors their descriptors never declare

Last updated: 2026-09-13
Effort: small–medium
Severity: MED
Class: Correctness (registry error declaration) / Documentation

Status: Fixed
Regression Test: src/codegen/builtins/udp/mod.rs and src/codegen/builtins/net/mod.rs (`members_declare_the_errors_they_raise`)

> **STATUS: FIXED (883bec900)** — Every `udp` form and every `net` member declares the errors its lowering raises (per-form raise-site audit; lists spelled as `tcp`'s descriptors for the same helpers); `mfb man udp receive` / `udp poll` / `net toUrl` / `net ping` render Errors tables. **Deviations:** (1) the Blast-radius audit widened the fix to `net` (`percentDecode`, `toUrl`, `lookup`, `ping`; `parseQuery` correctly stays empty — its helper RECOVERs); a runtime probe trapped 77050003 from `percentDecode`/`toUrl` and 77050007 from `toUrl`. (2) Phase 1's Errors-table render test and loopback `ErrTimeout` inline-TRAP test were not added: the declared-errors unit tests pin exactly the lists the man renderer reads, and the fallibility concern did not apply — `udp`/`net` members are `abi_function` bodies, not `AbiInline`, so the inline-TRAP census never read their `errors`, and a measured `udp::receive` TRAP handler already ran, so a TRAP test could not have been RED.

Nine `udp` forms register `errors: vec![]`: `bind`, `receive`, `localAddress`,
both `send` forms, both `poll` forms, `setReadTimeout`, `setWriteTimeout`
(`grep -n 'errors: vec!' src/codegen/builtins/udp/func_*.rs`). Only `udp::close`
declares any. But the lowering raises real, trappable errors:

```
grep -n -o 'ErrNetworkFailed\|ErrTimeout\|ErrAddressInvalid\|ErrMessageTooLarge\|ErrInvalidArgument\|ErrResourceClosed\|ErrOutOfMemory' \
  src/codegen/builtins/udp/gen_io.rs | cut -d: -f2 | sort | uniq -c
#  2 ErrAddressInvalid   2 ErrInvalidArgument   6 ErrMessageTooLarge   3 ErrNetworkFailed
#  3 ErrOutOfMemory      2 ErrResourceClosed    2 ErrTimeout
```

As a result, `mfb man udp bind`, `receive` and `send` render **no Errors section**,
so a developer is never told that `receive` times out with `ErrTimeout` or
rejects an oversized datagram with `ErrMessageTooLarge`. Worse, the declared-error
list is what the compiler reads to decide fallibility (`mfb spec language
error-model` §8.6 rule 11: "a member that raises an error it does not declare is
wrongly proved infallible and its handler is deleted" — for inline-lowered
members). Whether any `udp` form is affected that way must be established in
Phase 1.

**The single correct behavior a fix produces:** each `udp` descriptor declares
exactly the errors its lowering can raise; every page's Errors table lists them;
an inline `TRAP` on each form keeps its handler.

Found by plan-125-B Phase 3's Codex review of `udp`
(`planning/plan-125-findings/B-phase3/man-pkg-udp.md`, finding 1). Same class as
bug-606 (`encoding` decoders). **Filed, not fixed**, by user instruction during a
documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```
mfb man udp receive | grep -c '^ERRORS\|Errors'    # 0: no Errors section
```

A runtime repro needs a bound socket (plan-125-B §3.3: network probes run on the
main thread). Phase 1 writes one: bind a loopback socket, set a 10 ms read
timeout, `receive` with an inline `TRAP`, and assert the handler runs with
`ErrTimeout` — with no `TYPE_INLINE_TRAP_DEAD_HANDLER` warning at build.

## Root cause

The descriptors in `src/codegen/builtins/udp/func_{bind,receive,local_address,send,poll,set_read_timeout,set_write_timeout}.rs:register`
were written with empty `errors` lists. The raise sites are in
`src/codegen/builtins/udp/gen_io.rs`. Nothing checks a descriptor's `errors`
against the errors its lowering raises.

## Non-goals

- Changing any error a `udp` call raises.
- Hand-writing error rows into description prose instead of declaring them.

## Blast-radius audit

- `tcp`, `tls`, `net` descriptors: audit the same way (declared list vs. raise
  sites in their `gen_*.rs`). bug-606 found the class in `encoding`.
- A census test (every `raise_error*("Err…")` reachable from a member's lowering
  must appear in its `errors`) would close the class; recorded for the fixer.

## Fix

Phase 1 — per-form raise-site audit; RED tests: the Errors-table presence for
each form, and the loopback `ErrTimeout` inline-TRAP repro. Commit: 883bec900

Phase 2 — declare the audited errors on each descriptor (GREEN); check goldens
that carry error lists; full suite. Commit: 883bec900

## Phase 1 findings (fix-bug, 2026-09-15)

- Reproduced at main `9b5e5b55f`: `mfb man udp <form>` has no Errors section for
  `bind`, `receive`, `localAddress`, `send`, `poll`, `setReadTimeout`, `setWriteTimeout`
  (only `close` has one).
- **Fallibility: not affected (measured).** A loopback program binding a socket, setting a
  10 ms read timeout and calling `udp::receive(s, 100) TRAP(e)` builds with no
  `TYPE_INLINE_TRAP_DEAD_HANDLER` warning and prints `trapped Operation did not complete
  before its deadline.` The inline census (`builtins::inline_builtin_is_infallible` via
  `registry::native_member_declares_error`) only reads `errors` for `Body::AbiInline`
  members; every `udp` form is a `native_body` (`abi_function`), for which it returns
  `None`. So the defect is the missing declarations and Errors tables, not a deleted handler.
- Raise-site audit, per form (declared lists spelled as `tcp`'s descriptors for the same helpers):
  - `bind` — `gen_io.rs:lower_net_bind_udp_helper`: ErrAddressInvalid, ErrNetworkFailed, ErrOutOfMemory.
  - `receive` — `gen_io.rs:lower_net_receive_from_helper`: ErrAddressInvalid, ErrInvalidArgument,
    ErrMessageTooLarge, ErrNetworkFailed, ErrOutOfMemory, ErrResourceClosed, ErrTimeout.
  - `send` — `gen_io.rs:lower_net_send_to_helper`: ErrAddressNotFound, ErrMessageTooLarge,
    ErrNetworkFailed, ErrOutOfMemory, ErrResourceClosed, ErrTimeout; the byte form adds
    ErrInvalidArgument (`bad_payload`, bug-497, `if !text`).
  - `poll` — `os/socket/poll.rs:lower_net_poll_helper`: ErrInvalidArgument, ErrResourceClosed;
    list form `lower_net_poll_list_helper`: + ErrOutOfMemory, ErrTimeout.
  - `localAddress` — `os/socket/shared.rs:lower_net_address_helper`: ErrAddressInvalid,
    ErrOutOfMemory, ErrResourceClosed.
  - `setReadTimeout` / `setWriteTimeout` — `os/socket/poll.rs:lower_net_set_timeout_helper`:
    ErrInvalidArgument, ErrResourceClosed.
- Blast radius: `tcp` and `tls` declare no empty list (`grep -n 'errors: vec!\[\]' src/codegen/builtins/{tcp,tls}/*.rs`
  is empty), and `tcp`'s lists for the shared helpers match the sites above. **`net` has the
  same defect (sub-issue B, fixed here):** every member declares `errors: vec![]`, but
  - `percentDecode` reaches `FAIL error(77050003)` (ErrInvalidFormat) in `__net_percentDecodeImpl`;
  - `toUrl` reaches `FAIL error(77050003)` / `FAIL error(77050007)` (ErrInvalidFormat, ErrUnsupported),
    including through `__net_parsePort`;
  - `lookup` — `gen_io.rs:lower_net_lookup_helper`: ErrAddressInvalid, ErrAddressNotFound, ErrOutOfMemory;
  - `ping` (both overloads; the resolve step is not gated on the Address form) —
    `gen_ping.rs:lower_ping_posix` / `lower_ping_windows`: ErrAddressInvalid, ErrInvalidArgument,
    ErrNetworkFailed, ErrOutOfMemory;
  - `parseQuery` correctly declares none: `__net_decodeQueryComponent` `RECOVER`s the decode error.
- RED tests: `codegen::builtins::udp::tests::members_declare_the_errors_they_raise`,
  `codegen::builtins::net::tests::members_declare_the_errors_they_raise`.
