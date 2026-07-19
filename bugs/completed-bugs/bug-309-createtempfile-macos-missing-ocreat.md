# bug-309: `fs::createTempFile` fails on all macOS builds — open flags miscomputed, O_CREAT dropped

Last updated: 2026-07-18
Effort: small (<1h)
Severity: HIGH
Class: Correctness (platform)

Status: Fixed 2026-07-18
Regression Test: tests/rt-behavior (new) — `fs::createTempFile()` creates a file on macOS

`temp_file_open_flags` returns the literal `"16779266"` for the macOS branch =
0x1000802 = O_RDWR | O_EXCL | O_CLOEXEC. The intended value is O_RDWR | O_CREAT |
O_EXCL | O_CLOEXEC = 16779778; O_CREAT (0x200 = 512) is missing (16779778 −
16779266 = 512). `emit_open_file` is then called with this flag word on a
freshly-generated, non-existent UUID filename opened with O_EXCL but no O_CREAT, so
the kernel returns ENOENT and control falls to `open_error` → errno 2 →
`ERR_PATH_NOT_FOUND`. `fs::createTempFile()` therefore fails on every macOS build.
Linux is unaffected (`524482` is correct). This is a regression: bug-102 §1 recorded
the correct pre-fix value and prescribed "OR in 0x1000000" → 16779778, but the
applied fix miscomputed the constant and dropped O_CREAT.

The single correct behavior a fix produces: `fs::createTempFile()` creates and opens
a new temp file on macOS, exactly as on Linux.

References:

- `bugs/completed-bugs/bug-102-g10-runtime-low-cluster.md` §1 (correct value
  prescribed; the applied fix miscomputed it).
- Found during goal-06 review of `src/target/shared/code/fs_helpers_atomic.rs`.

## Failing Reproduction

```
RES f = fs::createTempFile()
```

- Observed (macOS aarch64, prebuilt `target/debug/mfb`): `Error: 7-705-0004 /
  Requested item, key, file, or resource was not found.`, exit 255.
- Expected: a temp file is created and opened.

## Root Cause

`src/target/shared/code/fs_helpers_atomic.rs:271` (`temp_file_open_flags`, non-linux
branch): the macOS literal `"16779266"` omits O_CREAT (0x200); the comment on
`:269` even writes the correct OR expression but the wrong decimal.

## Goal

- Change the macOS literal to `"16779778"` (O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC).

### Non-goals (must NOT change)

- The Linux flag word (`524482`, correct).
- `atomicWrite`/`writeText`/`writeBytes` (they use `mkstemps`/`open_flag_set`, not
  this function).

## Blast Radius

- `temp_file_open_flags` macOS branch — fixed here.
- Only `fs::createTempFile` uses this function (verified) — the sole affected
  builtin.

## Fix Design

Set the macOS literal to `16779778` and correct the comment's decimal. Rejected
alternative: computing the flags from named constants at codegen time — a larger
refactor; the literal fix is correct and minimal, but adding the OR-of-named-consts
would prevent recurrence (optional).

## Phases

### Phase 1 — failing test
- [ ] rt-behavior test creating a temp file; confirm it fails on macOS today.
### Phase 2 — the fix
- [ ] Correct the literal + comment.
### Phase 3 — validation
- [ ] Full suite green on macOS and Linux; repro creates a file.

## Validation Plan

- Regression: the createTempFile rt-behavior test (runs on macOS CI).
- Runtime proof: `fs::createTempFile()` succeeds on macOS.
- Doc sync: none.

## Summary

A one-constant arithmetic error drops O_CREAT, breaking `fs::createTempFile` on all
macOS builds; correcting the literal fixes it. HIGH because a shipped builtin is
entirely non-functional on a supported platform.

## Resolution

`temp_file_open_flags`'s macOS literal is now `16779778`. The comment beside it
already spelled the correct OR — `0x2|0x200|0x800|0x1000000` — but the decimal
written next to it had been evaluated without O_CREAT (0x200 = 512).

Verified both ways: `RES f = fs::createTempFile()` failed with
`Error: 7-705-0004 / Requested item, key, file, or resource was not found.`
(exit 255) before, and prints `created` at exit 0 after.

### The acceptance suite was green while this was completely broken

Three existing fixtures had the failure baked into their goldens as expected
output — `func_fs_createTempFile_valid`, `func_fs_isBuffered_valid` and
`func_fs_setBuffered_valid` each recorded

```
Error: 7-705-0004
Requested item, key, file, or resource was not found.
[exit 255]
```

so the suite passed while `fs::createTempFile()` failed on every macOS build.
Those goldens now record real behavior (`1 / TRUE / TRUE`, `FALSE / TRUE`, …).
This is worth remembering: a golden regenerated while a bug is live enshrines the
bug, and the suite then actively defends it. Two of the three fixtures had nothing
to do with temp files — they just used one as a scratch file — which is how the
breakage stayed invisible.

New fixture: `tests/rt-behavior/fs/fs-create-temp-file-rt`, which asserts the
create-and-close path directly rather than incidentally.
