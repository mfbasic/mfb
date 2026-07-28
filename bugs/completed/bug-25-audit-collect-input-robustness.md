# bug-25: `mfb audit` collect-layer LOW input-robustness cluster — lossy `lockfileVersion` cast + unbounded `.mfp` read for hashing

Last updated: 2026-07-08
Effort: small (<1h)

Two independent LOW-severity robustness gaps in the `mfb audit` collection layer,
both in how it consumes untrusted on-disk inputs. Batched (same module scope,
same severity, both trivial).

**(1) `lockfileVersion` read as f64 then lossily cast to i64.**
`src/audit/collect/lockfile.rs:25-28` reads `lockfileVersion` via
`.and_then(|value| value.get::<f64>()).map(|value| *value as i64)`. Any JSON
number is accepted and narrowed: `"lockfileVersion": 1e309` parses to f64 infinity
and `as i64` saturates to `i64::MAX`, reported verbatim; `1.9` silently truncates
to `1`. No integer/range validation. A hand-edited/crafted `mfb.lock` thus
misreports its version (no crash — Rust float→int is saturating).

**(2) Whole `.mfp` file read into memory unbounded for content hashing.**
`src/audit/collect/dependencies.rs:32-37` (`collect_dependencies`) and `:189-193`
(`collect_packages`) call `std::fs::read(&package_file)` — with no size cap — on a
path derived from the untrusted manifest package name, to compute
`package_content_hash`. An oversized `packages/x.mfp` (multi-GB) allocates the
whole file, a memory-exhaustion DoS during audit. Same class as the known PKG-*
bounded-alloc hardening, but these are separate call sites on the audit path.

The single correct behavior a fix produces: `lockfileVersion` is validated as a
finite non-negative integer (or a malformed-lockfile finding is surfaced), and
`.mfp` content hashing is bounded (streamed in chunks or size-capped) so a large
file cannot exhaust memory.

Severity LOW for both: (1) is a misreport with no crash; (2) is a DoS requiring a
crafted large file already present under `packages/`.

References:

- `src/audit/collect/lockfile.rs:25-28` (f64→i64 lossy cast). Contrast:
  `projectHash` is read as String and compared exactly (robust).
- `src/audit/collect/dependencies.rs:32-37`, `:189-193` (`std::fs::read`, no cap).
- Related: audit-1 PKG-05 (unbounded alloc from untrusted counts) — same class,
  different sites.
- Found during goal-01 review of `src/audit/**`.

## Failing Reproduction

(1) A `mfb.lock` with `"lockfileVersion": 1e309`; `mfb audit --format json`:
- Observed: `lockfileVersion` reported as `9223372036854775807`.
- Expected: rejected as malformed, or reported as-is only if a finite non-negative
  integer.

(2) A `packages/x.mfp` of several GB referenced by the manifest; `mfb audit`:
- Observed: audit allocates the whole file (RSS spike) to hash it.
- Expected: bounded/streamed hashing; large files do not exhaust memory.

Contrast: `projectHash` string handling is exact; the header reads go through a
bounded `read_mfp_header`.

## Root Cause

(1) `lockfile.rs` accepts any JSON number and narrows without validation.
(2) `dependencies.rs` uses `std::fs::read` (reads the entire file) for a content
hash instead of streaming.

## Goal

- `lockfileVersion` is validated (finite, non-negative integer) before storage.
- `.mfp` content hashing reads in bounded chunks or enforces a max size.

### Non-goals (must NOT change)

- `projectHash` handling (already robust).
- The reported version for well-formed lockfiles.

## Blast Radius

- `lockfile.rs:25-28`; `dependencies.rs:32-37` and `:189-193` (two hash sites).

## Fix Design

(1) Parse `lockfileVersion` as an integer with range/finiteness validation (reject
or emit a malformed-lockfile finding on failure).
(2) Replace `std::fs::read` at the two hash sites with a chunked streaming hash
(read a bounded buffer in a loop into the digest), or check file length against a
cap first.

## Phases

### Phase 1 — failing test + audit

- [ ] Lockfile test with a non-integer/huge version asserts rejection/validation.
- [ ] (Optional) a size-cap test for `.mfp` hashing.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Validate `lockfileVersion`; stream/cap the `.mfp` hash reads.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; well-formed lockfile/package audit goldens
      unchanged.

## Validation Plan

- Regression test(s): the malformed-lockfile test; streamed-hash equivalence to the
  previous whole-file hash for normal files.
- Full suite: `scripts/test-accept.sh`.

## Summary

Two small untrusted-input robustness gaps in the audit collector: a lossy numeric
cast and an unbounded file read. Both fixes are local and keep well-formed-input
behavior identical.
