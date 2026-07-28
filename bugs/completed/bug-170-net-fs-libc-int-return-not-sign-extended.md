# bug-170 — net (and fs `open`) libc `int` returns compared 64-bit without `sign_extend_word` (bug-04 class)

Last updated: 2026-07-12
Severity: MEDIUM (low confidence) — a negative C `int` return read as a large positive could be treated as success on a failing socket/poll.
Class: Correctness (defense-in-depth against the documented bug-04/bug-44 ABI condition).
Status: FIXED (2026-07-13; goal: resolve bugs 170-180; full acceptance suite green)

## Finding

`normalize_c_int_result` (`src/target/shared/code/fs_helpers_io.rs:17`) documents
that a C `int` return must be sign-extended before any signed relational compare,
because the ABI leaves the upper 32 bits of `x0`/`rax` unspecified. The audio,
TLS, and atomic-fs helpers apply it; the net helpers and fs `open` do not — they
`compare_immediate(ret, "0")` (a 64-bit cmp) then signed `branch_lt`/`branch_gt`
with no extension:

- `src/target/shared/code/net/io.rs:67-68` (accept), `:187-189`
  (getpeername/getsockname), `:1061-1062, :1085-1086` (socket/bind in bindUdp).
- `src/target/shared/code/net/mod.rs:420-422` (socket), `:465-467` (bind),
  `:479-482` (listen), `:533-536, :640-642` (connect), `:569-573` (poll — most
  dangerous: -1 read as large-positive takes `branch_gt(&connect_poll_ready)`,
  treating a poll error as "writable"), `:605-610` (getsockopt).
- `src/target/shared/code/net/poll.rs:75-78` (`net.poll` — -1 read as positive
  falls through to "socket ready").
- `src/target/shared/code/fs_helpers_atomic.rs:1337-1339, 842-844, 154-156` and
  `fs_helpers_io.rs:708-711` — `open`'s C-int return compared with `branch_ge`
  without normalization (read/write/lseek return 64-bit `ssize_t`/`off_t`, so
  those are genuinely safe; `open` is the narrowing gap).

## Trigger

One of these wrappers returns -1 with upper 32 bits left clear (the documented
bug-04 condition). A failed `accept`/`socket`/`open` then stores a bogus large
fd; a failed/interrupted `poll` reports "ready". Confidence LOW: glibc/musl/Darwin
return a sign-extended -1 in practice (the parallel unguarded fs-open path is
hardware-validated), so it does not bite today — but the guard is documented as
mandatory and is absent here.

## Fix

Insert `abi::sign_extend_word(ret, ret)` after each int-returning libc call
(accept/getsockname/getpeername/socket/bind/listen/connect/poll/getsockopt/open)
before the signed compare, or route them through `normalize_c_int_result`.
