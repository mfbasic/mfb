# bug-281: a corrupt/unparseable `mfb.lock` audits clean, even under `--locked`

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (security-tooling gap)

Status: Open
Regression Test: tests/ audit fixture (new) — a malformed mfb.lock produces a finding and errors under `--locked`

When `mfb.lock` exists but cannot be read/parsed (I/O error, invalid JSON, or not
a JSON object), `collect_lockfile` returns `present: true,
project_hash_matches: None`. `lockfile_findings` only emits `AUDIT-LOCK-MISSING`
on `!present` and `AUDIT-LOCK-STALE` on `Some(false)`, so the `None` case produces
no finding at all and `mfb audit --locked .` exits 0. A lockfile whose hash cannot
be validated is strictly worse than a stale one, yet it ranks better — silently
satisfying `--locked` (documented as "treat a missing/stale lockfile as an
error").

The single correct behavior a fix produces: a present-but-malformed lockfile
yields a distinct finding (e.g. `AUDIT-LOCK-MALFORMED`) — an error under `--locked`,
a warning otherwise.

References:

- `src/docs/spec/tooling/07_cli-reference.md` (`--locked` row).
- `bugs/completed-bugs/bug-25-*` (fixed the `lockfileVersion` narrowing at these
  lines; proposed but did not add a malformed-lockfile finding).
- Found during goal-06 review of `src/audit/collect/lockfile.rs`.

## Failing Reproduction

```
echo "totally not json {" > mfb.lock
mfb audit --locked .
```

- Observed: `Lockfile: present [--locked]`, `errors: 0`, exit 0.
- Expected: a malformed-lockfile finding; non-zero exit under `--locked`.

## Root Cause

`src/audit/collect/lockfile.rs:22-37` (`collect_lockfile`) collapses all
read/parse failures to `project_hash_matches: None` with `present: true`;
`src/audit/collect/findings.rs:3-40` (`lockfile_findings`) has no arm for the
`None`/malformed state.

## Goal

- `LockfileSummary` records a distinct malformed state (e.g. `parsed: bool`).
- `lockfile_findings` emits `AUDIT-LOCK-MALFORMED` — error under `--locked`,
  warning otherwise.

### Non-goals (must NOT change)

- The MISSING and STALE findings and their severities.
- The lockfile format.

## Blast Radius

- `collect_lockfile` + `lockfile_findings` — fixed here.
- No other consumer distinguishes the three lockfile states.

## Fix Design

Add a `parsed`/`malformed` flag to `LockfileSummary`; set it when read/parse
fails. Add the `AUDIT-LOCK-MALFORMED` finding with `--locked`-aware severity.
Rejected alternative: reusing `AUDIT-LOCK-STALE` — misleading; malformed ≠ stale.

## Phases

### Phase 1 — failing fixture
- [ ] Fixture with a malformed `mfb.lock`; assert a finding + non-zero exit under
      `--locked`. Confirm it passes clean today.
### Phase 2 — the fix
- [ ] Add the malformed state + finding.
### Phase 3 — validation
- [ ] Regenerate audit goldens; full suite green.

## Validation Plan

- Regression: the new fixture (with and without `--locked`).
- Doc sync: note `AUDIT-LOCK-MALFORMED` in 04_audit-format.md / 07_cli-reference.md.

## Summary

A malformed lockfile currently ranks better than a stale one; adding a distinct
finding closes the `--locked` bypass. Small, localized change.
