# bug-282: binary_repr `.mfp` decode LOW cluster (unverified callable exports, duplicate type names, missing trailing-byte checks, vestigial flat-code surface)

Last updated: 2026-07-17
Effort: medium (1h–2h across items)
Severity: LOW
Class: Security / Correctness / Dead-code

Status: Fixed
Regression Test: per-item (src/binary_repr/tests.rs)

A cluster of LOW-severity hardening/consistency gaps in the `.mfp` package decoder
found during goal-06. The decoder is a hardened untrusted-input surface (audit-1
PKG-01..08); these are residuals in sections added after those audits. Grouped per
the repo's low-cluster convention; distinct root causes, one document. Signature
gating (PKG-01) covers the whole file, so none is a tamper channel on a
well-signed package — the value is closing strictness asymmetries and forgery
corners consistently before MVP.

References:

- `planning/old-plans/audit-1-package-decode.md` (PKG-01..08),
  `bugs/completed-bugs/bug-21-*` (ABI-surface verification).
- Found during goal-06 review of `src/binary_repr/{reader,sections,builder,writer}.rs`.

## Items

### B1 — callable ABI_INDEX exports without an EXPORT_TABLE counterpart are never hash-verified
- `src/binary_repr/reader.rs:1147` (`validate_abi_index`; Func/Sub skipped at
  `:1173-1174`).
- The validation worklist is derived from decoded `EXPORT_TABLE` entries; the
  callable loop `continue`s on `Func|Sub`. An ABI_INDEX Func/Sub entry whose name
  matches no EXPORT_TABLE row is accepted with an arbitrary, unrecomputed sigHash,
  which then flows into `package_info().exports` → `pkg check-abi` and the registry
  `abi_index`, and can satisfy an importer's used-symbol pin at resolve while the
  function does not exist (failing later at merge with a confusing error). This is
  the callable-side mirror of the Type/Union/Enum asymmetry bug-21 closed
  ("no ABI surface is trusted unverified").
- Fix: after the callable loop, require every ABI Func/Sub export to correspond to
  some EXPORT_TABLE entry (name+kind), or recompute/verify its hash; reject
  otherwise.
- Prior-work: new (residual of bug-21's scope).

### B2 — duplicate type-table names let ABI validation and `package_type_exports` disagree
- `src/binary_repr/builder.rs:243` (`package_type_exports`, `type_by_name`
  last-wins) vs `reader.rs:1184-1204` (any-candidate hash match), dup accepted at
  `reader.rs:630`.
- `read_type_entries` accepts N entries sharing a name. `validate_abi_index`
  passes if *any* same-name candidate reproduces the hash, but
  `package_type_exports` decodes the *last* entry. A crafted package can carry
  entry A (hash-matching the export) + entry B (different fields): validation
  passes via A while importers compile against B. Bounded — merged IR is
  re-verified (PKG-02) so the end state is a late build error, not type-confused
  codegen; the writer never emits duplicates, so malicious-only.
- Fix: reject duplicate `(name, kind)` pairs in `read_type_entries` (a legitimate
  writer never produces them), which also makes the candidates loop single-candidate.
- Prior-work: new (same class as PKG-06 duplicate-section rejection, for names).

### B3 — doc-table (id 17) and native-library-table (id 10) decoders skip the trailing-bytes check
- `src/binary_repr/reader.rs:111` (`read_doc_table`) and
  `src/binary_repr/sections.rs:782` (`read_native_library_table`).
- Every other section ends with `if offset != bytes.len() { Err("invalid trailing
  bytes …") }`. The two newest sections return without it, so trailing garbage
  inside those sections decodes silently — a strictness asymmetry and smuggling
  nook (not a tamper channel post-PKG-01). audit-1 PKG-05 flagged the identical
  omission on `read_resource_table` (since fixed); reintroduced by these two.
- Fix: add the `offset != bytes.len()` rejection to both decoders.
- Prior-work: new for these two sections.

### B4 — vestigial flat-code wire surface (written-but-discarded fields, decode paths only crafted input reaches)
- `src/binary_repr/reader.rs:880-899`, `966-976`, `:450`;
  `src/binary_repr/writer.rs:1004-1008`, `565-566`; `builder.rs:178-201`.
- Since bodies moved to SECTION_BINARY_REPR, the writer always emits zero
  registers/cleanups and zero-length code and errors on nonzero code length at
  decode, so the register/cleanup decode loops, the `Cleanup`/`Register` structs,
  and `BinaryReprPackageInfoCleanup` are reachable only from crafted packages.
  The manifest's `entry_function`/`entry_flags` and dependency/export counts are
  written but ignored on read (never cross-validated), and `TypeEntry.owner_package`
  is decoded but never consumed — ~80 loc of unreachable-in-practice surface that
  invites drift and lets a manifest lie about its counts. All bounds-checked (no
  hazard).
- Fix: either cross-validate manifest counts against decoded tables and reject
  nonzero register/cleanup counts (tightening), or document the fields as reserved;
  keep the wire layout stable.
- Prior-work: new (bug-100 removed adjacent writer dead maps; read-side vestiges
  unexamined).

## Goal

- B1/B2 close forgery/desync corners with explicit rejections; B3 restores the
  trailing-byte strictness invariant; B4 removes or documents the dead surface.

### Non-goals (must NOT change)

- The `.mfp` wire layout for legitimate packages (B4 keeps byte-stability).
- PKG-01..08 hardening verified intact this pass.

## Blast Radius

Each item is a single decode/validate site (cited). B1 relates to bug-273/275
(registry trust) only thematically; independent code.

## Fix Design / Phases

Land per item with a crafted-package unit test each (all four are directly
constructible in `tests.rs`). No shared refactor.

- [ ] Phase 1: failing tests for B1/B2/B3 (B4 is an assertion/doc change).
- [ ] Phase 2: apply rejections; decide B4 tighten-vs-document.
- [ ] Phase 3: full `cargo test` green; no golden drift for legitimate packages.

## Validation Plan

- Regression tests: crafted-package rejection for B1/B2/B3.
- Full suite: acceptance + unit tests.
- Doc sync: spec/package/03_metadata-encoding.md and 07_functions.md for B3/B4.

## Summary

Four small decoder-strictness residuals; each is a localized rejection or
tightening with a directly-constructible test. No active exploit given signature
gating; value is consistency and defense-in-depth pre-MVP.

## Resolution

All four items landed, each with its own crafted-input regression test.

**B1** — `validate_abi_index`'s callable loop is driven by EXPORT_TABLE, so an
ABI_INDEX `Func`/`Sub` entry naming no EXPORT_TABLE row was simply never reached.
The `continue` arm now requires the counterpart before skipping, so an unbacked
callable is rejected by name instead of carrying an arbitrary sigHash into
`pkg info`, `pkg check-abi` and the registry `abi_index`.
Test: `validate_abi_index_rejects_callable_export_with_no_export_table_row`.

**B2** — `read_type_entries` now rejects a repeated `(name, kind)` pair. The
report's premise (a legitimate writer never emits one) was *not* taken on faith:
a comment in `validate_abi_index` explicitly anticipates several same-name
candidates, so the rejection was landed first and the whole suite -- unit and all
1000 acceptance tests, which build real packages -- run against it. Nothing broke,
confirming the duplicate is crafted-only.
Test: `read_type_entries_rejects_duplicate_name_and_kind`.

**B3** — the trailing-bytes rejection every other section performs was added to
`read_doc_table` and `read_native_library_table`.
Tests: `read_doc_table_rejects_trailing_bytes`,
`a_table_with_trailing_bytes_is_rejected`.

**B4** — took the report's second option (document as reserved) for the
register/cleanup surface, and the first (tighten) for the part that actually
matters. The decision point was a test: `encode_functions_emits_registers_and_cleanups`
deliberately round-trips registers and cleanups through encode → `pkg info`, so
rejecting a nonzero count would have meant deleting a live assertion and changing
`BinaryReprPackageInfoCleanup`, a public shape -- disproportionate for a
bounds-checked, no-hazard surface. What was closed instead is the real defect the
report names: the manifest's `dependency_count` and `export_count` were decoded
into `_`-prefixed locals and dropped, letting a manifest lie about its own tables.
Both now live on `BinaryReprManifest` and are cross-checked by a new
`validate_manifest_counts`, extracted as a named sibling of `validate_abi_index`
rather than left inline -- which is also what makes it directly testable.
`native_link_count` and `entry_function`/`entry_flags` are genuinely unvalidatable
(the writer emits a literal `0` for the first even on binding packages, and a
package has no entry point), so those are commented as reserved at the decode site.
Test: `validate_manifest_counts_rejects_a_manifest_that_lies_about_its_tables`.

The `.mfp` wire layout is unchanged throughout; every rejection is new strictness
on input a legitimate writer never produces.
