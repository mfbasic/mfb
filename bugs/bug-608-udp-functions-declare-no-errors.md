# bug-608: `udp` functions raise errors their descriptors never declare

Last updated: 2026-09-13
Effort: small–medium
Severity: MED
Class: Correctness (registry error declaration) / Documentation

Status: Open
Regression Test: none yet — see Phase 1

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
each form, and the loopback `ErrTimeout` inline-TRAP repro. Commit:

Phase 2 — declare the audited errors on each descriptor (GREEN); check goldens
that carry error lists; full suite. Commit:
