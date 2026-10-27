# bug-1127: Windows `process::poll` with a negative timeout returns at once instead of waiting forever

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Runtime (platform parity)

Status: Open
Regression Test: —

`mfb man process poll` says a negative `ms` waits with no deadline, until the
stream is readable or the child exits. On Unix that holds (probe: `poll(sh, -1)`
on `shell("sleep 1; echo late")` returns after 1.17 s and reads `late`). On
Windows (read, not run) `src/codegen/builtins/process/func_poll.rs` computes
`DEADLINE = GetTickCount64() + ms` and exits `FALSE` once `now >= DEADLINE`, so a
negative `ms` is a deadline already past: an immediate check.

Related naming drift: `poll` names the parameter `ms` while its siblings
`process::send`/`sendBytes` name the same quantity `timeoutMs`.

**The correct behavior a fix produces:** on Windows a negative `ms` waits with no
deadline, as on Unix. (Renaming `ms` to `timeoutMs`, with `ms` kept as an alias,
is an owner decision.)

References:

- Found by the plan-125 B recheck (`process` package), findings 3 and 5; probe
  `/tmp/plan-125-scratch/B-recheck/man-pkg-process/pollproj`.
