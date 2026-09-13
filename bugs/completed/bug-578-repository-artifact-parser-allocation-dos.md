# bug-578: repository artifact parsing permits multi-hundred-megabyte allocations

Last updated: 2026-09-12
Effort: medium (1h–2h)
Severity: HIGH
Class: Security / denial of service

Status: Fixed
Regression Test: `repository/src/abi.rs` —
`string_pool_rejects_excessive_entry_count`,
`string_pool_rejects_excessive_aggregate_bytes`,
`every_consumer_rejects_an_oversized_string_pool`,
`section_table_rejects_excessive_section_count`,
`abi_index_rejects_excessive_export_count`,
`package_meta_rejects_excessive_field_count`, plus the positive pins
`a_real_compiler_produced_package_still_parses` and
`payloads_exactly_at_every_ceiling_are_still_accepted`;
`repository/src/server.rs` —
`oversized_string_pool_is_rejected_at_the_request_boundary`.

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

`repository/src/abi.rs` unit tests (all six RED on the pre-fix parser,
measured with the guards stripped in a `git worktree add --detach` copy):

```
cargo test -p mfb_repository --lib abi:: --no-fail-fast
```

- `string_pool_rejects_excessive_entry_count` — pre-fix: `Ok`, one `String` per
  declared entry.
- `every_consumer_rejects_an_oversized_string_pool` — the same pool reached
  through `parse_abi_index`, `parse_vendor_blobs`, `parse_manifest_metadata`
  and `abi_index_json`.
- `string_pool_rejects_excessive_aggregate_bytes`
- `section_table_rejects_excessive_section_count`
- `abi_index_rejects_excessive_export_count`
- `package_meta_rejects_excessive_field_count`

Request level, `repository/src/server.rs`:

```
cargo test -p mfb_repository --lib oversized_string_pool_is_rejected_at_the_request_boundary
```

Pre-fix failure: `panicked ... an over-ceiling pool must not validate` — a
~4 MiB artifact (a sixteenth of `MAX_BODY_BYTES`) came back `valid: true`.

### What was measured, not assumed

A temporary instrumented run of the pre-fix `read_string_pool` on a 48 MiB pool
of zero-length entries reported:

```
MEASURE wire_bytes=50331652
MEASURE entries=12582912 vec_capacity=12582912 string_struct_bytes=24 total_bytes=301989888
```

48 MiB of wire bytes became 12,582,912 `String`s — **288 MiB of `String`
headers alone**, on a 512 MiB server, confirming the reported figure.

Note that bug-276 R8 had already capped the *pre-allocation* at
`count.min(bytes.len() / 4)`. That is not an absolute bound: `bytes.len() / 4`
is exactly the number of empty entries the attacker supplies, so the cap was
satisfied by the attack payload and the pool was still fully built. Only an
absolute entry ceiling closes it.

## Root Cause

Confirmed as filed, with two additions found while auditing:

- `read_string_pool` — no absolute entry or aggregate-byte ceiling. The
  dominant amplification: 24 bytes of `String` header per 4 wire bytes, ~6x.
- `read_section_table` — table sized only by the input body (~2.7 million
  24-byte entries inside a 64 MiB body).
- `read_abi_exports` — no export ceiling; 38 wire bytes per export means
  ~1.7 million `hex::encode` allocations per request.
- **(new)** `parse_package_description` — section 18's field loop was bounded
  only by truncation at 6 bytes per field, ~10 million iterations.
- **(new)** `abi_index_json` swallows the parser error, so before the fix the
  best-effort path paid the full pool cost and then reported `{}`.

## Docs vs code: the "shared parse" goal

The original Goal asked to "parse shared section/string-pool state once per
request where practical". Measured: `validate_package_request` and the publish
handler call the parsers **sequentially**, never concurrently
(`server.rs:2870`, `2881`, `2903`, `3163`, `3198`), so peak resident memory is
one pool, not four. With the entry ceiling in place that peak is ~24 MiB. The
shared-view refactor would therefore buy CPU, not the memory bound this bug is
about, while changing the error semantics of four best-effort call sites. It is
deliberately **not** done here; the ceilings are the fix. Recorded rather than
silently dropped.

## Ceilings and their derivation

Derived from the compiler's *actual* maxima, censused over every committed
compiler-produced package (`packages/*/*.mfp`):

| package     | sections | pool entries | pool bytes | ABI exports | meta fields |
|-------------|---------:|-------------:|-----------:|------------:|------------:|
| cli         |       11 |           45 |        423 |           8 |           1 |
| json_schema |       12 |          628 |      9,175 |          24 |           1 |
| jwt         |       12 |          914 |     12,763 |          48 |           1 |
| libsnd      |       14 |           56 |        669 |          12 |           1 |
| mustache    |       12 |          305 |      4,112 |          12 |           1 |
| sqlite3     |       14 |           64 |        767 |          28 |           1 |
| yaml        |       12 |          356 |      4,600 |          11 |           1 |

| constant                  |     value | largest real | headroom | worst-case cost |
|---------------------------|----------:|-------------:|---------:|-----------------|
| `MAX_MFPC_SECTIONS`       |       256 |           14 |      18x | 256 map entries |
| `MAX_STRING_POOL_ENTRIES` | 1,048,576 |          914 |   1,147x | ~24 MiB         |
| `MAX_STRING_POOL_BYTES`   |    32 MiB |     12.7 KiB |   2,600x | 32 MiB          |
| `MAX_ABI_EXPORTS`         |   262,144 |           48 |   5,461x | ~10 MiB section |
| `MAX_PACKAGE_META_FIELDS` |       256 |            1 |     256x | no allocation   |

`MAX_MFPC_SECTIONS` is the tightest ratio and is still 18x: the writer *defines*
only fourteen section ids (1..=8, 10, 11, 15..=18) and emits at most all
fourteen (`src/binary_repr/writer.rs:1095`), so 256 leaves room for every id the
format is plausibly ever going to define.

## Goal

- Enforce explicit, documented ceilings for MFPC sections, string-pool entries
  and bytes, ABI exports and package-meta fields before allocation or unbounded
  iteration.

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
- `repository/src/backfill.rs:run` — latent consumer of the same parsers; needs
  no edit, because it already routes every parser `Err` into its operator-facing
  `unparseable` counter and diagnostic (`backfill.rs:96`, `117`, `130`). A
  ceiling rejection surfaces there as one bounded line, exactly as a truncation
  rejection already did.

## Fix Design

Format-level constants derived from the compiler's actual maxima, checked
against the declared count **before** any `Vec` growth or loop entry. A
request-body limit alone is insufficient because the in-memory representation
is ~6x the wire bytes that declare it.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add red unit tests for each excessive count and a normal near-limit case.
- [x] Census every `read_section_table` and `read_string_pool` consumer
      (`server.rs:2870,2881,2903,3163,3198`; `backfill.rs:96,117,130`).

Acceptance: the six new limit tests plus the request-level test failed on the
pre-fix parser; both positive pins passed before *and* after.
Commit: f2368455b

### Phase 2 — the fix

- [x] Add validated parser budgets before allocation and iteration.
- [x] Apply the shared result/budgets to every in-scope consumer.

Positive pins (mandatory — a ceiling that refuses a legitimate package is the
failure mode this fix must not introduce):

- `a_real_compiler_produced_package_still_parses` — parses the committed
  `packages/libsnd/libsnd.mfp` through `package::parse_mfp_package` and asserts
  the exact ABI index (12 exports), all seven section-10 `vendor` locators with
  their hashes/arch/libc, the signed manifest author/url, the section-18
  description, all 14 sections and all 56 pool entries. libsnd is the widest
  shape the compiler emits.
- `payloads_exactly_at_every_ceiling_are_still_accepted` — a payload sitting
  exactly *on* each of the five ceilings is still accepted, so an off-by-one in
  the rejecting direction fails loudly.

One pre-existing test was corrected, not re-baselined:
`string_pool_does_not_preallocate_beyond_what_the_section_can_hold` (bug-276 R8)
asserted a `u32::MAX` count is refused with a *truncation* message. The
behaviour it protects — the reservation follows the section, not the claim — is
untouched; only its choice of `u32::MAX` was over-specified, since that now
trips the strictly stronger entry ceiling first. It now probes the same
invariant at `MAX_STRING_POOL_ENTRIES` (a count the ceiling admits) *and*
additionally asserts `u32::MAX` is refused on the count alone.

Commit: f2368455b

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository --no-fail-fast` — exit 0 (341 + 21 passed, 0 failed).
- [x] Run the oversized-artifact request reproduction and verify rejection
      (`oversized_string_pool_is_rejected_at_the_request_boundary`).

Acceptance: full suite green; a ~4 MiB hostile artifact is refused with one
bounded diagnostic instead of validating and building a multi-hundred-megabyte
pool.
Commit: f2368455b
