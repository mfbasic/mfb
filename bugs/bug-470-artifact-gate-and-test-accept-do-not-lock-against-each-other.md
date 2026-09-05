# bug-470: `artifact-gate.sh` and `test-accept.sh` do not lock against each other, and share the fixture dump files

Last updated: 2026-08-30
Effort: small (one `pgrep` per guard) — but see "Status of the evidence"
Severity: MEDIUM (harness integrity; the failure mode is a silent flake, not an error)
Class: Test-harness race

Status: **FIXED** (2026-09-05)
Regression Test: `tests/gate_mutual_exclusion.rs` — five cases: same-tree
refusal in BOTH orderings, cross-tree non-interference, stale-holder reclaim,
and same-process-tree re-entrancy.

## Status of the evidence: BOTH HALVES NOW REPRODUCED (2026-09-05)

This section previously read "INFERRED, NOT REPRODUCED" — a code reading, with
the note that the race might cost more to reproduce than to close. That caution
was right to record and is now superseded: both halves were observed end to end
against the pre-fix scripts, in about two minutes, using `byte-identity/bits` to
keep each run short.

**Half 1 — too narrow (the filed bug).** In ONE tree at `527eca202`, with
`test-accept.sh` live, `artifact-gate.sh` ran to completion:

    $ bash scripts/test-accept.sh target/release/mfb /tmp/scratch 'byte-identity/bits*' &
    $ bash scripts/artifact-gate.sh target/release/mfb bits
    artifact-gate [bits]: 1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
    GATE_EXIT=0

Neither guard fired. Both runs were regenerating and deleting the same fixture
dumps in the same tree.

**Half 2 — too broad.** With that same `test-accept.sh` live in `/tmp/gate-red`,
a `test-accept.sh` in the SEPARATE worktree `/tmp/wt-546` was refused:

    OTHER_TREE_EXIT=98
    Another test-accept (pid 60169) is running.

Two trees with entirely separate `tests/` directories, refused for no reason.

**After the fix, both answers invert**, verified with the real scripts:

    A. same tree, other script  -> EXIT=98
       "Refusing to run: test-accept.sh (pid 64395) holds
        /tmp/wt-470/tests/.gate.lock"
    B. different tree, same script -> EXIT=0, gate completed normally

So the fix does not merely add exclusion; it moves the guard's axis from the
script's NAME to the TREE it mutates, which is the one dimension both old
answers were missing.

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

**Reproduced 2026-09-05 — see "Status of the evidence" above for the observed
output of both halves.** The candidate shape below was written before that and
is exactly what worked, so it is kept as the recipe:

1. In one worktree, start `bash scripts/artifact-gate.sh target/release/mfb all`.
2. Immediately start `bash scripts/test-accept.sh target/release/mfb /tmp/scratch`.
3. Expect: both proceed (neither guard fires), and one reports missing actuals or
   spurious diffs on fixtures the other deleted mid-flight.

Note step 2 must not pass a real directory as the second argument — it is an
`rm -rf` scratch path.

**A failed repro attempt proves nothing here** — and note what WAS and was not
shown. The observation above is that **neither guard fires**, which is a
deterministic property of the guards and reproduces every time. It is NOT an
observation of an actual corrupted artifact: that still needs two runs to touch
one dump file inside the window between the other's build and its compare, and
missing that window is the expected outcome, not evidence of safety. The
distinction matters, because the guard gap is what the fix closes. Set against that, the mitigation is a one-line
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

### Second defect, 2026-08-31: the guard is also too BROAD — it serializes across worktrees

Found while deciding the above. This document describes the guard as too narrow
(it misses the sibling script). It is **also too broad**, and the two are the
same missing dimension: the guard keys on the script's *name*, never on *which
tree it belongs to*.

```
$ sed -n '/case "$ca0" in/,/esac/p' scripts/test-accept.sh
  case "$ca0" in
    */test-accept.sh|test-accept.sh) ;;
```

`*/test-accept.sh` matches **any** path. A run in
`.claude/worktrees/467/scripts/test-accept.sh` therefore blocks a run in
`.claude/worktrees/474/` — two trees that own entirely separate `tests/`
directories and cannot corrupt one another. This document already says so in
"The two guards each match only their own script": *"Cross-session this is
harmless: each worktree owns its own `tests/`. The unguarded case is one session
running both in one tree."* The guard enforces the opposite of that analysis.

**Consequence, and why it matters for the fix's shape.** Nine worktrees were
live during the session that recorded this (four `/fix-bug` agents, three peer
sessions, main). Every cross-worktree pair is a spurious refusal (`exit 98`) or a
wait — pure lost throughput, on the exact workload the harness exists to serve.

This also **rules out a machine-wide lock** as the fix. An `flock` on a fixed
global path would make the false serialization *worse* and permanent: correct
against corruption, but it would serialize nine independent trees that were never
in conflict.

**So the acquire must be per-tree.** Put the lock inside the tree the run
mutates — e.g. `"$REPO_ROOT/tests/.gate.lock"`, with `REPO_ROOT` derived from
the script's own location (`git rev-parse --show-toplevel`, or `cd "$(dirname
"$0")/.."`), not from `$PWD`. Then:

* two runs **in one tree** (either script, either order) → one refuses; the filed
  bug is fixed;
* two runs **in different trees** → both proceed, which is correct and is what
  happens by accident today only because the `pgrep` race lets them through.

A guard test should assert **both** directions: same-tree cross-script refusal,
and cross-tree non-interference. The second is the one that would have caught
this, and neither exists today.

Status of this evidence: a code reading, like the rest of this document — the
`case` arm above is conclusive on its own, but nobody has timed the lost
throughput.

### Decision, 2026-08-31 (coordinator session mfb-59): buy the atomic lock

The doc leaves this open. Deciding it here with evidence gathered while running
four concurrent `/fix-bug` agents against this tree, so whoever implements it
does not have to re-derive the call.

**Take the `flock`/`O_EXCL` acquire, not the `pgrep` widening alone.** Reasons,
in order of weight:

1. **The concurrency this guard faces is no longer two runs.** `git worktree
   list` reported **9 worktrees** during this session — four `/fix-bug` agents,
   three peer sessions (bug-471, bug-480, P-98) and the main checkout. Every one
   of them runs the full suite, and `tests/golden.rs` invokes `artifact-gate.sh`.
   The check-to-start window is not sampled twice; it is sampled continuously by
   ~8 independent writers. A window that is "narrow" against one competitor is
   not narrow against eight.
2. **The failure is already known to occur in practice.** It has its own
   standing memory note — a contended gate reports phantom diffs, and the
   documented mitigation is "re-run uncontended and ask peers", i.e. humans
   currently absorb the race by hand. That is the cost the widening leaves in
   place.
3. **The phantom-diff failure mode is the expensive kind**: it does not error,
   it produces a *plausible wrong answer* (a diff list) that an operator may act
   on — reverting a correct change, or filing an arch-scoped batch as noise. Both
   are worse than a refusal.
4. **Neither script has any lock primitive today** — `grep -n
   'flock\|lockfile\|O_EXCL' scripts/artifact-gate.sh scripts/test-accept.sh`
   returns nothing — so this is additive, not a rewrite. `test-accept.sh:105`
   already has an `EXIT` trap releasing `$MFB_HOME`, which is the release hook a
   lock file can reuse.

Keep the `pgrep` widening too, as a **diagnostic**, not as the mechanism: it is
what lets the refusal message name the competing script and PID instead of just
"busy". Keep exit code `98` for refusal (`tests/golden.rs:39` branches on it).

macOS has no `flock(1)`, so a portable acquire is `mkdir` on a lock dir or
`set -o noclobber` + `O_EXCL` on a lock file, released from the existing `EXIT`
trap — and the release must be robust to a killed run (record the holder PID in
the lock and treat a lock whose PID is gone as stale).

A guard test should assert the refusal in both directions rather than only the
new one, so the existing same-kind guard cannot regress unnoticed.

## Third contender, found by audit after the fix (2026-08-31)

The title names two scripts. A completeness sweep for other guards and other
writers found a **third**, which the document does not mention and which the
two-script fix would have left open:

```
$ grep -rln "pgrep" scripts/
scripts/artifact-gate.sh  scripts/test-accept.sh  scripts/test-accept-selftest.sh
scripts/test-appimage.sh  scripts/test-macapp.sh
```

`scripts/sync-goldens.sh` has **no guard at all**, yet it is a golden *writer*:
it invokes `test-accept.sh` (`:28`) and then copies the produced artifacts over
the committed goldens (`:45`, `cp "$adir/$name" "$gf"`). With the lock added,
the invoked `test-accept.sh` acquires and **releases** — so the copy runs
unlocked, and an `artifact-gate.sh` starting in that window reads half-written
goldens. Same corruption class, longer window, and it writes the *committed*
files rather than scratch artifacts.

Locking it naively self-deadlocks: the `test-accept.sh` it spawns refuses its
own parent. Hence the **re-entrancy** rule in `gate_lock_acquire` — the owner pid
is exported, a child that finds the live lock already owned by its process tree
neither takes nor releases it, and the parent holds across both phases. The
exported pid is re-checked against the live lock so a stale value from an
earlier run in the same shell cannot wave a caller through.

**Left deliberately alone**, with reasons, so the next reader does not re-derive
them: `test-accept-selftest.sh` is a distinct lightweight harness the bug-455
`.sh` anchor already excluded on purpose; `test-appimage.sh` and
`test-macapp.sh` are packaging tests with their own guards that do not write
fixture dumps. If any of those three ever starts writing under `tests/`, it
needs the same `gate_lock_acquire` call — one line, and this note is the pointer.

## What landed

- `scripts/gate-lock.sh` — atomic `mkdir` acquire (no `flock(1)` on macOS),
  per-tree by construction, tree derived from the script's own location rather
  than `$PWD`, stale-holder reclamation by recorded pid, re-entrant for the
  nesting case, refusal still exit `98` and now naming the rival and tree.
- `artifact-gate.sh`, `test-accept.sh`, `sync-goldens.sh` all acquire; ~40 lines
  of `pgrep`/PGID/argv matching deleted from each of the first two.
- `tests/gate_mutual_exclusion.rs` — five tests. Each was verified RED against
  exactly the implementation it rejects and no other: a no-op lock fails the two
  same-tree tests and the nesting test; a machine-wide lock fails only the
  cross-tree test.

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
