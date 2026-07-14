# bug-109 — net.write/writeText misclassifies errors on linux-x86_64 (raw-syscall write + stale errno)

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — cross-target divergence: a write timeout is reported as a
closed connection on x86-64 only.
**Class:** correctness (platform-divergent behavior).

## Finding

`src/target/shared/code/net/io.rs:557-588` (`lower_net_write_helper`) with
`src/target/linux_x86_64/code.rs:308-322` (`emit_write` = raw `SYS_WRITE`
syscall).

The net write loop's failure path calls `platform.emit_errno`
(`__errno_location`) to distinguish EAGAIN (→ ERR_WRITE_TIMEOUT) from
everything else (→ ERR_CONNECTION_CLOSED). On x86-64, `emit_write` is a raw
`syscall` that returns `-errno` in rax and never sets libc `errno`, so the
helper reads a stale/unrelated errno. The fs/io write paths handle exactly this
via `write_uses_raw_syscall()` + the raw branch of `emit_eintr_retry_or_error`
(fs_helpers_io.rs:32,76-84, the bug-62 fix); net/io.rs was never given the same
treatment. This also makes the libc `write` import declared by the net arm
(linux_x86_64/plan.rs:246-252 via `net_libc_symbols("net.write") = ["write"]`)
dead on x86 (bug-71 class).

## Trigger

linux-x86_64 program: `net::setWriteTimeout(sock, 100)`, peer stops reading,
keep `net::write`-ing until the socket buffer fills. Kernel returns -EAGAIN
from the raw write; stale errno ≠ 11 → program gets ERR_CONNECTION_CLOSED
(77070004) instead of ERR_WRITE_TIMEOUT. Same program on aarch64/riscv64/macOS
gets ERR_WRITE_TIMEOUT.

## Fix sketch

Give net/io.rs the same raw-syscall-aware error extraction the fs/io side uses:
when `write_uses_raw_syscall()`, read the errno from the negated return value
in rax rather than from `__errno_location`.

## Prior art

bug-62 covered fs/io only; bug-71 covered other dead imports;
audit-1-fs-net-thread.md does not mention it.

## Resolution

FIXED in commit e0fa88b8. net write derives errno from the negated raw-syscall return when write_uses_raw_syscall(); validated on the x86 box (port 2227).

Regression test: `tests/rt-behavior/net/bug109_write_timeout` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
