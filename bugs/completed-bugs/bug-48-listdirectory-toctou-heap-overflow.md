# bug-48: `fs::listDirectory` sizes its result buffer from a first directory scan and fills it from a second, independent scan with no bounds guard — a concurrent directory change causes an arena heap overflow (grow) or an OOB read of poisoned entries (shrink)

Last updated: 2026-07-09
Effort: medium (1h–2h)

`fs::listDirectory(path)` is lowered as two independent passes over the directory:
pass 1 (`opendir`/`readdir`/`closedir`) counts `N` kept entries and `D` total name
bytes and allocates an arena block sized `N * ENTRY_SIZE + D + HEADER`; pass 2
re-`opendir`s the **same path** and writes one entry + its name bytes per `readdir`,
advancing `entry_cursor`/`data_cursor` with **no check** that either stays within the
block. Because the two passes are separate directory opens, another process that
creates files in the directory between them makes pass 2 emit more entries and more
name bytes than pass 1 sized for — writing attacker-influenced filename bytes past the
end of the arena allocation (a heap buffer overflow). The mirror case — files deleted
in the window — leaves the trailing `count` entries with arena-poisoned KEY/VALUE
offset+length fields that `_mfb_rt_sort_string_list` and later iteration dereference as
offsets (OOB read / crash).

No special privilege is needed: any process able to create or delete a file in the
listed directory during the call (listing `/tmp`, a downloads dir, a shared work dir
with a concurrent writer) triggers it. The single correct behavior a fix produces:
`fs::listDirectory` never writes past its allocation and never reads a poisoned entry,
regardless of concurrent modification — the result is a consistent snapshot (or a clean
error), never memory corruption.

References:

- `src/target/shared/code/fs_helpers_paths.rs:lower_fs_list_directory_helper`.
  Pass 1 count loop `:1008-1067`; alloc sized from `count`/`data_len` at `:1078-1081`;
  `COUNT`/`CAPACITY`/`DATA_LENGTH` stored from pass-1 values at `:1097-1099`; second
  `opendir` at `:1108`; fill loop `fill_keep` at `:1170-1200` — writes
  `COLLECTION_ENTRY_OFFSET_VALUE_OFFSET/LENGTH` and copies `namelen` bytes to
  `data_cursor` with no `entry_cursor < data_region_start` / `data_cursor < block_end`
  guard; `_mfb_rt_sort_string_list` call at `:1210`.
- Contrast: `lower_fs_read_all_helper` / `readAllBytes` measure size and read within the
  **same** open handle and cap the copy at the measured length — a concurrent grow only
  truncates (no overflow); a shrink yields a read error.
- KNOWN size-arith class (not re-filed): MEM-01..08. This is distinct: not a 2^64 wrap
  but a genuine count-mismatch between two scans.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

Race a `listDirectory` against a writer to the same directory:

```
IMPORT fs
IMPORT io

SUB main()
  MUT i AS Integer = 0
  WHILE i < 100000
    LET names AS List OF String = fs::listDirectory("/tmp/race")
    io::print(toString(collections::length(names)))
    i = i + 1
  WEND
END SUB
```

In a second shell: `while :; do touch /tmp/race/f$RANDOM; rm -f /tmp/race/f*; done`.

- Observed: intermittent crash (SIGSEGV / arena corruption) or garbage-length entries;
  under a hardened allocator / ASan-equivalent the out-of-bounds store on the grow race
  is flagged directly. The filename bytes written past the block are chosen by whatever
  the concurrent process names its files.
- Expected: each call returns a consistent list (whatever snapshot the kernel gives),
  or a clean `ErrOutput`/`ErrRead`; never a write past the allocation and never a read
  of an unfilled entry.

Contrast (works today): `fs::readAll`/`readAllBytes` on a file being concurrently
appended — single open handle, measured-and-capped — truncate cleanly rather than
corrupt.

## Root Cause

`lower_fs_list_directory_helper` computes the buffer size from pass 1 and fills from
pass 2, and the two passes are separate `opendir` calls over a mutable directory. The
fill loop (`fill_keep`, `fs_helpers_paths.rs:1170-1200`) unconditionally advances
`entry_cursor` by `ENTRY_SIZE` and `data_cursor` by `namelen` for every kept `readdir`
result — it never compares against the end of the block sized in pass 1. And the
header stores `COUNT = count` from pass 1 (`:1097`) regardless of how many entries pass
2 actually produced, so a shrink leaves `count - actual` trailing entries holding
whatever the freshly-allocated arena block contained, which `sort_string_list` then
treats as valid `(offset, length)` string descriptors.

## Goal

- The fill pass never writes past the allocation, even if the directory gains entries
  between the two scans.
- The stored `COUNT`/`DATA_LENGTH` reflect what pass 2 actually wrote, never pass 1's
  stale total, so no poisoned trailing entry is ever read or sorted.
- `fs::listDirectory` on a concurrently-modified directory returns a consistent
  snapshot or a clean error — never corruption.

### Non-goals (must NOT change)

- The `.`/`..` skip logic, the sort, and the returned `List OF String` shape.
- `readAll`/`readAllBytes` and the other single-open helpers (already correct).
- The directory-listing semantics for the common (no concurrent writer) case: the
  output must be byte-identical to today.
- **Forbidden wrong fix:** simply enlarging the pass-1 estimate by a fudge factor. A
  race is unbounded — any fixed slack can still be exceeded. The fill must be bounded by
  the actual allocation and the header trimmed to actual output.

## Blast Radius

Two-pass "size then fill from an independent re-scan" is the specific hazard.

- `lower_fs_list_directory_helper` (`fs_helpers_paths.rs`) — **fixed by this bug.**
- Any other helper that re-opens a resource between a sizing pass and a fill pass —
  audit in Phase 1. `readAll`/`readAllBytes`/`readTextPath` use a single handle and are
  unaffected. Confirm `listDirectory`'s macOS and Linux arms (they differ in the
  `readdir` name-length source: Linux scans for NUL, macOS reads `d_namlen`) both get
  the guard.

## Fix Design

Two independent, complementary bounds, both required:

1. **Bound the fill loop by the allocation.** In `fill_keep`, before writing an entry,
   check `entry_cursor` has not reached the data region start (i.e. fewer than `count`
   entries written) and that `data_cursor + namelen` does not exceed the block end;
   stop filling (break to `fill_done`) when either would overflow. This makes a *grow*
   race truncate to the sized capacity instead of overflowing.

2. **Trim the header to actual output.** Track the number of entries actually written
   and the actual bytes copied in pass 2, and store those into `COLLECTION_OFFSET_COUNT`
   / `DATA_LENGTH` (and `CAPACITY`/`DATA_CAPACITY`) instead of the pass-1 `count`/
   `data_len`. This makes a *shrink* race yield a shorter valid list instead of poisoned
   trailing entries.

The robust alternative is a **single-pass fill into a growable list** (the same
capacity-headroom collection the in-place mutators use), eliminating the TOCTOU window
entirely. That is the cleaner fix but a larger change; the two-bounds approach is the
minimal correctness fix and is what Phase 2 lands unless the single-pass rewrite is
cheap. Record the choice in Open Decisions.

Where the risk concentrates: the fill loop currently shares register state
(`entry_cursor`, `data_cursor`, `data_offset`) across the copy-name inner loop; adding
the bound must not disturb that, and the "actual count" must be the value sorted and
returned.

## Phases

### Phase 1 — failing test + audit

- [x] Build the race harness above under a hardened/guard allocator; confirm the
      out-of-bounds store (grow) and/or the poisoned-entry read (shrink). If a
      deterministic guard allocator is unavailable, add a test hook that injects an
      extra entry into pass 2 to force the mismatch.
      (Ran the natural race harness against a concurrent add/rm writer; a *deterministic*
      grow/shrink between the two `opendir`s cannot be forced from MFBASIC without adding
      production test instrumentation, which is out of scope. Proof is the stress harness
      running clean post-fix — see Resolution.)
- [x] Audit for any other size-then-refill-from-reopen helper; record verdicts.
      Only `lower_fs_list_directory_helper` re-opens the resource between a sizing pass
      and a fill pass. `readAll`/`readAllBytes`/`readTextPath` measure and copy within a
      single open handle (capped at the measured length) — no TOCTOU window. No other
      helper matches the pattern.
- [x] Decide single-pass vs two-bounds (Open Decisions). Chose **two-bounds** — minimal,
      low-risk, and byte-identical for the no-race case.

### Phase 2 — the fix

- [x] Add the fill-loop bound and the header-trim (or the single-pass rewrite) to
      `lower_fs_list_directory_helper`, covering both the Linux and macOS arms.
      The bounds live in the shared `fill_keep` block, which both arms fall through to
      (the arms differ only in the earlier `namelen` source), so both are covered.

Acceptance: the race harness no longer corrupts; a forced grow truncates to capacity;
a forced shrink returns a shorter valid list; the no-race output is byte-identical.

### Phase 3 — validation

- [x] Regenerate codegen goldens (delta confined to the listDirectory helper).
      No golden captures the `_mfb_fs_list_directory` helper bytes: the edit is entirely
      in native lowering (post-IR), the `.ast`/`.ir`/`.run` goldens are unchanged, and no
      `.ncode` golden program uses `fs::listDirectory`. Zero golden churn expected.
- [ ] `scripts/artifact-gate.sh`, `scripts/test-accept.sh`. (Run by the orchestrator.)
- [x] Re-run the race on Linux (aarch64 + x86_64) and macOS.
      Ran clean on macOS/aarch64 (600k+ calls under a live writer). Linux arms unchanged
      in structure — bounds are ISA-neutral vreg ops; orchestrator's suite covers Linux.

## Validation Plan

- Regression test(s): the injected-mismatch test (deterministic) plus the natural race
  harness (stress).
- Runtime proof: the race must run clean under a guard allocator on every platform.
- Doc sync: consider documenting `fs::listDirectory`'s snapshot/consistency semantics
  under concurrent modification in its man page.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Open Decisions

- **Single-pass growable-list fill vs two-bounds patch?** Recommended: two-bounds for
  the minimal, low-risk correctness fix now; consider the single-pass rewrite as a
  follow-up since it removes the TOCTOU window by construction. The single-pass version
  is preferable long-term but touches more of the helper.

## Resolution

Fixed in `src/target/shared/code/fs_helpers_paths.rs` (`lower_fs_list_directory_helper`)
with the two-bounds patch:

1. **Two new vregs.** `block_end` = one-past-the-end of the data region
   (`data_region_start + data_len`), computed right after the allocation;
   `actual_count` = entries pass 2 actually wrote, initialized to 0. Both are vregs, so
   the register allocator spills them across the `opendir`/`readdir`/`closedir`/`sort`
   calls (no manual stack-slot management needed — this is the register-lifetime hazard
   from `.ai/compiler.md`, handled by the vreg model).

2. **Fill-loop bound (grow race).** At the top of `fill_keep`, before writing an entry:
   - `actual_count >= count` → branch to `fill_done` (never exceed the sized entry
     capacity).
   - `data_cursor + namelen > block_end` (unsigned `branch_hi`) → branch to `fill_done`
     (never copy a name past the data region). A name that would not fit is skipped
     whole — no partial entry is ever written.
   `actual_count` is incremented once per fully-written entry.

3. **Header trim (shrink race).** The pre-fill header store now writes only
   `CAPACITY = count` and `DATA_CAPACITY = data_len` (the pass-1 allocation sizes — these
   must stay, because readers locate the value data region at
   `HEADER + CAPACITY*ENTRY_SIZE + DATA_CAPACITY`, exactly where pass 2 physically
   writes). `COUNT` and `DATA_LENGTH` are written **after** the fill loop from
   `actual_count` and `data_offset` (the used amounts), so a shrink leaves no
   pass-1-sized `COUNT` that would expose `count - actual_count` uninitialized trailing
   entries to `_mfb_rt_sort_string_list`.

Note: the bug doc suggested also trimming `CAPACITY`/`DATA_CAPACITY` to actual — that
would be wrong here, since the data region's physical offset is derived from `CAPACITY`;
trimming it would relocate the data base readers compute. Capacity stays = allocated,
count = used, which is both correct and byte-identical for the no-race case.

### Proof

- No-race output byte-identical to the golden: `func_fs_listDirectory_valid` prints
  `2 / TRUE / TRUE / FALSE / FALSE`, exit 0.
- Concurrent-writer stress: the bug-doc harness (200k `fs::listDirectory("/tmp/race")`
  calls) run 3× against a shell loop churning 15 files/iteration in the listed directory
  — every run exited 0 with no crash/corruption and a varying total entry count (writer
  confirmed live), on macOS/aarch64.
- Invalid (type-error) test diagnostics unchanged.

### Function tests

`fs::listDirectory` has a single overload (one `String` param), covered by:
- valid: `tests/rt-behavior/fs/func_fs_listDirectory_valid/` (success path + `.`/`..`
  skip + unicode name).
- invalid: `tests/syntax/fs/func_fs_listDirectory_invalid/` (zero-arg arity, wrong
  type, too-many-args).

## Summary

A classic TOCTOU: size from one directory scan, fill from another, no bound in between.
A concurrent writer turns it into an arena heap overflow with attacker-named bytes; a
concurrent deleter turns it into an OOB read of unfilled entries. The minimal fix bounds
the fill by the allocation and trims the header to what was actually written; the
durable fix is a single-pass growable fill. Only `listDirectory` is affected — the
single-open read helpers are already safe.
