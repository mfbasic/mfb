# bug-593: a FAILING runtime-helper call grows a flat, unattributed block per call

Last updated: 2026-09-12
Effort: unknown — the attribution is the work; nobody has localized it yet
Severity: MEDIUM — unbounded growth in any retry loop over a call that fails
Class: Memory / correctness

Status: **Open — measured twice by other bugs, NOT yet reproduced by this filing.**
Regression Test: an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`, once reproduced.

## Why this is filed, and why it is not a duplicate

Two separate bugs measured the same shape, and each explicitly declined to own it:

- **bug-574** (`bugs/completed/bug-574-runtime-helper-string-argument-leaks-its-marshalled-block.md`)
  fixed the argument leak that scaled with length, and recorded that what was left on
  the failure path is *not* that leak:

  | program | before | after bug-574 |
  |---|---:|---:|
  | `net::lookup(<388-char host>)`, resolve failure | 2 801 B/call | 1 089 B/call |
  | `net::lookup(<10-char host>)`, resolve failure | 1 146 B/call | 1 032 B/call |

  > the LENGTH SCALING is gone … and the ~1 KB that remains is flat in the argument and
  > is **NOT attributed here**. It is not the orphaned `ErrorLoc` either … Whatever it is
  > survives both arena fixes.

- **bug-575** fixed the twelve `tls::` C-string marshalling leaks. Its agent then
  measured a failing `tls::connect` still growing **~260 B/call on Linux and ~1.9 KB/call
  on macOS, independent of host length**, and `tcp::connect` — already fixed by bug-574 —
  growing the same way. It concluded "that's the trapped-error path, not this bug" and
  did not file it.

Defect search before filing (not just a number check — see the 587/588/589 duplicate
withdrawal, `fdad98ccc`): `git grep` over `bugs/` and `bugs/completed/` for
failing/trapped connect growth, "flat in the argument", "resolve failure" and the
per-call figures found only bug-574's own "NOT attributed" note. Number verified free
across main, every worktree, and `git log --all --grep=bug-593`.

## The shape, as measured so far

- It appears on the **failure** path of a runtime helper that returns an `Error`
  (`net::lookup` resolve failure, a refused `tcp::connect` / `tls::connect`).
- It is **flat in the argument's length** — so it is not a marshalled argument
  (bug-574 / bug-575 already own those).
- It **survived** bug-573's fix (orphaned `ErrorLoc`) and both arena fixes in bug-574,
  unchanged within noise (1 089 → 1 056, 1 163 → 1 040 B/call).
- It differs by platform (~260 B Linux, ~1.9 KB macOS on `tls::connect`), which suggests
  a per-platform error-construction or error-message block rather than a shared one.
- bug-566's measurements are a useful contrast: `fs::readText` bound with **no** `TRAP`
  grows 129 B/call and that rate is unchanged by bug-566's fix, i.e. a plain successful
  helper call in that table also has a flat residual. Whether that is the same defect is
  unknown.

## Phase 1 — reproduce and attribute (do this before theorising)

1. Reproduce each shape above on the current tree at **>=200k iterations**, RSS with
   `--test-threads=1` (Linux RSS pins are ~4x leaner than macOS — calibrate per host).
2. Separate the variables: failing vs succeeding call; `TRAP`ped vs propagated; bound vs
   unbound error; one helper family (`net`) vs another (`tcp`/`tls`).
3. Only then localize. Candidates to test, not conclusions: the `Error` record built on
   the failure path, its message `String`, a platform error-text lookup, or a per-call
   scratch that the success path frees and the error path skips (the shape bug-575 found
   in the tls helpers).

If the growth does not reproduce, record the conditions tried and keep this open with
that evidence, rather than closing it — two independent measurements saw it.

## Memory gate (required when fixed)

1. the RED RSS pin flips flat;
2. name the contract in `.ai/collections.md` / `mfb spec` §14 the fix realizes, and show
   it only ADDS a free rather than moving a lifetime;
3. artifact-gate delta confined to emitting fixtures, everything else byte-identical,
   zero `.run` goldens moving;
4. a POSITIVE pin that a failing call still reports the correct `ErrorLoc` and message,
   and that a succeeding call is unchanged — the dangerous direction on an error path is
   freeing a block the propagated `Error` still refers to.
