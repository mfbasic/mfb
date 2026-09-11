# bug-578: repository artifact parsing permits multi-hundred-megabyte allocations

Last updated: 2026-09-11
Effort: medium (1h–2h)
Severity: HIGH
Class: Security / denial of service

Status: Open
Regression Test: `repository/src/abi.rs` unit tests to be added for section,
string-pool, and ABI-export ceilings; `repository/src/server.rs` request-level
test to prove rejection happens before expensive validation.

An authenticated publisher can submit a body within the 64 MiB request limit
whose MFPC string pool declares millions of empty strings.  The repository
allocates one `String` per entry and can allocate hundreds of MiB from an
approximately 48 MiB decoded artifact.  A self-registered account can therefore
terminate the 512 MiB deployed server through `/validate` or `/publish`.

Correct behavior: malformed or excessively complex MFPC payloads are rejected
with a bounded diagnostic before allocating or iterating beyond documented
limits; ordinary compiler-produced packages remain accepted.

References:

- Security review, 2026-09-11 (repository-only scope)
- `repository/src/server.rs:MAX_BODY_BYTES`

## Failing Reproduction

Add a unit fixture containing an MFPC string-pool section with a count above
the new ceiling and zero-length entries, then run:

```
cargo test -p mfb_repository abi::tests::string_pool_rejects_excessive_entry_count
```

- Observed today: `read_string_pool` accepts the count, reserves space for each
  entry, and constructs every `String`.
- Expected: the parser returns a deterministic limit error before the vector is
  allocated or entries are decoded.

A 48 MiB decoded pool can contain about 12 million four-byte empty entries;
on a 64-bit target, its `Vec<String>` reservation alone is roughly 288 MiB.

## Root Cause

`repository/src/abi.rs:read_string_pool` bounds `Vec::with_capacity` by the
section's byte length but has no absolute entry or aggregate-string ceiling;
the subsequent loop still creates every declared entry.  `read_section_table`
also permits a table sized only by the input body, and `read_abi_exports` has no
export-count ceiling.  `repository/src/server.rs:validate_package_request`
calls the parser through vendor, ABI-index, manifest, and description readers,
amplifying the work on the same attacker-controlled payload.

## Goal

- Enforce explicit, documented ceilings for MFPC sections, string-pool entries
  and bytes, and ABI exports before allocation or unbounded iteration.
- Parse shared section/string-pool state once per request where practical.

### Non-goals (must NOT change)

- Do not raise `MAX_BODY_BYTES` or silently truncate package metadata.
- Do not weaken signature, payload-hash, or vendor-blob validation.
- Do not reject valid packages merely because they have an absent optional ABI
  index or metadata section.

## Blast Radius

- `repository/src/abi.rs:parse_vendor_blobs` — fixed by shared parser limits.
- `repository/src/abi.rs:parse_abi_index` / `abi_index_json` — fixed by shared
  parser limits and export cap.
- `repository/src/abi.rs:parse_manifest_metadata` and
  `parse_package_description` — fixed by shared parser limits.
- `repository/src/backfill.rs:run` — latent consumer of the same parsers;
  fixed by shared parser limits, but its operator-facing error reporting must
  remain intact.

## Fix Design

Define conservative format-level constants based on the compiler's actual
maximums, then reject count and byte budgets before `Vec` growth.  Prefer a
parsed MFPC view shared by the repository consumers over repeated independent
parses.  A request-body limit alone is insufficient because representation
overhead exceeds input bytes.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add red unit tests for each excessive count and a normal near-limit case.
- [ ] Census every `read_section_table` and `read_string_pool` consumer.

Acceptance: the new limit tests fail on HEAD for acceptance/allocation behavior.
Commit: —

### Phase 2 — the fix

- [ ] Add validated parser budgets before allocation and iteration.
- [ ] Apply the shared result/budgets to every in-scope consumer.

Acceptance: red tests pass and valid compiler-produced fixture packages retain
their ABI/vendor/metadata results.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Run the oversized-artifact request reproduction and verify rejection.

Acceptance: full suite green; oversized input cannot create proportional memory
or backend work.
Commit: —
