# bug-854: when a callee `thread::transfer`s a caller's `RES` argument, the caller's scope drop still closes it and logs a spurious `Cleanup failure: 7-703-0009`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/resources/callee-transfer-no-cleanup-failure-rt (new) — name suggested

A function that receives `RES f` and transfers it (`thread::transfer(t, f)`) moves the caller's
resource at run time (the statically untracked case §14.9 describes). At the caller's scope
exit, the cleanup close of `f` returns `ErrResourceMoved`; the drop path treats only
`ErrResourceClosed` as benign, so it records a secondary cleanup failure. On an error exit the
entry banner then prints `Cleanup failure: 7-703-0009 / Resource handle was moved to another
thread...` after the real error. The failure is recorded on the success path too (just not
printed), polluting the cleanup-failure audit.

Correct behavior: the sender's cleanup of a moved handle is a no-op — never a close and never a
recorded cleanup failure.

References:

- src/docs/spec/language/15_resource-management.md transfer-outcome table: "Successful
  continuation | Sender views are moved; ... **Sender cleanup must not close it.**"; line ~304:
  re-close during drop "is a benign no-op — and is never reported as a drop-close failure."
- src/docs/spec/language/14_memory-semantics.md §14.9: "A transfer through a name the check
  cannot connect to the caller's binding — a callee transferring its `RES` parameter — is still
  refused at run time: a later guarded operation raises `ErrResourceMoved`."

## Failing Reproduction

```
IMPORT io
IMPORT fs
IMPORT thread
ISOLATED FUNC w(t AS ThreadWorker OF String RES fs::File TO Integer, seed AS String) AS Integer
  RES g AS fs::File = thread::accept(t)
  RETURN fs::size(g)
END FUNC
FUNC send(RES t AS Thread OF String RES fs::File TO Integer, RES f AS fs::File) AS Nothing
  thread::transfer(t, f)
END FUNC
FUNC main() AS Integer
  RES t AS Thread OF String RES fs::File TO Integer = thread::start(w, "s")
  RES f AS fs::File = fs::createTempFile()
  send(t, f)
  io::print("w=" & toString(thread::waitFor(t)))
  FAIL error(77050002, "boom")
  RETURN 0
END FUNC
```

(`/tmp/hardening/B03/memory/src/tr_i.mfb`)

- Observed:
  ```
  w=0
  Error: 7-705-0002
  boom
  Cleanup failure: 7-703-0009
  Resource handle was moved to another thread by `thread::transfer` and is no longer usable by the sender.
  ```
- Expected: the first three lines only.

Contrast: the same program with `thread::transfer(t, f)` inline in `main` (`tr_h.mfb`) prints
no cleanup failure — there the static move retires the cleanup
(`retire_moved_resource_cleanup`). Reading `f` after `send` (`tr_f.mfb`) correctly raises
`ErrResourceMoved` (spec-conformant), then also prints the spurious banner.

## Root Cause

`src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_cleanup_call` calls the
close helper and compares the result only against `ErrResourceClosed` before
`record_secondary_cleanup_failure()`. The fs close helper (`src/codegen/builtins/fs/gen_handle.rs`)
returns `ErrResourceMoved` for a moved record (bit `RESOURCE_MOVED_BIT`), which falls through to
"genuine failure".

## Goal

- Drop of a moved handle records no cleanup failure; the banner is absent in the repro.

### Non-goals (must NOT change)

- Explicit `fs::close(f)` / guarded ops on a moved handle keep raising `ErrResourceMoved`.
- Genuine drop-close failures stay recorded.

## Blast Radius

- `emit_resource_cleanup_call` — fixed by this bug (also treat `ErrResourceMoved` as benign, or
  test the moved bit before the call as the reclaim path already does).
- LINK-declared resources' close thunk (`src/codegen/link/thunk/link_thunk.rs`, also reports
  moved) — same path, same fix.

## Fix Design

Skip the close call when the record's moved bit is set (mirrors the reclaim guard just below),
or add `ErrResourceMoved` to the benign comparison.

## Phases

### Phase 1 — failing test
- [ ] rt fixture asserting stderr has no `Cleanup failure` line.
Commit: —
### Phase 2 — the fix
- [ ] Moved-bit guard in `emit_resource_cleanup_call`.
Commit: —
### Phase 3 — full validation
- [ ] Full suite.
Commit: —

## Validation Plan

- Regression test(s): above. Runtime proof: repro banner gone. Doc sync: none. Full suite.

## Summary

Small drop-path predicate gap; noise in the error banner and audit ledger, no resource misuse.
