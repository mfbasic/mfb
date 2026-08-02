# bug-417: Windows `FIONBIO` ioctl constant is corrupted (0x8004547E vs 0x8004667E) → `ioctlsocket` fails, sockets never go non-blocking, timeouts never fire

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (wrong constant → silently ineffective non-blocking sockets)

Status: FIXED (0ff62c711)
Regression Test: `fionbio_tests::ioctl_cmd_immediate_is_fionbio`
(`src/target/win_x86_64/code.rs`) — an emit-inspection test that calls
`emit_ioctl_fionbio` and pins the `mov_imm` into `ARG[1]` (the `ioctlsocket`
`cmd`) to `2147772030` (0x8004667E) for both the non-blocking and restore-blocking
paths. RED before the fix (`left "2147767422"` vs `right "2147772030"`), GREEN
after. (A live Windows `net.connectTcp`-timeout runtime check is Windows-only /
box-2230 and not part of the macOS acceptance matrix.)

## STATUS: FIXED (0ff62c711)

Single root cause, single-line fix — no fan-out. `const FIONBIO` in
`src/target/win_x86_64/code.rs:38` was `"2147767422"` (0x8004547E): the `'f'`
magic byte (0x66) of `_IOW('f', 126, u_long)` was corrupted to 0x54. Corrected to
`"2147772030"` (0x8004667E, the canonical Winsock FIONBIO). `ioctlsocket(fd,
0x8004547E, &argp)` had been returning WSAEINVAL, so `emit_set_nonblocking` /
`emit_restore_blocking` never toggled Windows sockets' blocking mode and connect
`timeoutMs` / poll-based flows silently stayed blocking.

Verification: arithmetic proof now matches (0x8004667E == canonical FIONBIO);
new emit-inspection test RED→GREEN; full `cargo test --bin mfb` = 3747 passed,
0 failed. The change is Windows-only (a constant used solely by
`emit_ioctl_fionbio`); it touches no macOS/linux/rv64 emitted bytes, so no
`.ncodesum` golden shifts (Windows has none in artifact-gate). Fix commit
8d0b48b74, merge 0ff62c711.

Deviation from the doc's non-goal note ("consider also checking the ignored
return value"): left as-is. It is an explicit non-goal; the call sequence is
correct once the constant is right, and adding return-value handling would be a
separate change with its own Windows runtime proof.

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
  ✅ Fixed: `"2147767422"` → `"2147772030"` + emit-inspection regression test
  (`fionbio_tests`). Commit 8d0b48b74, merge 0ff62c711.
