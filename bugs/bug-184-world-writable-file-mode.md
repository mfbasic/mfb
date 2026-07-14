# bug-184: file-creating fs builtins open with mode 0o666 → secrets land world-readable/writable

Last updated: 2026-07-14
Effort: small (<1h)
Severity: HIGH
Class: Security

Status: Open
Regression Test: tests/rt-behavior/fs_create_mode_0600 (to be added)

Every general file-creating path in the `fs` runtime helpers passes the literal
mode `438` (octal `0o666`) to the `open`/`openat` syscall. The resulting file is
created with permission bits `0o666 & ~umask` — world-readable under the common
umask `022` (→ `0o644`), and world-*writable* under a permissive umask (e.g. a
daemon with `umask 0`). A program that writes a secret (a token, a private key,
a config with credentials) via `fs::writeText`/`writeBytes`/`open`/append
therefore leaks it to every local user by default. The single correct behavior a
fix produces: newly-created files are `0o600` (owner-only) unless the program
explicitly requests otherwise, matching what `createTempFile`/`atomicWrite`
already do.

This is the still-open audit-1 finding **OS-01**, re-verified against current
code. See `planning/audit-2-fs-net-thread.md`.

References:

- `planning/audit-2-fs-net-thread.md` (OS-01), `planning/old-plans/audit-1-fs-net-thread.md`
- Safe contrast in-tree: `createTempFile` uses mode `384` (`0o600`),
  `src/target/shared/code/fs_helpers_atomic.rs:149`; `atomicWrite` goes through
  `mkstemp`/`mkstemps` (0o600).

## Failing Reproduction

```
mfb init /tmp/permproj
cat > /tmp/permproj/src/main.mfb <<'EOF'
IMPORT fs
FUNC main() AS Integer
  fs::writeText("/tmp/secret.txt", "api-key=hunter2")
  RETURN 0
END FUNC
EOF
mfb build /tmp/permproj && /tmp/permproj/target/*/permproj
ls -l /tmp/secret.txt
```

- Observed (umask 022): `-rw-r--r--` — the secret is world-readable. Under
  `umask 000` (e.g. inside a service manager that clears umask): `-rw-rw-rw-`,
  world-writable.
- Expected: `-rw-------` (`0o600`) — owner-only — for a freshly created file.

Contrast: `fs::createTempFile` produces a `0o600` file today; the general
write/open path does not.

## Root Cause

The `open` syscall's mode argument is hardcoded to `"438"` (0o666) on the
file-creating paths:

- `src/target/shared/code/fs_helpers_io.rs:700` — `openFile` write/create path
  (`abi::move_immediate(abi::ARG[2], "Integer", "438")`).
- `src/target/shared/code/fs_helpers_atomic.rs:920` — `writeText`/`writeBytes`
  create.
- `src/target/shared/code/fs_helpers_atomic.rs:1442` — append create.

`O_CREAT` mode `0o666` is a POSIX convention for *shared* files that assumes a
restrictive umask; it is the wrong default for a language runtime that writes
arbitrary user data, including secrets. There is no code path that tightens the
mode afterward.

## Goal

- Files created by `fs::open`/`writeText`/`writeBytes`/append are `0o600` by
  default (owner read/write only), across Linux and macOS, on all four targets.

### Non-goals (must NOT change)

- Do not change any `fs::` built-in signature or add a new public mode parameter
  (language surface is frozen for this fix).
- Do not alter the already-correct `0o600` paths (`createTempFile`,
  `atomicWrite`).
- Do not touch the *access* mode flags (`O_RDWR`/`O_APPEND`/`O_TRUNC`) — only the
  create permission bits.

## Blast Radius

- `fs_helpers_io.rs:700` — fixed by this bug.
- `fs_helpers_atomic.rs:920`, `:1442` — same hardcoded `438`; fixed here.
- `createTempFile` (`fs_helpers_atomic.rs:149`), `atomicWrite` (mkstemp path) —
  already `0o600`; unaffected.
- Directory-creation modes (`mkdir`) — audit separately; not in this bug unless
  they share the `0o777` analogue (verify during Phase 1).

## Fix Design

Replace the `"438"` create-mode literal with `"384"` (0o600) at the three
file-creating sites. This mirrors the existing tempfile constant and needs no
signature or format change. If a future design wants caller-selectable
permissions it is a separate feature; the security default must be owner-only.
Rejected alternative: a post-open `fchmod` — racy and redundant when `open` can
set the mode directly.

## Phases

### Phase 1 — failing test + audit
- [ ] Add a rt-behavior test that creates a file and asserts mode `0o600`
      (masking out umask influence by checking `& 0o077 == 0`). Confirm it fails
      today.
- [ ] Grep every `open`/`openat`/`mkdir` create-mode literal; record each site's
      verdict (fix / already-0600 / directory-out-of-scope).

### Phase 2 — the fix
- [ ] Change the three `438` create-mode literals to `384`.

### Phase 3 — validation
- [ ] Full acceptance suite green; re-run the reproduction on Linux and macOS
      and confirm `0o600`.

## Validation Plan

- Regression test: the mode assertion above.
- Runtime proof: `ls -l` on a freshly written file shows `-rw-------` on both OSes.
- Full suite: `scripts/test-accept.sh`.

## Summary

A one-literal-per-site change with a clear correct value already used elsewhere
in the tree. The only diligence is confirming no legitimate path depends on the
group/other bits (none should) and checking `mkdir` modes in the same sweep.
