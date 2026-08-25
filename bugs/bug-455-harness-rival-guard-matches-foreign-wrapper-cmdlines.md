# bug-455: artifact-gate/test-accept rival detection phantom-matches other sessions' shell-wrapper command lines (pgrep -f on argument text)

Last updated: 2026-08-25
Effort: small (<1h)
Severity: LOW
Class: Footgun (harness coordination; no miscompile)

Status: Open
Regression Test: — (Phase 1 adds a scripted repro: a sleeping `bash -c 'true # scripts/artifact-gate.sh'` decoy must NOT trip the guard)

The concurrency guards in `scripts/artifact-gate.sh` (and the same pattern in
`scripts/test-accept.sh`) detect a rival run with `pgrep -f '<script>\.sh'`,
which matches the **whole command line of any process** — including another
Claude session's `zsh -c "… eval '… scripts/artifact-gate.sh …'"` wrapper
whose gate hasn't even started (it was still in its `cargo build` stage when
the refusal fired). The result is a phantom "Another artifact-gate (pid N) is
running." abort against a process that holds no lock, and — observed live on
2026-08-25 — mutual-wait deadlocks between sessions politely queuing on each
other's *text*. The guards already exclude their own process group
(memory: `pgrep-self-match-guard.md` case 1); foreign wrapper text is the
remaining hole. **The single correct behavior a fix produces: the guard
matches only a process actually executing the script (and, ideally, in the
same repository), never a command line that merely mentions it.**

References:

- Memory note `pgrep-self-match-guard.md` (both self-match cases; this bug is
  the third, cross-session case appended there on 2026-08-25).
- `scripts/artifact-gate.sh:61-80` (the guard + its self-children comment).
- Observed during the optimizer worktree's loop-rows landing, coordinating
  with two concurrent sessions (mfb-50, mfb-81).

## Failing Reproduction

With any long-lived process whose argv merely *contains* the script path:

```
bash -c 'sleep 300 # scripts/artifact-gate.sh target/release/mfb all' &
bash scripts/artifact-gate.sh target/release/mfb all
```

- Observed: `Another artifact-gate (pid <decoy>) is running.` + exit 1,
  against a decoy that runs nothing.
- Expected: the gate runs (the decoy is not an artifact-gate).

Live occurrence: pid 46820 (`zsh -c … eval '… cargo build --release … &&
scripts/artifact-gate.sh …'`, still in its build phase) blocked this
session's gate twice; the workaround was hand-anchoring
`pgrep -f '^bash scripts/artifact-gate\.sh'` in every waiter.

## Root Cause

`scripts/artifact-gate.sh:71` — `pgrep -f 'artifact-gate\.sh'` matches full
argv text. A shell wrapper that *quotes* the future command (zsh `eval`
strings, `bash -c` bodies, `sh -c` CI steps) matches before/without executing
it. The existing process-group exclusion only covers the script's own forked
children, not foreign processes. `scripts/test-accept.sh` shares the pattern
(its guard message names test-accept pids the same way).

## Goal

- The decoy reproduction above runs the gate; a genuine concurrent
  `bash scripts/artifact-gate.sh` still trips the guard.

### Non-goals (must NOT change)

- Real mutual exclusion must stay — two genuine concurrent gates corrupting
  shared state is the disease the guard prevents (see
  `.ai/testing-gates.md` on the concurrent test-accept clobber). Do NOT
  weaken to "no guard".
- The guard's own-process-group self-exclusion (CI-proven, memory case 1)
  stays.

## Blast Radius

(actual `grep -rn "pgrep -f" scripts/` in Phase 1 — from this session's
reading:)

- `scripts/artifact-gate.sh:71` — fixed by this bug.
- `scripts/test-accept.sh` (same guard shape) — fixed by this bug.
- `scripts/test-appimage.sh`, `scripts/test-macapp.sh` — memory says same
  pattern; audit and fix alike.
- Ad-hoc waiter loops in sessions' own commands — out of scope (operator
  habit; the memory note now prescribes anchored patterns).

## Fix Design

Two candidate mechanisms; recommended is (a), possibly with (b) as belt:

(a) **Match the executing process, not the text**: for each pgrep candidate,
    read its argv0/argv1 (`ps -o command= -p $pid`) and require the argv to
    *begin* with the interpreter + script path (`^(ba)?sh .*scripts/<x>.sh`),
    or compare `lsof`-derived cwd to the repo. Cheap, portable to the macOS
    BSD userland already assumed by the scripts.

(b) **Replace process-sniffing with a real lockfile** (`mkdir`-based lock with
    stale-pid detection) — stronger, but changes failure modes (stale locks
    after SIGKILL) and touches the scripts' cleanup paths; bigger than the
    observed problem strictly needs.

## Phases

### Phase 1 — failing test + audit

- [ ] Scripted repro (decoy process) added under the harness's own test
      conventions (or a documented manual repro block in the script header if
      no script-test rig exists); confirm the phantom refusal today.
- [ ] `grep -rn "pgrep -f" scripts/` — fill the blast-radius list with a
      verdict per site.

Acceptance: decoy repro documented/failing; audit complete.
Commit: —

### Phase 2 — the fix

- [ ] Tighten candidate filtering in all audited scripts per design (a),
      keeping the process-group self-exclusion.

Acceptance: decoy repro passes; genuine-rival case still refuses (manual
two-terminal check); CI self-match tests still green.
Commit: —

### Phase 3 — full validation

- [ ] `cargo test --no-fail-fast` (golden.rs exercises the gate guard),
      full `test-accept.sh`, `artifact-gate.sh all`.

Acceptance: suites green; no phantom refusals across a session of concurrent
use.
Commit: —

## Validation Plan

- Regression: the decoy repro; the existing CI self-match coverage.
- Runtime proof: two concurrent real runs still exclude each other.
- Doc sync: update `pgrep-self-match-guard.md` memory when fixed.
- Full suite: as Phase 3.

## Open Decisions

- (a) argv-anchored filtering vs. (b) lockfile. Recommended (a) now; (b) only
  if stale-pid handling is wanted anyway.

## Summary

Small, contained scripting fix; the only risk is over-tightening (missing a
genuine rival spelled unusually), bounded by keeping the guard's refusal
default for ambiguous candidates.
