# plan-61-A: Server-side metadata capture

Last updated: 2026-07-21
Overall Effort: x-large (1d–3d) — see `plan-61-repo-web.md`
Effort: medium (1h–2h)
Depends on: nothing (beyond the plan-61 Prerequisites)
Produces:
- `package_versions.author`, `package_versions.url`, `package_versions.description` columns
- `package_version_targets` table `(package_version_id, blob_hash, os, arch, libc, lib_type, logical, source)`
- `VendorBlobRef { logical, source, hash, os, arch, libc, lib_type }` in `repository/src/abi.rs`
- `store.package_metadata(ident, version) -> VersionMetadata`
- `store.package_targets(package_version_id) -> Vec<TargetRow>`
- `mfb-repo backfill-metadata` subcommand

Gets human-facing and platform metadata into the registry database at publish
time. **No `.mfp` format change is required by this sub-plan** — every field it
persists is already present in the bytes the server receives and already parsed;
the server simply discards it today. The `description` column is created here but
stays NULL until plan-61-E.

The single behavioral outcome: after publishing a package that ships native
blobs for two platforms, `sqlite3 <db> "select os, arch, libc from
package_version_targets"` returns both platform rows, and the author and url from
the artifact's signed MANIFEST section are readable from `package_versions`.

References:
- `plan-61-repo-web.md` §2 (Current State), §Prerequisites
- `repository/src/abi.rs` — section-10 and string-pool parsing
- `repository/src/store.rs:163-338` — the schema
- `src/docs/spec/tooling/01_project-manifest.md:180-240` — target vocabulary

## Prerequisites

See `plan-61-repo-web.md` §Prerequisites. **bug-347 must be fixed before this
sub-plan begins** — re-run its command and update the status there, do not trust
the recorded snapshot.

## 1. Goal

- Publish persists `author`, `url`, and the native target matrix.
- Already-published packages are backfillable by re-parsing their stored blobs,
  with no republish and no publisher action.

### Non-goals

- No `.mfp` format change. If this sub-plan finds itself needing one, that is a
  plan defect — stop and record it in `plan-61-repo-web.md` §Corrections.
- No new HTTP route. A only writes to the database; B reads from it.
- No `description` *value* — the column is created NULL-able and left NULL.
- No change to `GET /index/:ident`'s response shape.

## 2. Current state

`package_versions` (`repository/src/store.rs:263`) is
`(id, package_id, version, hash, state, abi_index TEXT DEFAULT '{}', created_at)`.
`publish_package_version` (called at `repository/src/server.rs:2079`) persists
owner_id, ident, version, content hash, blob ref, abi_index JSON, and vendor
hashes — nothing else.

`author` and `url` are parsed into `MfpPackage` (`repository/src/package.rs:22-27`)
from the header and dropped. They also exist, interned, in MANIFEST section 1
(`src/binary_repr/writer.rs:992-1014`), which is inside the signed payload.

`repository/src/abi.rs:104-112` skips the platform axis:

```rust
// os, arch: interned string ids (skipped — not needed here).
offset += 4; // os
offset += 4; // arch
let _libc = read_u8(bytes, offset)?;
```

### Verified properties

- **VERIFIED — the string pool is already loaded before section 10 is walked.**
  `parse_vendor_blobs` at `repository/src/abi.rs:69-73` fetches
  `SECTION_STRING_POOL` and calls `read_string_pool` (`abi.rs:197-223`) *before*
  `read_native_vendor_locators`. `table_string` (`abi.rs:133-138`) already does
  bounds-checked id→string resolution and is already called for `logical` and
  `source` on the same locator. Resolving `os` and `arch` needs no new parsing,
  no new section, and no new I/O.
- **VERIFIED — section 2 is always present.** `parse_abi_index` hard-errors
  without it (`repository/src/abi.rs:146-148`), and `encode_native_library_table`
  interns into it by construction, so a package with a section 10 necessarily has
  a section 2.

### Section-10 locator layout (little-endian)

From `src/binary_repr/sections.rs:718-754` (writer) and
`repository/src/abi.rs:76-131` (reader):

```
u32 entry_count
  per entry:
    u32 logical        <- interned string id
    u32 locator_count
      per locator:
        u32 os         <- interned string id
        u32 arch       <- interned string id ("" = any-arch wildcard)
        u8  libc       <- raw enum, NOT interned
        u8  lib_type   <- raw enum (0=system, 1=vendor)
        u32 source     <- interned string id
        [u8; 32] hash  <- present IFF lib_type == 1
```

`libc` values (`src/binary_repr/mod.rs:353-359`): `0` unspecified, `1` glibc,
`2` musl. `lib_type`: `0` system, `1` vendor.

Vocabulary (`src/manifest/libraries.rs:96-101`, `src/target.rs:23`): `os` ∈
{`macos`, `linux`}; `arch` ∈ {`aarch64`, `x86_64`, `riscv64`} or `""`.

## 3. Design — two gotchas that decide the schema

**Gotcha 1: `arch == ""` is the any-arch wildcard, not missing data.** Store it
as SQL `NULL` and render it as "any" in the UI, but never conflate it with an
absent row. A locator with `arch = ""` legitimately matches every architecture on
that OS.

**Gotcha 2: do not accumulate targets per distinct blob hash.** `repository/src/server.rs:2288`
dedupes blob *existence probes* by hash — correctly, since probing the same blob
twice is waste. But one vendored file is legitimately listed under several
platform locators, so a platform set accumulated from the deduped hash set will
**under-report targets**. Accumulate per *locator*, in the loop at
`repository/src/server.rs:2297-2306`, which already iterates the right way.

This is the one place in A where a plausible-looking implementation is silently
wrong, so the phase has a test dedicated to it (Phase 2).

`package_version_targets` is a separate table rather than a JSON column on
`package_versions` because B needs to query it ("which packages ship linux/musl
blobs?") and because a version legitimately has many rows.

## 4. Read author/url from the signed MANIFEST, not the plaintext header

The header copy is a plaintext fast-scan convenience. The MANIFEST copy lives
inside `packageBinaryRepr`, covered by `packageBinaryHash`, covered by the
package signature. For a registry whose purpose is transparency, the rendered
value must be the signed one.

Practically: `MfpPackage` already gives the header values; A must additionally
read section 1 via the existing section splitter and interned string pool, and
**assert the two agree**. A mismatch is a malformed or tampered artifact and must
be rejected at publish with a diagnostic, not silently resolved in favor of
either copy.

## Phases

> Tick `- [x]` in the same commit as the work. `- [~]` for partial, with one line
> on what remains. `- [x] ~~text~~ — moot: <evidence>` rather than deleting.
> **An unticked box means NOT DONE.**

### Phase 1 — Schema and migration

Adds the columns and table. Safe alone: nothing writes to them yet.

- [ ] Add to `repository/src/store.rs` schema (`:163-338`): `ALTER`-equivalent
      columns on `package_versions` — `author TEXT`, `url TEXT`,
      `description TEXT`, all NULL-able.
- [ ] Add table `package_version_targets (package_version_id INTEGER NOT NULL
      REFERENCES package_versions(id), blob_hash TEXT, os TEXT NOT NULL,
      arch TEXT, libc TEXT, lib_type INTEGER NOT NULL, logical TEXT NOT NULL,
      source TEXT NOT NULL)` with an index on `package_version_id`. `arch` NULL
      = any-arch wildcard.
- [ ] Follow the existing migration idiom in `store.rs` — read how the current
      schema handles version upgrades on an existing database file before
      inventing one. An existing deployed database (`repository/DEPLOY.md`: Fly.io
      volume) must open cleanly after this change.
- [ ] Tests: a `store.rs` unit test that opens a database created *before* this
      change and confirms the new columns exist and are NULL.

Acceptance: `cargo test -p mfb_repository store` passes, and opening a
pre-migration database file succeeds with the new columns present and NULL.
Commit: —

### Phase 2 — Capture native targets

The one place a plausible implementation is silently wrong.

- [ ] In `repository/src/abi.rs:44-48`, extend `VendorBlobRef` with `os: String`,
      `arch: Option<String>` (None ⇔ `""` wildcard), `libc: Option<Libc>`,
      `lib_type: u8`.
- [ ] In `repository/src/abi.rs:104-112`, replace the two `offset += 4` skips
      with `table_string(strings, read_u32(bytes, offset)?)?` for `os` and
      `arch`, and decode the `libc` u8 per the mapping in §2. Map `""` arch to
      `None`.
- [ ] In `repository/src/server.rs:2297-2306`, write one
      `package_version_targets` row **per locator** (not per distinct hash) as
      part of the existing publish transaction.
- [ ] Tests: a unit test in `repository/src/abi.rs` asserting os/arch/libc are
      resolved for a fixture with two platforms. **Plus the regression test for
      gotcha 2**: a package where one blob hash appears under two distinct
      platform locators must produce **two** target rows. This test is the point
      of the phase — write it first and watch it fail against a
      dedupe-by-hash implementation.
- [ ] Tests: a locator with `arch = ""` produces a row with `arch IS NULL`, and
      is distinguishable from a locator with a concrete arch.

Acceptance: publishing a fixture package whose section 10 lists the same vendor
blob under `linux/x86_64/glibc` and `linux/x86_64/musl` yields exactly two rows in
`package_version_targets`, with distinct `libc` values, and the same `blob_hash`.
Commit: —

### Phase 3 — Capture author and url

- [ ] Parse MANIFEST section 1 server-side to obtain the interned `author` and
      `url`. Reuse the existing section splitter
      (`repository/src/abi.rs:168-195`) and `read_string_pool`.
- [ ] Assert the section-1 values equal the header values from `MfpPackage`
      (`repository/src/package.rs:22-27`). On mismatch, reject the publish with a
      distinct error message naming both values. Do **not** prefer one silently.
- [ ] Persist the section-1 values into `package_versions.author` / `.url` in the
      publish transaction.
- [ ] Tests: publish with author+url set → both persisted; publish with both
      empty → NULL, not `""`; a hand-built artifact whose header and MANIFEST
      disagree → publish rejected with the mismatch error.

Acceptance: a published package's `author`/`url` round-trip from the signed
MANIFEST into `package_versions`, and a header/MANIFEST mismatch is rejected.
Commit: —

### Phase 4 — Backfill (largest blast radius: touches every stored blob)

Last, because it reads every package blob on the server.

- [ ] Add a `backfill-metadata` subcommand to `repository/src/main.rs`, alongside
      the existing `reanchor` / `init-root` / `gc` operator ceremonies (usage
      block at `main.rs:11-33`). It re-parses every stored package blob and
      populates the columns and target rows added above.
- [ ] Make it idempotent and resumable: re-running it must not duplicate
      `package_version_targets` rows. Delete-then-insert per version, inside a
      transaction per version.
- [ ] It must **not** fail the whole run on one unparseable blob — log the ident
      and version, skip, continue, and report a count at the end. An old blob
      that no longer parses is exactly the kind of thing this surfaces.
- [ ] Tests: a store test that publishes two versions with the columns stubbed
      NULL, runs the backfill, and asserts both are populated; and that a second
      run changes nothing (row counts identical).

Acceptance: `mfb-repo backfill-metadata --dbpath <db> --datapath <dir>` populates
author/url/targets for pre-existing versions, is idempotent across two runs, and
reports a skip count rather than aborting on a bad blob.
Commit: —

## Validation Plan

- Tests: inline `#[cfg(test)] mod tests` in `repository/src/abi.rs`,
  `repository/src/store.rs`, and the publish path. Negative cases: wildcard arch,
  header/MANIFEST mismatch, unparseable blob during backfill.
- Coverage check: `sh scripts/coverage.sh && sh scripts/coverage-check.sh`. This
  is the first sub-plan to add code under the newly-in-workspace `repository/`
  crate — confirm the new files appear in the report, not just that the gate is
  green.
- Runtime proof: publish `bindings/sqlite3` (a real package with native
  libraries) against a local `mfb-repo`, then
  `sqlite3 <db> "select os, arch, libc, logical from package_version_targets"`
  and confirm the rows match its `project.json` `libraries` block.
- Doc sync: `src/docs/spec/package-manager/01_repository-protocol.md` — the
  operational section, for the new `backfill-metadata` subcommand. No wire-format
  topic changes, because no wire format changes.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` and
  `cargo test -p mfb_repository`.

## Open Decisions

- **Store `libc` as an integer or a token string?** *Recommended:* the token
  string (`"glibc"` / `"musl"` / NULL), so B's JSON and C's HTML need no mapping
  table and the database is readable by an operator with `sqlite3`. The integer
  is one byte smaller and nothing else.

## Corrections

- *(none yet)*
