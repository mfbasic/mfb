# bug-470: `artifact-gate.sh` and `test-accept.sh` do not lock against each other, and share the fixture dump files

Last updated: 2026-08-30
Effort: small (one `pgrep` per guard) — but see "Status of the evidence"
Severity: MEDIUM (harness integrity; the failure mode is a silent flake, not an error)
Class: Test-harness race

Status: Open
Regression Test: — (a guard test would assert each script refuses while the
other is live; see "What a fix must produce")

## Status of the evidence: INFERRED, NOT REPRODUCED

This is a **code reading**, not an observed corruption. Two mutual-exclusion
guards were read and found not to cover each other, and the paths they write and
delete were read and found to overlap. Nobody has yet watched a run corrupt
another. Whoever picks this up should decide whether to chase a repro first — the
mitigation is small enough that proving the race may cost more than closing it.

Recording that distinction deliberately: a bug doc that says "reproduced" when it
means "inferred from two greps" is the same error class as a golden that says
"verified" when it means "regenerated".

## The two guards each match only their own script

```
$ grep -n "pgrep -f" scripts/artifact-gate.sh scripts/test-accept.sh
scripts/test-accept.sh:36:  for pid in $(pgrep -f 'test-accept\.sh'); do
scripts/artifact-gate.sh:71: for pid in $(pgrep -f 'artifact-gate\.sh'); do
```

Both guards are otherwise careful — they exclude their own process group so a
subshell does not self-match, and (bug-455) they require the script to be
`argv[0]`/`argv[1]` so a wrapper shell mentioning the path in a `-c` string does
not count. Neither, however, looks for the *other* script. So an `artifact-gate`
and a `test-accept` run concurrently in the **same worktree** proceed without
either noticing.

Cross-session this is harmless: each worktree owns its own `tests/`. The
unguarded case is **one session running both in one tree**, which is exactly what
a session does when it starts a sweep to save wall-clock while its
`cargo test` — whose `tests/golden.rs` step *is* an `artifact-gate all` — has not
yet exited.

## They share the per-fixture dump files (not `build/`)

The two scripts delete different things, and the overlap is easy to state
wrongly. `test-accept.sh` owns `build/`:

```
$ grep -n "remove_output_dir\|rm -rf" scripts/test-accept.sh
scripts/test-accept.sh:299:  rm -rf "$test_dir/build"
```

`artifact-gate.sh` never touches `build/` at all. It deletes the **dump files
beside the fixture source**:

```
$ grep -n "rm " scripts/artifact-gate.sh
scripts/artifact-gate.sh:153:  rm -f "$td/$pkg".{ast,ir,hex,nir,nplan,nobj,ncode,mir} 2>/dev/null
scripts/artifact-gate.sh:163:  rm -f "$td/$pkg".{ast,ir,hex,nir,nplan,nobj,ncode,mir} 2>/dev/null
scripts/artifact-gate.sh:179:  rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
scripts/artifact-gate.sh:206:  rm -f "$td/$pkg".{nir,nplan,nobj,ncode,mir} 2>/dev/null
```

`test-accept.sh` produces those same dumps — that is how it compares a fixture's
`.ir`/`.ast` goldens. So the contended resource is
`tests/<fixture>/<pkg>.{ast,ir,hex,nir,nplan,nobj,ncode,mir}`, written by both
and deleted by one.

## Why the symptom is nastier than a diff

A stray `rm -f` landing between the other run's build and its compare removes the
actual before it is read. The harness then reports **"missing actual"** — not a
content mismatch. That matters because the two failures get different treatment
by a human: a diff gets investigated, a missing/absent artifact reads as a flake
and gets re-run. So a corrupted run is likely to be silently retried rather than
noticed, and the retry (uncontended) passes, confirming the "flake" reading.

This compounds an already-known hazard: `.ai/testing-gates.md` "Concurrency &
macOS hazards" documents that two artifact-gates must not overlap, and that a
killed run leaves stray untracked dump files behind. The gate-vs-accept case is
the same family and is not covered there.

That a harness can report success about work it did not do is not theoretical
here. While bug-457 was being fixed in this same tree, a full
`cargo test --release --no-fail-fast` was green and `artifact-gate all` reported
its expected 2 diffs and nothing else, while `tests/acceptance` was in fact dying
on a signal partway through — `cargo test` does not run the acceptance harness,
and the execution-free gate skips `tests/acceptance` outright for want of a
`golden/` dir. That is a *different* mechanism from this bug (coverage the
harness never had, rather than an artifact deleted mid-run), but it is the same
shape and the same consequence: two green gates over a real failure. It is the
concrete reason to treat "an absent artifact reads as a flake" as a live risk
rather than a tidy hypothesis.

## Failing reproduction

None yet — see "Status of the evidence". A candidate shape, untested:

1. In one worktree, start `bash scripts/artifact-gate.sh target/release/mfb all`.
2. Immediately start `bash scripts/test-accept.sh target/release/mfb /tmp/scratch`.
3. Expect: both proceed (neither guard fires), and one reports missing actuals or
   spurious diffs on fixtures the other deleted mid-flight.

Note step 2 must not pass a real directory as the second argument — it is an
`rm -rf` scratch path.

**A failed repro attempt proves nothing here.** The race needs two runs to touch
one dump file within the window between the other's build and its compare, in a
tree you are deliberately corrupting; missing that window is the expected
outcome, not evidence of safety. Set against that, the mitigation is a one-line
`pgrep` widening in each guard, reusing filtering both scripts already have. That
asymmetry — an unfalsifiable-in-practice repro against a near-free fix — is the
argument for closing this without a reproduction, and nobody picking it up should
feel obliged to chase one first.

## What a fix must produce

Each guard refuses while *either* script is live.

The cheap version is to widen each guard's `pgrep` pattern to the other script's
name, reusing the filtering both already have (process-group self-match
exclusion, the bug-455 `argv[0]`/`argv[1]` wrapper test) and keeping the distinct
refusal exit code `98` so callers can still tell "refused" from "found diffs"
(`1`). `tests/golden.rs:39` already branches on that refusal and reports
"nothing was checked", so the caller side needs no change.

**But note this narrows the window rather than closing it.** Both guards are
`pgrep`-then-proceed with no atomicity: a run that observes a free lock has
learned something about the past, not made a claim about the present, and two
runs that check at the same moment both proceed. The loser is whoever *calls*
second, regardless of who started first. So a widened `pgrep` reduces the
exposure from "the whole of the other run" to "the check-to-start window" — a
real improvement, and possibly enough — while a genuine fix needs an atomic
acquire (an `O_EXCL` lock file or `flock` on a shared path, released on EXIT as
`test-accept.sh:105` already does for `$MFB_HOME`).

Whoever takes this should decide which of the two they are buying; the doc's
"Effort: small" line refers to the `pgrep` widening only.

A guard test should assert the refusal in both directions rather than only the
new one, so the existing same-kind guard cannot regress unnoticed.

## Blast radius

Any session that runs `cargo test` (which reaches `artifact-gate all` through
`tests/golden.rs`) and `test-accept.sh` concurrently in one worktree. Today the
only thing preventing it is every session remembering to serialise by hand,
which is exactly what this pair of sessions had to do to avoid it.

References: `scripts/artifact-gate.sh:71,153,163,179,206`;
`scripts/test-accept.sh:36,299`; `tests/golden.rs` (the `artifact_gate_all` test
that shells out to the gate); `.ai/testing-gates.md` §"Concurrency & macOS
hazards" (documents the gate-vs-gate case, not this one).

Credit: the cross-guard gap was spotted by a peer session (mfb-a3) while
serialising its own runs, as was the check-then-act consequence for the fix
design; the shared-path detail and the missing-actual symptom were established
here by reading the `rm` sites.

Observed while filing this: two sessions' `cargo test` runs did collide on the
gate lock, and `tests/golden.rs` reported it correctly and unambiguously
("could not START: another gate run holds the lock. This is NOT a golden
regression -- nothing was checked"). That is the *same-kind* guard working as
designed — evidence that the refusal path is sound and only its coverage is
missing, not that the guard is broken.
