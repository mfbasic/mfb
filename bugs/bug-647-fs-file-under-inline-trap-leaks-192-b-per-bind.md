# bug-647: an `fs::File` bound through an inline TRAP leaks 192 B per bind

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

`RES f AS fs::File = fs::openFile(...) TRAP(e) ... END TRAP` leaves 192 B live per bind. The
non-`TRAP` spelling of the same program leaks 96 B per bind. Both are residuals left standing
by bug-643's fix, which removed one 96 B record from this shape (288 → 192 B per iteration)
but could not remove these two.

**The single correct behavior a fix produces:** a loop of `fs::openFile` under an inline
`TRAP` reports equal `live_bytes` at N and 2N, with `double_free_skips 0` and every handle
still closed exactly once.

References:

- bug-643 (`61a688e31`), which measured this while fixing the sendable-resource half.
- `builder_resource_cleanup.rs:48` — the `resource_record_freed_at_drop` exclusion.
- `.ai/resources-packages.md`, "Canonical resource-record header".

## Failing Reproduction

```
IMPORT io
IMPORT fs

FUNC readOne(path AS String) AS Integer
  RES f AS fs::File = fs::openFile(path) TRAP(e)
    RETURN 0
  END TRAP
  RETURN 1
END FUNC

SUB main()
  MUT ok AS Integer = 0
  FOR i = 1 TO {n}
    ok = ok + readOne("/etc/hosts")
  NEXT
  io::print("ok=" & toString(ok))
END SUB
```

`mfb build --debug`:

- Observed (subagent measurement at bug-643's branch, 200 iterations): 38400 B live, i.e.
  192 B per bind. Before bug-643's fix the same probe read 57600 B (288 B per bind).
- Expected: equal `live_bytes` at both N.

**Not yet reproduced on the main thread at these exact counts** — confirm in Phase 1, and
pick N so the growth exceeds `rt_debug_soak.rs`'s `BLOCK_BOUND` (4096): at 192 B per bind,
N=100/200 grows 19200 B, comfortably above it.

## Root Cause

Partly known, to confirm and split in Phase 1. `resource_record_freed_at_drop` deliberately
excludes `fs::File` — "`fs::File` shares the record with the drop's buffer reclaim"
(`builder_resource_cleanup.rs:48`) — so neither of the two 96 B records this shape allocates
is reclaimed:

1. the **producer's** record, which for a sendable resource bug-643 now carries by pointer but
   which is still never freed at drop for `fs::File`; and
2. the **closed default** record the `$trap_valN` bind materializes, whose assign-time
   reclaim passes `frees_record: false` for the same exclusion.

96 B of the 192 is therefore the documented design (the non-`TRAP` spelling leaks it too) and
96 B is the default record that nothing frees. Splitting the two is the first job: they need
different answers, and only the second is `TRAP`-specific.

## Goal

- The reproduction flat at N and 2N; the non-`TRAP` `fs::openFile` loop flat too.

### Non-goals (must NOT change)

- The `fs::File` buffer reclaim (`FILE_OFFSET_BUF_PTR` / `FILE_OFFSET_READ_PTR`) must keep
  working; the record-sharing exclusion exists because those buffers are reached through the
  record.
- No double free: a re-dropped or moved record must stay skipped.

## Phases

### Phase 1 — failing test + audit

- [ ] Soak test for the `TRAP` and non-`TRAP` `fs::File` shapes; confirm RED on the main
      thread and localize each 96 B half.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

A resource whose record is excluded from drop-time reclaim leaks it, and the inline-`TRAP`
shape leaks a second copy on top.
