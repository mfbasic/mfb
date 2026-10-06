# bug-987: the Windows test scripts still default to the retired box 2230

Last updated: 2026-10-06
Effort: small (<1h) plus one real run per script on 206
Severity: LOW
Class: Footgun (test tooling)

Status: Open
Regression Test: each script run with no `--box` argument reaches 206 and passes there

**Owner decision (2026-10-06):** 2230 is retired, and 206 (`ssh test@192.168.1.206`, real
hardware) is the only Windows box (`.ai/remote_systems.md`). Repoint every Windows script's
default at 206.

## Today

`grep -n 2230 scripts/*.sh` (bug-staging-17, main 468ff0f94):

- `scripts/test-winprocess.sh`: `PORT=2230`, `host="test@127.0.0.1"`; `--box <port>` documented
  as "default 2230".
- `scripts/test-winapp.sh`: `PORT=2230`, `GPU_PORT=22`. Port 22 is mapped to
  `test@192.168.1.206` and every other port to `test@127.0.0.1`. The header comment carries the
  bug-741 2230-lavapipe note.
- `scripts/test-wintls.sh`, `scripts/test-winleak.sh`, `scripts/test-canvas-gpu-rows.sh`: also
  reference 2230 (check each one's default and host mapping).
- `scripts/README.md`, `.ai/net-tls.md`, `.ai/testing-gates.md`, `.ai/arch-abi.md`: mention
  2230 as the Windows test box.

Run with no arguments, each script tries `ssh -p 2230 test@127.0.0.1`, which is refused, so a
Windows proof silently needs a flag nobody remembers.

## Fix

- Default every script to 206 (`test@192.168.1.206`, port 22). Keep `--box` working for an
  explicit override.
- Collapse `test-winapp.sh`'s separate `GPU_PORT` default into the same box, since 206 runs both
  the console and GPU rows.
- Update `scripts/README.md` and the `.ai/*.md` mentions, so that nothing presents 2230 as a
  live box. Historical notes that explain a past measurement (for example bug-741's lavapipe
  faults) may stay, phrased as history.
- Watch for 2230-specific assumptions: the remote work directory, `cmd.exe` vs PowerShell
  quoting, and the `%ERRORLEVEL%` parse-time trap (see `.ai/remote_systems.md`, Box 206 quirks).
  206 is a real desktop, and `test` is not an administrator there.

## Proof

Run each script once against 206 with no `--box` flag, and record pass/fail with its output
tail in this doc. A script that fails on 206 for a reason unrelated to the default box gets its
own bug doc.
