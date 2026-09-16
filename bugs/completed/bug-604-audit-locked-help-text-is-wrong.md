# bug-604: `mfb audit --help` misdescribes `--locked` and names a lockfile that does not exist

Last updated: 2026-09-12
Effort: small (<1h)
Severity: LOW
Class: Documentation (CLI help text)

Status: Fixed
Regression Test: src/cli/help.rs (`cli::help::tests::audit_help_describes_locked_as_the_lockfile_gate`)

> **STATUS: FIXED (f03beaa31)** — `mfb audit --help` now says `--locked  Treat a missing or stale mfb.lock as an error, not a warning`, and no help text names `project.lock`. The RED test (a content assertion on `AUDIT_HELP`) and the one-line fix landed in one commit; RED at `9b5e5b55f`, GREEN after. No deviation from the documented fix.

`mfb audit --help` says:

```
  --locked            Only audit packages defined in project.lock
```

Both halves are wrong. The lockfile is `mfb.lock` (`mfb --help`: "pkg update —
Resolve dependencies and write mfb.lock"; `src/audit/text.rs` tests assert
`stale lock (mfb.lock)`), and `project.lock` appears nowhere else in the tree.
`--locked` does not narrow which packages are audited: it makes a missing or
stale lockfile an **error** instead of a warning
(`mfb spec tooling audit-format`, Invocation and Exit Status and the
`AUDIT-LOCK-*` catalogue rows; `src/audit/collect/findings.rs:lockfile_findings`).

**The single correct behavior a fix produces:** `mfb audit --help` describes
`--locked` as "Treat a missing or stale mfb.lock as an error, not a warning", and
no help text names `project.lock`.

Found by plan-125-B Phase 1, while writing the new `mfb man tooling audit` page.
**Filed, not fixed**, by user instruction during a documentation-only plan
("file all bugs, make no fixes"). The man page documents the real behavior.

## Reproduction

```
mfb init /tmp/demo
mfb audit --help | grep locked
#   --locked            Only audit packages defined in project.lock   <- observed
mfb audit --locked /tmp/demo; echo "exit=$?"
#   ... error AUDIT-LOCK-MISSING ...  exit=1                           <- the real behavior
mfb audit /tmp/demo; echo "exit=$?"
#   Lockfile: absent ... exit=0
```

Expected: the help line describes what the last two commands show.

## Root cause

`src/cli/help.rs:AUDIT_HELP` is a hand-written string that was never updated when
the lockfile was named `mfb.lock` and `--locked` got its current meaning
(`src/audit/mod.rs:parse_options` sets `locked`; the lockfile findings consume it).
Nothing tests the help text against the option's behavior.

## Non-goals

- Changing what `--locked` does. The behavior matches the spec; only the text is wrong.
- Renaming the lockfile.

## Blast-radius audit

- `rg -n 'project\.lock' src` — `src/cli/help.rs` is the only hit. Fixed here.
- Other `*_HELP` strings in `src/cli/help.rs` describing option semantics — unaudited;
  out of scope, recorded here so a fix can grep them.

## Fix

Phase 1 — a test that `AUDIT_HELP` contains `mfb.lock` and not `project.lock` (RED).
Commit: f03beaa31

Phase 2 — correct the `--locked` line in `src/cli/help.rs:AUDIT_HELP` (GREEN); run
the `cli` tests. Commit: f03beaa31

## Phase 1 findings (fix-bug, 2026-09-15)

- Reproduced at main `9b5e5b55f` with `target/release/mfb`: `mfb audit --help | grep locked`
  prints `Only audit packages defined in project.lock`; `mfb audit --locked` on a fresh
  `mfb init` project exits 1 with `AUDIT-LOCK-MISSING mfb.lock is required by --locked`;
  plain `mfb audit` exits 0. Mechanism as documented: `src/cli/help.rs:AUDIT_HELP`.
- RED test: `cli::help::tests::audit_help_describes_locked_as_the_lockfile_gate`.
