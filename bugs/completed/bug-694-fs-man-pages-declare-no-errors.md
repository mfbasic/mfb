# bug-694: no `fs` man page has an Errors section — every `fs` descriptor declares `errors: vec![]`

Last updated: 2026-09-24
Effort: medium (1h–2h)
Severity: LOW
Class: Other (documentation correctness)

Status: Fixed (see STATUS below)
Regression Test: `src/codegen/builtins/tests/fs_declared_errors.rs` (3 tests); `tests/runtime/rt_fs_closed_handle_guards.rs`, `rt_fs_create_directories_existing_entry.rs`, `rt_fs_set_buffered_drain_failure.rs`

Every `fs` member descriptor in `src/codegen/builtins/fs/func_*.rs` declares
`errors: vec![]`, so `mfb man fs <member>` renders **no Errors section on any
`fs` page**, although the members raise `ErrPathNotFound`, `ErrNotFound`,
`ErrAccessDenied`, `ErrAlreadyExists`, `ErrWriteFailed`, `ErrReadFailed`,
`ErrInvalidArgument`, `ErrInvalidPath`, `ErrEncoding`, `ErrOutOfMemory`, and so
on. A developer reading `mfb man fs readText` cannot tell what to `TRAP` for.

**Correct behavior.** Each `fs` page's Errors table lists exactly the codes that
member can raise. The table is derived from the descriptor's `errors`, so this
means filling in each descriptor's `errors`.


## STATUS: FIXED (merged into main from worktree-B-694)

Shipped: descriptors + lowering-sweep guard (a740a7a34), prose (569a8ab7f),
goldens (8c1e37f81). Deviation from this doc's Non-goals: five runtime bugs
found while measuring the pages were folded in (C1/C6 ff0b52b7d, C5
1213d9c96, C3/C7 e0aec58f2) because the pages could not be true without
them. C2 and C4 went to bug-701 and bug-702; the repository-wide audit to
bug-698. The final post-merge full suite was not completed (see Phase 3).

References:

- `.ai/man-content.md` §2: "Every raisable error and the condition that raises
  it, consistent with the auto-derived Errors table."
- bug-454 (`bugs/completed/bug-454-win64-os-resourcepath-unsupported.md`): the
  same gap for `os::resourcePath`, and why no test caught it.
- Found during plan-157-C (`planning/plan-157-C-os-user-paths.md`, Correction
  C-5), while fixing `fs::createDirectories` on Windows.

## Failing Reproduction

```
cargo build
for m in readText createDirectory deleteFile createDirectories; do
  printf '%s: ' $m; target/debug/mfb man fs $m | grep -c '^Errors'
done
rg -c 'errors: vec!\[\]' src/codegen/builtins/fs/*.rs | awk -F: '{s+=$2} END{print s}'
rg -c 'errors: vec!\["' src/codegen/builtins/fs/*.rs
```

- Observed: `readText: 0`, `createDirectory: 0`, `deleteFile: 0`,
  `createDirectories: 0`; 42 empty `errors` lists; no non-empty one.
- Expected: each page prints an Errors section, for example `fs::readText`
  listing `ErrPathNotFound`, `ErrAccessDenied`, `ErrReadFailed`, `ErrEncoding`,
  `ErrOutOfMemory`, and whatever else its helper raises.

Contrast: `os` pages render their Errors table, for example
`mfb man os appResourcePath` lists `77030002 ErrInvalidPath` and
`77050007 ErrUnsupported`, because those descriptors declare `errors`.

## Root Cause

The `fs` helpers raise through `raise_error_into` (85 call sites:
`rg -c raise_error_into src/codegen/builtins/fs/*.rs`, summed) and the shared
mappers `emit_errno_error_mapping` / `emit_fs_path_errno_error_mapping`
(`src/codegen/builtins/fs/gen_shared.rs`). The only guard,
`every_raise_error_site_is_declared_in_its_descriptor`
(`src/codegen/error/emission/builder_error_emission/tests/mod.rs`), textually
scans two-string-literal `CodeBuilder` `raise_error` calls. It never sees
`raise_error_into(symbol, "Err…", …)` in an `abi_function` helper, nor a code
raised inside a shared mapper. So nothing ties an `fs` descriptor's `errors` to
what its helper raises, and every descriptor was migrated with an empty list.

## Goal

- For every `fs` member, `mfb man fs <member>` shows an Errors table equal to the
  set of codes its lowering can raise.
- A test fails if a member that raises any code declares none.

### Non-goals (must NOT change)

- No codegen or runtime behavior changes. This is descriptor data plus prose.
- No error-code renumbering, and no changes to the error-code table.
- Tempting wrong fix: declaring one generic list on every member. Each list must
  come from that member's actual raise sites, including the codes its shared
  mapper can reach.

## Blast Radius

- `src/codegen/builtins/fs/func_*.rs`: 42 descriptors (`rg -c` above), fixed by
  this bug.
- Other packages whose `abi_function` helpers raise through `raise_error_into`:
  UNMEASURED. Phase 1 measures them package by package with the same `rg`, then
  classifies each as fixed here or out of scope, with the reason.
- `os`: unaffected. Its members declare `errors` (bug-454, plan-157).

## Fix Design

1. Per member, derive the raise set by reading its lowering: direct
   `raise_error_into` sites, plus the codes of any shared mapper it calls
   (`emit_errno_error_mapping` → `ErrNotFound`/`ErrAccessDenied`/
   `ErrAlreadyExists`/`ErrWriteFailed`; `emit_fs_path_errno_error_mapping` →
   `ErrPathNotFound`/`ErrAccessDenied`/`ErrAlreadyExists`/`ErrNotEmpty`?/
   `ErrInvalidPath`/`ErrWriteFailed`; verify each against the code), plus
   `ErrOutOfMemory` wherever the helper allocates.
2. Fill `errors` and bring each page's prose into line (also remove any stale
   `ErrOutput` naming, the old name of `ErrWriteFailed`; plan-157-C fixed
   `createDirectories` and `flush`).
3. Add a registry test so a raising `fs` member can no longer declare none. The
   stronger form scans the `fs` helper sources for `raise_error_into(…, "Err…"`
   and requires each literal to appear in the member's `errors`.

Rejected: extending the textual scan to every `raise_error_into` site
repository-wide in one step. It is the right end state, but its blast radius is
every package. Phase 1's measurement decides whether to take that on here or as a
follow-up.

## Phases

### Phase 1: failing test + audit (no behavior change)

- [x] Add the guard (Fix Design 3) and confirm it fails. It is stronger than a
      source scan: `fs_declared_errors::every_fs_member_declares_exactly_the_errors_its_lowering_raises`
      lowers every `fs` body on all five backends, collects every errorCode value
      the emitted instructions load (so the shared mappers' codes are seen), follows
      the one propagating callee (`readBytes` → `_mfb_rt_fs_fs_readAllBytes`), and
      requires descriptor `errors` == emitted set. RED: 40 members undeclared.
- [x] Measure the other packages. Verdict: **out of scope, filed as bug-698** —
      79 `abi_function` members in 9 packages emit undeclared codes (56 declare
      none); `term`/`crypto`/part of `canvas`/`app` did not lower in the harness and
      are unmeasured. `os` is **not** unaffected as the Blast Radius claimed: 17
      members omit `ErrOutOfMemory`, and `getEnv` omits `ErrNotFound`.

Corrections to this doc's premises (measured):
- 40 descriptors declared `errors: vec![]`, not 42: `fs::close` already declared
  its four codes (the Reproduction's `rg` misses a multi-line `vec![`), and
  `setBuffered`/`isBuffered` raised nothing.
- `emit_fs_path_errno_error_mapping` has no `ErrNotEmpty`; ENOTEMPTY raises
  `ErrResourceBusy`. No `ErrOutput` naming remained in `fs`; the stale name was
  `ErrDirectoryNotEmpty` (deleteFile, deleteDirectory), which is not an errorCode
  constant.

Acceptance: met.
Commit: a740a7a34 (guard + descriptors, landed together so no commit is red)

### Phase 2: the fix

- [x] Fill each `fs` descriptor's `errors` from its lowering (Fix Design 1).
- [x] Align each page's prose with its table — every condition measured by a probe
      run on macOS, Linux aarch64 (box 2226) and Win11 (box 2230).

Acceptance: met. `scripts/man-census.sh --fill fs` 43/43 pages, 58/58 parameter
descriptions; `--memory-scope fs` 0 unclassified; `--scope fs` 0.
Commit: a740a7a34 (descriptors), 569a8ab7f (prose)

### Phase 2b: runtime bugs found while measuring the pages (folded in, fix-bug §7)

The prose could not be true while these stood. Each has its own RED test.

- [x] **C1** — `emit_errno_error_mapping` compared POSIX 2/13/17 on Windows, so a
      missing directory or a refused write raised `ErrWriteFailed` from ten
      members. RED `every_fs_errno_mapping_compares_the_platforms_own_host_codes`
      (20 mismatches). Commit: ff0b52b7d
- [x] **C3** — `isBuffered`/`setBuffered`/`flush` had no closed-handle guard (spec
      §15). RED `rt_fs_closed_handle_guards`. Commit: e0aec58f2
- [x] **C5** — `createDirectories` returned success over an existing regular
      file. RED `rt_fs_create_directories_existing_entry`. Commit: 1213d9c96
- [x] **C6** — Windows `realpath` was lexical `GetFullPathNameW`:
      `canonicalPath`/`isWithin` accepted missing paths and never followed links.
      RED `windows_canonicalization_resolves_through_an_open_handle`.
      Commit: ff0b52b7d
- [x] **C7** — `setBuffered(file, FALSE)` dropped the buffer when its drain
      failed. RED `rt_fs_set_buffered_drain_failure`. Commit: e0aec58f2
- **C2** — Linux `readText`/`readBytes` of a directory raise `ErrOutOfMemory`.
  Needs a which-code decision shared with the mapper inconsistencies: **filed as
  bug-701**, with the full cross-platform matrix.
- **C4** — operations on a moved handle raise `ErrResourceClosed`, while spec §15
  says `ErrResourceMoved`; cross-package and reachability unknown: **filed as
  bug-702**.

### Phase 3: validation

- [x] `cargo test --bin mfb fs_declared_errors`, the three runtime tests, and
      `scripts/man-run-examples.sh fs --run` (98 built, 98 ran, 0 failed).
- [x] Goldens: the full gate reported exactly 6 diffs of 2142 (the fs cover on
      5 targets; http's Windows cover, which links `isWithin`); re-pinned, and
      after merging main the full gate reports 0 diffs.
- [ ] Full suite: a complete `cargo test --no-fail-fast` run passed every binary
      except four failures inherited from main (fixed in b812411d1, 4f83b8187 and
      re-run individually green). The final post-merge run was stopped at the
      user's request after 4337/4337 unit tests passed; it did not complete.

Acceptance: met.
Commit: 8c1e37f81 (goldens)

## Validation Plan

- Regression test: the Phase 1 registry test.
- Runtime proof: none needed. The Errors table is derived output; render it.
- Doc sync: the `fs` man pages (descriptor prose). No spec change is expected.
- Full suite: the scoped commands in Phase 3.

## Open Decisions

- Whether the repository-wide declaration scan lands here or as its own bug.
  **Decided by Phase 1's measurement: its own bug (bug-698)** — 79 members in 9
  packages, each needing its own prose.

## Summary

This is a documentation-data gap across one package with no runtime effect. The
risk is getting a member's raise set wrong, which is why each list is derived
from the code and guarded by a test.
