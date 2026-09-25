# bug-694: no `fs` man page has an Errors section — every `fs` descriptor declares `errors: vec![]`

Last updated: 2026-09-24
Effort: medium (1h–2h)
Severity: LOW
Class: Other (documentation correctness)

Status: Open
Regression Test: a registry test asserting every `fs` member that raises declares the codes it raises (Phase 1)

Every `fs` member descriptor in `src/codegen/builtins/fs/func_*.rs` declares
`errors: vec![]`, so `mfb man fs <member>` renders **no Errors section on any
`fs` page**, although the members raise `ErrPathNotFound`, `ErrNotFound`,
`ErrAccessDenied`, `ErrAlreadyExists`, `ErrWriteFailed`, `ErrReadFailed`,
`ErrInvalidArgument`, `ErrInvalidPath`, `ErrEncoding`, `ErrOutOfMemory`, and so
on. A developer reading `mfb man fs readText` cannot tell what to `TRAP` for.

**Correct behavior.** Each `fs` page's Errors table lists exactly the codes that
member can raise. The table is derived from the descriptor's `errors`, so this
means filling in each descriptor's `errors`.

References:

- `.ai/man-content.md` §2: "Every raisable error and the condition that raises
  it, consistent with the auto-derived Errors table."
- bug-454 (`bugs/completed/bug-454-win64-os-resourcepath-unsupported.md`): the
  same gap for `os::resourcePath`, and why no test caught it.
- Found during plan-156-C (`planning/plan-156-C-os-user-paths.md`, Correction
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
- `os`: unaffected. Its members declare `errors` (bug-454, plan-156).

## Fix Design

1. Per member, derive the raise set by reading its lowering: direct
   `raise_error_into` sites, plus the codes of any shared mapper it calls
   (`emit_errno_error_mapping` → `ErrNotFound`/`ErrAccessDenied`/
   `ErrAlreadyExists`/`ErrWriteFailed`; `emit_fs_path_errno_error_mapping` →
   `ErrPathNotFound`/`ErrAccessDenied`/`ErrAlreadyExists`/`ErrNotEmpty`?/
   `ErrInvalidPath`/`ErrWriteFailed`; verify each against the code), plus
   `ErrOutOfMemory` wherever the helper allocates.
2. Fill `errors` and bring each page's prose into line (also remove any stale
   `ErrOutput` naming, the old name of `ErrWriteFailed`; plan-156-C fixed
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

- [ ] Add the registry test (Fix Design 3) and confirm it fails listing the 42
      `fs` members.
- [ ] Measure the other packages (Blast Radius) and record a verdict per package.

Acceptance: the test fails for exactly the documented reason; the audit list is
complete.
Commit: —

### Phase 2: the fix

- [ ] Fill each `fs` descriptor's `errors` from its raise sites (Fix Design 1).
- [ ] Align each page's prose with its table.

Acceptance: the Phase 1 test passes, and
`scripts/man-census.sh --fill fs` / `mfb man fs <member>` show the tables.
Commit: —

### Phase 3: validation

- [ ] Run `cargo test --bin mfb registry` and
      `scripts/man-run-examples.sh fs --run`.
- [ ] Byte-identity: the `fs` `.ncodesum` goldens are expected **unchanged**. A
      registry description can churn goldens (`.ai/testing-gates.md`, "What
      churns the goldens #3"), so run `scripts/artifact-gate.sh` and root-cause
      any diff.

Acceptance: green, with the only deltas being the man output.
Commit: —

## Validation Plan

- Regression test: the Phase 1 registry test.
- Runtime proof: none needed. The Errors table is derived output; render it.
- Doc sync: the `fs` man pages (descriptor prose). No spec change is expected.
- Full suite: the scoped commands in Phase 3.

## Open Decisions

- Whether the repository-wide `raise_error_into` declaration scan lands here or
  as its own bug. Recommended: decide from Phase 1's measurement.

## Summary

This is a documentation-data gap across one package with no runtime effect. The
risk is getting a member's raise set wrong, which is why each list is derived
from the code and guarded by a test.
