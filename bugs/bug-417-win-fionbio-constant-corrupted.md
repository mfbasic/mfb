# bug-417: Windows `FIONBIO` ioctl constant is corrupted (0x8004547E vs 0x8004667E) → `ioctlsocket` fails, sockets never go non-blocking, timeouts never fire

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (wrong constant → silently ineffective non-blocking sockets)

Status: Open
Regression Test: tests/ — a Windows `net.connectTcp` with a short `timeoutMs` to a
blackhole host must fail within the timeout (currently blocks the OS default).

`const FIONBIO: &str = "2147767422"` (`src/target/win_x86_64/code.rs:38`) decodes to
`0x8004547E`, but the correct Winsock `FIONBIO` is `0x8004667E` (= 2147772030) — the
`'f'` command byte (`0x66`) is corrupted to `0x54` in the literal
(`FIONBIO = _IOW('f', 126, u_long)`). `emit_ioctl_fionbio` (:451) passes this as the
`cmd` to `ioctlsocket(fd, cmd, &argp)` for both `emit_set_nonblocking` and
`emit_restore_blocking`. `0x8004547E` is not a valid ioctlsocket command, so the call
returns `SOCKET_ERROR` (WSAEINVAL 10022) and the socket is **never** switched to/from
non-blocking mode. The return value is ignored, so nothing surfaces the failure.

The shared net layer relies on non-blocking mode for `net.connectTcp` with a timeout,
non-blocking `accept`/`poll`, etc.; on Windows those sockets silently stay **blocking**
— a short connect `timeoutMs` cannot fire (it blocks up to the OS default) and
poll-based flows misbehave.

References: `src/target/win_x86_64/code.rs:38` (the constant), `:451`
(`emit_ioctl_fionbio`). Found during goal-07.

## Failing Reproduction

Windows-only; not reproducible on the macOS host. Arithmetic proof:

```
python3 -c "print(hex(2147767422), hex(0x8004667E))"  # 0x8004547e  0x8004667e
```

- Observed: `ioctlsocket(fd, 0x8004547E, …)` → WSAEINVAL; socket stays blocking;
  timeouts don't fire.
- Expected: `FIONBIO = 0x8004667E`; the socket switches to non-blocking.

## Root Cause

The decimal literal encodes `0x8004547E` — the FIONBIO magic byte `'f'`(0x66) is
wrong (`0x54`).

## Goal

- `FIONBIO == "2147772030"` (`0x8004667E`), so `ioctlsocket` succeeds and Windows
  sockets honor non-blocking mode and timeouts.

### Non-goals

- The `ioctlsocket` call sequence and the net-layer timeout logic (correct once the
  constant is right). Consider also checking the ignored return value.

## Blast Radius

- `src/target/win_x86_64/code.rs:38` — the single constant, used by
  `emit_set_nonblocking`/`emit_restore_blocking` via `emit_ioctl_fionbio`.
