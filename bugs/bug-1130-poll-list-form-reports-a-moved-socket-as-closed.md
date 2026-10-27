# bug-1130: list-form `poll` reports a socket handed to another thread as `ErrResourceClosed`, not `ErrResourceMoved`

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Runtime (error reporting)

Status: Open
Regression Test: —

`mfb man udp` and `mfb man udp close` say any use of a socket after
`thread::transfer` handed it away raises `ErrResourceMoved`. The list form of
`poll` does not: `src/codegen/os/socket/poll.rs:lower_net_poll_list_helper` never
calls `closed_or_moved_guard` (its only resource error is "a stale/closed fd →
`ErrResourceClosed`"), and `src/codegen/builtins/udp/func_poll.rs` overload 2
declares no `ErrResourceMoved`. The same helper backs `tcp::poll`'s list form.

**The correct behavior a fix produces:** a moved socket in the list raises
`ErrResourceMoved`, and the list overloads declare it.

References:

- Found by the plan-125 B recheck (`udp` package), finding 3 (source only).
