# bug-172 — CLI robustness/security nits (predictable non-exclusive test temp dir, misleading install/update arity error, stale doc comment)

Last updated: 2026-07-12
Severity: LOW (batch).
Class: Security / Correctness / Dead-code.
Status: Open

## Findings

**A. `mfb test` writes and executes its binary from a predictable, non-exclusively-
created temp directory.** `src/cli/build.rs:711-719`. `make_temp_output_dir()`
builds `temp_dir()/mfb-test-<pid>-<nanos>` and calls `create_dir_all(&dir)` (result
discarded); the freshly linked test executable is written there and run
(`run_test_binary`, :382/:402). Unlike `stage_package_blob`/`write_new_file`
(which use `create_new`/`O_EXCL`), `create_dir_all` silently succeeds on a
pre-existing or symlinked directory an attacker planted at the predictable path,
redirecting where the executable is written and executed from. Mitigated by the
high-res `nanos` (attacker must win a race with a known/guessed pid). Fix: create
the temp dir with an exclusive/atomic primitive (fail on `AlreadyExists`, or a
randomized mkdtemp-style name).

**B. `mfb pkg install a b` / `update a b` report "unknown pkg command".**
`src/cli/pkg.rs:42-53, 105-107`. `install`/`update` only have `[command]` and
`[command, location]` arms and no `[command, ..]` arity arm (unlike `validate`,
`verify`, `check-abi`, `transfer`), so a 3+ element slice falls to the catch-all
"unknown pkg command `install`" instead of an arity/usage message. Fix: add
`[command, ..]` usage arms for install/update.

**C. Stale/self-contradictory doc comment on `transfer_accept`.**
`src/cli/pkg.rs:230-236`. Leftover stream-of-consciousness ("...read from the
local session...? No — ...falling back to prompting") that contradicts the
implementation (requires `<owner>#<package>@<to-owner>`, never prompts). Fix:
replace with a comment describing the actual `@<to-owner>` parsing.
