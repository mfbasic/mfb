# bug-577: uncaught runtime errors exit with status 0

Last updated: 2026-09-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: pending `tests/cli/`

An executable that reaches an uncaught MFBASIC runtime error prints the correct
diagnostic but returns a successful host exit status. The correct behavior is:
every uncaught runtime error exits nonzero, while successful programs continue
to exit zero. Shell callers cannot reliably distinguish the two today.

References:

- `planning/plan-128-B-cli-package.md` Phase 2 acceptance (nonzero error exits).

## Failing Reproduction

```
target/debug/mfb build /tmp/plan128-cli-consumer.x69gA2
/tmp/plan128-cli-consumer.x69gA2/build/cli_smoke.out -p
```

- Observed: `Error: 7-705-0002` and `cli: option \`-p\` needs a value`, but host
  status is 0.
- Expected: the same diagnostic and a nonzero host status.

| Environment | Result |
| --- | --- |
| macOS aarch64 | fails: uncaught `ErrInvalidArgument` exits 0 |

## Root Cause

Unknown. The likely fault is the generated runtime error termination path, not
`packages/cli`: `parseArgs` raises `errorCode::ErrInvalidArgument`, and the
diagnostic is correct. Audit the native entry/error exit sequence before
changing CLI behavior.

## Goal

- Uncaught runtime errors always produce a nonzero process status.

### Non-goals (must NOT change)

- Do not rewrite CLI tests to accept status 0 or change caught `TRAP` behavior.

## Blast Radius

- Every emitted executable with an uncaught runtime error — potentially affected;
  audit the common runtime exit path before implementation.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a CLI/runtime regression asserting nonzero status for an uncaught error.
- [ ] Audit the generated entry and runtime error termination paths.

Acceptance: regression fails with status 0 and the shared exit path is identified.
Commit: —

### Phase 2 — the fix

- [ ] Set a nonzero native process status on every uncaught runtime-error path.

Acceptance: regression passes without changing caught-error behavior.
Commit: —

### Phase 3 — validation

- [ ] Run full compiler, acceptance, and artifact gates.

Acceptance: full gates pass and the reproduction exits nonzero.
Commit: —
