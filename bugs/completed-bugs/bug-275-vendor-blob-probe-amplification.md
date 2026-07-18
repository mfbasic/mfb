# bug-275: unbounded per-request vendor-blob existence probes from an attacker-controlled section-10 table

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Security (resource amplification / DoS)

Status: Fixed 2026-07-18
Regression Test: `repository/src/abi.rs` —
`abi::tests::oversized_vendor_table_is_rejected_before_probing`,
`abi::tests::normal_sized_vendor_table_still_parses`

## Resolution

`read_native_vendor_locators` rejects a table declaring more than
`MAX_VENDOR_ENTRIES` (1024) entries or more than `MAX_VENDOR_LOCATORS` (4096)
locators in total, both checked on the declared counts before any locator is
parsed. The total is a *running* one across entries, not per-entry: many small
entries reach the same fan-out as one oversized entry, and a per-entry check would
let that straight through.

`validate_package_request` now probes each distinct hash once and reuses the
result, so a library that legitimately lists the same file for several platforms
costs one round trip rather than one per locator. Diagnostics stay per-locator, so
a missing blob still names every logical library and source filename referencing
it.

The limits sit orders of magnitude above any real table (one entry per logical
library, one locator per supported platform triple); `normal_sized_vendor_table_still_parses`
guards that direction.

Verified both directions. With the caps removed the oversized-table test fails with
`truncated u8` — i.e. the parser walks locators until it runs off the payload,
which is precisely the amplification: on a real ~48 MiB payload that is ~1M
locators, each costing a blob-store probe.

Note on writing the test: a table cannot declare locators it does not supply bytes
for, because the parser walks each entry's locators before reaching the next. The
running-total case therefore has to emit real (`system`, hash-free) locators; an
earlier version declared counts with no bytes and failed for the wrong reason.

Doc sync done: the limits and the dedup are recorded in
`src/docs/spec/package-manager/01_repository-protocol.md`.

The plan-48-A vendor-blob check in `validate_package_request` iterates *every*
locator returned by `crate::abi::parse_vendor_blobs(&package.payload)` and calls
`state.blob_store.exists(&vref.hash, BlobKind::Native).await` once per locator,
with no dedup and no cap. `read_native_vendor_locators` reads `entry_count` /
`locator_count` as raw `u32`s and only stops when the offset runs off the payload,
so a ~48–64 MiB payload can encode on the order of ~1M vendor locators (≈46 bytes
each). On the S3 backend each `exists` is a network `head_object`; on local it is
a `stat`. The loop runs to completion (diagnostics accumulate, never
short-circuit). It is reached as soon as the package parses, the ident splits on
`#`, the owner matches the session, and `signature_type == 1` — all attacker
controlled by a self-registered owner, so the expensive signature checks need not
pass first.

The single correct behavior a fix produces: a package whose section-10 vendor
table exceeds a small fixed locator count is rejected up front, and the existence
probes run over a deduplicated, bounded hash set — so one `/validate` or
`/publish` request cannot fan out to ~1M backend operations.

References:

- `planning/old-plans/audit-2-repository.md` REPO-16 (anonymous read
  amplification — different loop; this vendor-blob loop is post-audit-2 code).
- Found during goal-06 review of `repository/src/server.rs`.

## Failing Reproduction

```
# Registered owner (registration is open) posts a valid-structured package
# for their own ident whose section-10 table lists ~1,000,000 vendor locators:
POST /validate   (or /publish)
```

- Observed: one request triggers ~1M `blob_store.exists` calls (S3 HEADs or local
  stats); repeatable up to the 60/min per-owner cap and across many owners →
  CPU / S3-cost / latency amplification.
- Expected: the package is rejected for an over-large vendor table before any
  probing, and legitimate tables probe a bounded, deduped hash set.

## Root Cause

`repository/src/server.rs:2266-2283` (`validate_package_request`, vendor-blob
loop) probes once per parsed locator; `repository/src/abi.rs:60-100`
(`read_native_vendor_locators`) bounds only by payload length, not by a locator
count cap, and there is no dedup before probing.

## Goal

- Reject packages whose section-10 locator count exceeds a small fixed cap.
- Dedup locator hashes before calling `exists`, so N duplicate hashes cost one
  probe.

### Non-goals (must NOT change)

- The legitimate vendor-blob validation semantics for reasonably-sized tables.
- The `.mfp`/section-10 wire format.

## Blast Radius

- `server.rs:validate_package_request` vendor loop — fixed by this bug.
- `abi::read_native_vendor_locators` — add the count cap here (shared by
  validate/publish).
- The string-pool over-allocation in `abi.rs:read_string_pool` is a related but
  distinct amplification — tracked in bug-276 (LOW cluster).

## Fix Design

Add a hard cap on the number of vendor locators parsed in
`read_native_vendor_locators` (reject beyond, e.g., a few thousand), and collect a
`HashSet` of distinct hashes in the validate loop before probing. Rejected
alternative: rate-limiting alone — the 60/min per-owner cap still permits 60M
probes/min/owner, far too high.

## Phases

### Phase 1 — failing test + audit
- [ ] Test: package with an oversized vendor table is rejected pre-probe; audit
      `/publish` path shares the same parser.
### Phase 2 — the fix
- [ ] Add the count cap + dedup.
### Phase 3 — validation
- [ ] Full `repository/` suite green; normal vendor validation unaffected.

## Validation Plan

- Regression test: oversized-table rejection; deduped probing.
- Runtime proof: crafted package no longer fans out to per-locator probes.
- Doc sync: note the vendor-table size limit in the repository protocol spec.

## Summary

A newly-added validation loop trusts an attacker-controlled count; a fixed cap
plus dedup bounds the fan-out. Risk is only picking a cap that never rejects
legitimate packages.
