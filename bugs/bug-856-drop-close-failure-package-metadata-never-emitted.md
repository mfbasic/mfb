# bug-856: §15 requires compiled cleanup metadata for drop-close failures, but the `.mfp` cleanup table is always empty, so `pkg info`'s secondary-close audit line can never fire

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Dead-code

Status: Open
Coordinator note: repro measured by the review agent only; the coordinator did not re-run it.
Regression Test: none yet

**Owner decision (2026-10-06, supersedes 2026-10-05): remove the cleanup table.** It is a
bytecode-era structure (PC ranges, a resource register). Structured IR already encodes every
resource lifetime through binding types and scopes, `resource_owners` and `RESOURCE_TABLE`, and
nothing at runtime reads the table. So:
- Remove the per-function cleanup table from the package format entirely, including
  `FUNCTION_TABLE`'s `cleanupCount`/`cleanupOffset` fields. They have no use. Bump the package
  format version, rebuild every committed `.mfp`, and decide how an old-format package is
  rejected (a clear version error).
- Remove the writer and reader code, the `pkg info` "Resource Cleanups" section, the
  `package/07_functions.md` cleanup fields and the `package/09` "Relationship to the
  function-table cleanup fields" section. Remove §15's "compiled cleanup metadata" sentence.
- **Keep the secondary-close audit by deriving it**: `pkg info`'s secondary-close line and
  `AUDIT-RESOURCE-SECONDARY-CLOSE` fire for a package function that binds a resource whose
  `RESOURCE_TABLE` entry has the close-may-fail flag. Spec the derived rule in the same change.

Earlier decision (2026-10-05, superseded): follow the language spec (§15). Compiled cleanup metadata is emitted, so a drop-close
failure can be reported by runtime and audit tooling, and package audit output identifies
those cleanup regions. The writer (`src/binary_repr/writer.rs:lower_function`) stops emitting
an empty cleanup table. `package/09_resource-regions.md` is rewritten to describe the emitted
table instead of "always empty / reserved". The currently dead consumers (`pkg info`'s
secondary-close line, `AUDIT-RESOURCE-SECONDARY-CLOSE`) become reachable from real packages.

Language spec §15 says compiled cleanup metadata must keep what tooling needs to report a
drop-close failure, and that package audit output should identify those cleanup
regions. The package spec says the opposite: the per-function cleanup table is always
emitted empty and is reserved. The writer agrees with the package spec
(`src/binary_repr/writer.rs:lower_function` sets `cleanups: Vec::new()`). That makes the
consumers dead code: `src/cli/pkg.rs`'s "audit: records secondary close failure" line and
`src/audit/collect/findings.rs`'s `AUDIT-RESOURCE-SECONDARY-CLOSE` finding are only
reachable from a hand-built test (`src/binary_repr/tests/gap_tests.rs`).

At runtime, drop-close failures do go to the arena cleanup-failure ledger
(`builder_resource_cleanup.rs:record_secondary_cleanup_failure`, read by
`src/debugger/runtime.rs` as `cleanupFailCount`). Source-level `mfb audit` also emits
`AUDIT-RESOURCE-CLOSE-MAY-FAIL`. So the runtime half of the claim holds. Only the
compiled-package half does not.

Class: UNCLEAR (needs an owner decision). The two specs contradict each other. Either
§15 drops or rewords the compiled-metadata/package-audit sentences, or the writer
starts emitting cleanup records that carry the flag.

References:

- `src/docs/spec/language/15_resource-management.md` line 323: "Compiled cleanup metadata must preserve enough information for runtime and audit tooling to report a drop-close failure. Package audit output should identify cleanup regions that retain this failure metadata."
- `src/docs/spec/package/09_resource-regions.md` line 9: "The producer always emits an **empty** cleanup table for structured functions ... treat the cleanup-table fields as reserved/empty"

## Failing Reproduction

```
mkdir -p /tmp/p/src && cd /tmp/p
cat > project.json <<'J'
{ "name": "pkgaudit", "version": "0.1.0", "mfb": "1.0", "kind": "package",
  "description": "probe", "sources": [{ "root": "src", "role": "lib", "include": ["**/*.mfb"] }],
  "targets": ["native"] }
J
cat > src/lib.mfb <<'M'
IMPORT fs
EXPORT FUNC touch(path AS String) AS Integer
  RES f AS fs::File = fs::open(path, "write")
  RETURN 1
END FUNC
M
mfb build && mfb pkg info pkgaudit.mfp | grep -A2 "Resource Cleanups"
```

- Observed: `cleanups: 0` and `Resource Cleanups:` / `  <none>`, even though a function holds a drop-closed resource.
- Expected (per §15): a cleanup entry flagged `audit: records secondary close failure`, or §15 should not promise one.

## Root Cause

`src/binary_repr/writer.rs:lower_function` always writes `cleanups: Vec::new()` (the
structured-IR design in package/09). Nothing sets `CLEANUP_FLAG_RECORD_SECONDARY_CLOSE_FAILURE`.

## Goal

- The two specs agree. If §15 is reworded, either delete the unreachable consumers in
  `pkg.rs` and `findings.rs` or document them as reserved.

## Summary

This is a contract inconsistency, not a runtime fault. Drop-close failures are still
swallowed safely and still recorded in the runtime ledger.
