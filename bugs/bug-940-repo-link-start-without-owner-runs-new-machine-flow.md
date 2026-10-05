# bug-940: `mfb repo link --start` with no owner runs the new-machine pairing flow with owner `--start`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: `src/cli/repo.rs` tests module (beside the existing `link` usage assertions)

`mfb repo link --start <owner>` (old machine) shows a pairing code, and `mfb repo
link <owner>` (new machine) reads one from stdin. When `<owner>` is left off the
old-machine form, `mfb repo link --start` matches the new-machine arm with
`owner = "--start"`. It prints `Enter the pairing code shown on the linked
machine:` and blocks on stdin. Only after reading a line does it fail, with
`error: invalid owner name: must start with a letter or underscore`, and it exits
`1`.

A user on the old machine who forgot the owner is told to type a code from a
machine that has none, and gets a runtime error, not a usage error. **Correct
behavior:** `repo link --start` with no owner is the usage error `mfb repo link
requires `--start <owner_name>` (old machine) or `<owner_name>` (new machine)`,
exits `2`, reads nothing and makes no network call. The same applies to any owner
operand starting with `-`.

References:

- `src/docs/spec/tooling/07_cli-reference.md` command table: `repo link` | `mfb repo
  link [--start] <owner_name>` | `0 ok; 2 usage; 1 failed`. *pkg and repo
  Subcommands*: "`link --start <owner>` (old machine, needs a session) displays a
  one-time pairing code; `link <owner>` (new machine) reads the code from stdin".
- Found during hardening batch B14.

## Failing Reproduction

```
export HOME=/tmp/fakehome MFB_REPO_URL=http://127.0.0.1:1
mfb repo link --start </dev/null; echo "exit=$?"
```

- Observed (binary /tmp/hardening/mfb-head, HEAD 28a15a153):
  ```
  Enter the pairing code shown on the linked machine:
  error: invalid owner name: must start with a letter or underscore
  exit=1
  ```
- Expected: `error: mfb repo link requires `--start <owner_name>` (old machine) or
  `<owner_name>` (new machine)` on stderr, nothing on stdout, `exit=2`.

Contrasts that are correct today: `mfb repo link` → that usage error, exit 2.
`mfb repo link --start a b` → the same usage error, exit 2.

## Root Cause

`src/cli/repo.rs:run_repo_command`, the `"link"` arm, matches the slices
`[_, flag, owner] if flag == "--start"` and then `[_, owner]`. The second pattern
has no guard, so `["link", "--start"]` binds `owner = "--start"`. The owner-name
validation runs later, inside the client, after the prompt is printed and stdin is
read.

## Goal

- `[_, owner]` where `owner` starts with `-` is `RepoCommandError::Usage` with the
  link usage text (exit 2), before any output, stdin read or network call.

### Non-goals (must NOT change)

- `link --start <owner>` and `link <owner>` behavior for valid owners.

## Blast Radius

Every other `<owner_name>` positional takes any word: `repo register`, `repo auth`,
`key rotate`, `machine revoke`. A flag-looking owner such as `--frob` there fails
later as an invalid owner name with exit 1. That is latent and lower impact,
because those commands do not prompt first. Worth fixing with the same guard.
