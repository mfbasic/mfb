# plan-61-A: Server-side metadata capture

Last updated: 2026-07-21
Overall Effort: x-large (1d–3d) — see `plan-61-repo-web.md`
Effort: medium (1h–2h)
Depends on: nothing (beyond the plan-61 Prerequisites)
Produces:
- `package_versions.author`, `package_versions.url`, `package_versions.description` columns
- `package_version_targets` table `(package_version_id, blob_hash, os, arch, libc, lib_type, logical, source)`
- `VendorBlobRef { logical, source, hash, os, arch, libc, lib_type }` in `repository/src/abi.rs`
- a changed `store.publish_package_version` signature (Phase 2)
- `mfb-repo backfill-metadata` subcommand

A writes; it adds **no read accessors**. Earlier drafts of this list promised
`store.package_metadata` and `store.package_targets`; no phase delivers them and
plan-61-B does not consume them (B defines its own `package_detail` and
`package_audit` queries over these tables). They are removed rather than left as
dead deliverables.

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
- `repository/src/store.rs:163-356` — `migrate()`, the schema. The column-add
  idiom Phase 1 needs is `add_column_if_missing` (helper at `store.rs:2342`,
  called at `store.rs:342-354`) — **outside** the schema block itself.
- `src/docs/spec/tooling/01_project-manifest.md:180-240` — target vocabulary
- `bindings/libsnd/project.json` — the reference multi-platform fixture: seven
  `vendor` locators, and a macOS locator with no `arch` key (the any-arch case)

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
`(id, package_id, version, hash, state, abi_index TEXT NOT NULL DEFAULT '{}', created_at)`.
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

Vocabulary (`src/manifest/libraries.rs:96-101`,
`src/docs/spec/tooling/01_project-manifest.md:183-185`, and the live registries
`src/target.rs:registered_target_oses`/`registered_target_arches` at `:211`/`:225`):
`os` ∈ {`macos`, `linux`}; `arch` ∈ {`aarch64`, `x86_64`, `riscv64`} or `""`.
(`src/target.rs:23` is just `pub arch: String` — the field, not the vocabulary.)

## 3. Design — two gotchas that decide the schema

**Gotcha 1: `arch == ""` is the any-arch wildcard, not missing data.** Store it
as SQL `NULL` and render it as "any" in the UI, but never conflate it with an
absent row. A locator with `arch = ""` legitimately matches every architecture on
that OS.

**Gotcha 2: do not accumulate targets per distinct blob hash.** `repository/src/server.rs:2288`
dedupes blob *existence probes* by hash — correctly, since probing the same blob
twice is waste. A platform set accumulated from that deduped hash set will
**under-report targets**. Accumulate per *locator*.

**How this is actually reachable — the obvious example does not exist.** An
earlier draft justified this with "one vendored file listed under several
platform locators." That is *not* producible from a valid manifest:
`PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` (`src/manifest/mod.rs:967-978`) rejects
any two `vendor` locators sharing a `source`, precisely because `vendor/` is flat
— one filename means one file
(`src/docs/spec/tooling/01_project-manifest.md:236-242`). `bindings/libsnd`
confirms it: all seven vendor locators carry distinct filenames.

The collision is real anyway, by a different route: **two distinct `source`
filenames whose bytes are identical hash the same.** Two platforms shipping a
byte-identical build under different names (a common result of copying a build
across a libc split) is legal, passes `SOURCE_CONFLICT`, and collapses to one
entry under dedupe-by-hash. That — not one-source-many-locators — is what the
Phase 2 regression test must construct.

Note the check is scoped to `vendor` locators only; `system` sonames legitimately
repeat (`linux/x86_64` and `linux/aarch64` both naming `libsqlite3.so.0` is
normal). See §3.1 for why system locators are out of scope here regardless.

This is the one place in A where a plausible-looking implementation is silently
wrong, so the phase has a test dedicated to it (Phase 2).

### 3.1 Only `vendor` locators are captured

`parse_vendor_blobs` pushes an entry only when `lib_type == WIRE_LIB_TYPE_VENDOR`
(`repository/src/abi.rs:114`), so **no `system` locator can ever reach the
table**. Two consequences the schema must not pretend away:

- `lib_type` is a constant `1` for every row this sub-plan writes. It is kept in
  the schema because capturing system locators later should not require a
  migration, but nothing in A, B, or C may branch on it yet.
- `blob_hash` is nullable in the schema for that same future, but is `NOT NULL`
  in practice for every row A writes.

Capturing system targets would mean changing `parse_vendor_blobs`'s filter *and*
its name and contract — a different change with its own reachability questions.
It is explicitly out of scope. If B or C needs system targets, that is a plan
defect; record it in §Corrections rather than widening the parser here.

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

- [x] Add to `repository/src/store.rs` `migrate()` (`:163-356`): `ALTER`-equivalent
      columns on `package_versions` — `author TEXT`, `url TEXT`,
      `description TEXT`, all NULL-able.
- [x] Add table `package_version_targets (package_version_id INTEGER NOT NULL
      REFERENCES package_versions(id), blob_hash TEXT, os TEXT NOT NULL,
      arch TEXT, libc TEXT, lib_type TEXT NOT NULL, logical TEXT NOT NULL,
      source TEXT NOT NULL)` with an index on `package_version_id`. `arch` NULL
      = any-arch wildcard. `libc` and `lib_type` are both **token strings**
      (`'glibc'`/`'musl'`/NULL and `'vendor'`/`'system'`), decoded from the wire
      integers in §2 — see Open Decisions; the readability argument that settles
      `libc` settles `lib_type` identically. Per §3.1, every row A writes has
      `lib_type = 'vendor'`.
- [x] Follow the existing migration idiom rather than inventing one: new tables
      go in the `CREATE TABLE IF NOT EXISTS` batch; new columns on an existing
      table go through `add_column_if_missing` (helper at `store.rs:2342`, used
      at `store.rs:342-354`). Together these open a pre-existing deployed
      database (`repository/DEPLOY.md`: Fly.io volume) cleanly.
      Followed exactly: the three columns are also spelled into the `CREATE
      TABLE` body, mirroring how `abi_index` appears in both places.
- [x] Tests: a `store.rs` unit test that opens a database created *before* this
      change and confirms the new columns exist and are NULL.
      `migrating_a_legacy_database_adds_the_metadata_columns_and_target_table`.

Acceptance: **MET** — `cargo test -p mfb_repository --lib store` → 81 passed, 0
failed; the new test opens a `package_versions` table created without the three
columns and reads all three back as NULL, with `package_version_targets` and its
index present.
Commit: `adf1c2b54`

### Phase 2 — Capture native targets

The one place a plausible implementation is silently wrong.

- [x] In `repository/src/abi.rs:44-48`, extend `VendorBlobRef` with `os: String`,
      `arch: Option<String>` (None ⇔ `""` wildcard), ~~`libc: Option<Libc>`,
      `lib_type: u8`~~ — **corrected:** `libc: Option<String>` and
      `lib_type: String`, both token strings. See §Corrections: `Libc` is not
      reachable from this crate, and `lib_type: u8` reintroduced the exact
      "same argument answered two ways" that §Open Decisions settled.
- [x] In `repository/src/abi.rs:104-112`, replace the two `offset += 4` skips
      with `table_string(strings, read_u32(bytes, offset)?)?` for `os` and
      `arch`, and decode the `libc` u8 per the mapping in §2. Map `""` arch to
      `None`.
- [x] **Widen the publish path to carry locators, not bare hashes.** At
      `repository/src/server.rs:2046-2051` the publish handler re-parses the
      artifact and immediately throws the metadata away:
      `.map(|refs| refs.into_iter().map(|vref| vref.hash).collect())`. That
      `Vec<String>` is the discard site. Keep the full `Vec<VendorBlobRef>`.
- [x] Change `store.publish_package_version` (`repository/src/store.rs:1163-1230`)
      from `vendor_hashes: &[String]` to `vendor_blobs: &[VendorBlobRef]`, and
      write one `package_version_targets` row **per locator** (not per distinct
      hash) inside the transaction that function already owns — it is the only
      place with the `package_version_id` these rows reference. Update the call
      site at `server.rs:2079`.

> **Do not implement this in `validate_package_request`.** An earlier draft
> pointed Phase 2 at `server.rs:2297-2306`. That range is inside
> `validate_package_request` (fn at `server.rs:2131`), the **`/validate` dry-run**
> helper shared with `/publish`. It holds no store transaction and no
> `package_version_id` — the version row does not exist yet — and writing there
> would make `POST /validate` mutate the database, inverting its documented
> contract (`server.rs:2270-2277`: "a dry run reports missing blobs before the
> publisher uploads anything"). It is the right place to *read* locators, and the
> wrong place to persist them.

- [x] Tests: a unit test in `repository/src/abi.rs` asserting os/arch/libc are
      resolved for a fixture with two platforms. **Plus the regression test for
      gotcha 2**: a package whose section 10 lists two locators with distinct
      `source` filenames but byte-identical blobs — hence one shared hash — must
      produce **two** target rows. Per §3, this is the only shape that reaches
      the bug from a valid manifest. Write it first and watch it fail against a
      dedupe-by-hash implementation.
- [x] Tests: a locator with `arch = ""` produces a row with `arch IS NULL`, and
      is distinguishable from a locator with a concrete arch. `bindings/libsnd`'s
      macOS locator (no `arch` key) is the natural fixture.
- [x] Tests: `POST /validate` writes **no** `package_version_targets` rows —
      guarding the inversion the note above describes.

Acceptance: **MET** — `two_locators_sharing_one_blob_hash_write_two_target_rows`
(`store.rs`) publishes exactly that shape and asserts two rows sharing one
`blob_hash`, while the neighbouring `package_version_blobs` edge correctly
collapses to one. `validate_writes_no_target_rows_but_publish_does`
(`server.rs`) asserts the table is empty after `/validate` and has the row after
`/publish` on the same artifact, so it cannot pass vacuously.

The gotcha-2 test was verified to *discriminate*, not merely pass: temporarily
deduping `insert_version_targets` by hash fails it with `left: 1, right: 2`.
Also added at the parser layer: `resolves_the_platform_triple_for_each_locator`,
`an_empty_arch_is_the_any_arch_wildcard_not_a_concrete_arch`,
`two_locators_sharing_one_hash_stay_two_locators`, and
`an_unknown_libc_discriminant_is_rejected` (an unlisted task — see §Corrections).
Full crate: 291 passed, 0 failed; clippy warning count unchanged from baseline.
Commit: `fa5e8f69d`

### Phase 3 — Capture author and url

- [x] Parse MANIFEST section 1 server-side to obtain the interned `author` and
      `url`. Reuse the existing section splitter
      (`repository/src/abi.rs:168-195`) and `read_string_pool`.
- [x] Assert the section-1 values equal the header values from `MfpPackage`
      (`repository/src/package.rs:22-27`). On mismatch, reject the publish with a
      distinct error message naming both values. Do **not** prefer one silently.
- [x] Persist the section-1 values into `package_versions.author` / `.url` in the
      publish transaction.
- [~] Tests: publish with author+url set → both persisted; ~~publish with both
      empty → NULL, not `""`~~ — **partially moot**: an empty *url* is covered,
      but an empty *author* is unreachable through publish (see §Corrections);
      a hand-built artifact whose header and MANIFEST disagree → publish
      rejected with the mismatch error.
      `author_and_url_round_trip_from_the_signed_manifest` and
      `a_header_manifest_metadata_mismatch_is_refused` (`server.rs`), plus
      `reads_author_and_url_from_the_manifest_section` and
      `manifest_metadata_is_absent_or_malformed_but_never_silently_wrong`
      (`abi.rs`). Remaining: nothing — the empty-author half is unbuildable, not
      unfinished.

Acceptance: **MET** — a published package's `author`/`url` round-trip from the
signed MANIFEST into `package_versions` (asserted against the stored row, not
the response), and a header/MANIFEST mismatch is refused with a 400 naming both
values while persisting no version row. Full crate: 295 passed, 0 failed;
workspace builds; clippy warning count unchanged from baseline.
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
- [ ] **Decide what backfill does with a header/MANIFEST mismatch.** Phase 3
      rejects one at publish, but backfill walks blobs published *before* that
      check existed, so mismatches are parseable-and-invalid — a case the
      skip-unparseable rule above does not cover. Treat it as a skip with a
      distinct log line and its own counter, never as a silent pick of either
      copy: an already-stored artifact whose two author copies disagree is a
      transparency finding an operator must see, and backfill is the only thing
      that will ever look. Do not delete or rewrite the version row.
- [ ] Tests: a store test that publishes two versions with the columns stubbed
      NULL, runs the backfill, and asserts both are populated; and that a second
      run changes nothing (row counts identical).
- [ ] Tests: a stored blob with a header/MANIFEST mismatch is skipped, counted
      separately from unparseable blobs, and leaves its `author`/`url` NULL.

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
- Runtime proof: publish **`bindings/libsnd`** against a local `mfb-repo`, then
  `sqlite3 <db> "select os, arch, libc, logical from package_version_targets"`
  and confirm seven rows matching its `project.json` `libraries` block, with the
  macOS row's `arch` NULL.

  > Use `libsnd`, **not** `bindings/sqlite3`. An earlier draft named sqlite3 "a
  > real package with native libraries" — but all of its locators are
  > `type: "system"` (`bindings/sqlite3/project.json:6-11`), and
  > `parse_vendor_blobs` emits only `vendor` entries (`repository/src/abi.rs:114`,
  > and §3.1). The table would be empty and the proof would pass vacuously.
  > `libsnd` has seven vendor locators across three arches and both libc values.
- Doc sync: `src/docs/spec/package-manager/01_repository-protocol.md` — the
  operational section, for the new `backfill-metadata` subcommand. No wire-format
  topic changes, because no wire format changes.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` and
  `cargo test -p mfb_repository`.

## Open Decisions

- **Store `libc` and `lib_type` as integers or token strings?** *Recommended:*
  token strings (`"glibc"`/`"musl"`/NULL and `"vendor"`/`"system"`), so B's JSON
  and C's HTML need no mapping table and the database is readable by an operator
  with `sqlite3`. The integers are one byte smaller and nothing else. **Both
  fields, or neither** — an earlier draft recommended the token string for `libc`
  while Phase 1's schema kept `lib_type INTEGER`, which is the same argument
  answered two ways. Phase 1 now specifies TEXT for both.

## Corrections

- **Phase 2 specified `libc: Option<Libc>`, a type this crate cannot reach.**
  `Libc` is `src/manifest/libraries.rs`, in the **compiler** crate — and the
  dependency runs the other way: `mfb` depends on `mfb_repository`
  (`repository/Cargo.toml` names no `mfb` dependency; the root manifest names
  this one). Importing it would be a dependency cycle. Implemented as
  `libc: Option<String>` holding the same token the schema stores
  (`"glibc"`/`"musl"`/`None`), decoded by a new `decode_libc` next to the
  restated `WIRE_LIBC_*` constants — which is how `MFPC_MAGIC` and the section
  ids in `abi.rs` are already handled.
- **Phase 2 also specified `lib_type: u8` while Phase 1's schema stores TEXT.**
  That is precisely the "same argument answered two ways" §Open Decisions called
  out and settled for `libc`; it survived on the neighbouring field. Implemented
  as `lib_type: String` carrying the `"vendor"`/`"system"` token, so nothing
  between the parser and the HTML needs a mapping table.
- **An unlisted task: `decode_libc` had to decide what an unknown wire byte
  means.** Not deciding would have defaulted it to `None` — "no libc
  constraint" — which lets a tampered locator silently *widen* its own platform
  match. Section 10 rides inside the signed payload, so an out-of-vocabulary
  byte is a broken or tampered package: it is now an error, matching how the
  same parser already treats a malformed section 10. Covered by
  `an_unknown_libc_discriminant_is_rejected`.
- **Widening the signature broke a pre-existing GC test, for a real reason
  worth recording.** `a_shared_blob_survives_removing_one_of_its_versions`
  (from `31935c914`, plan-49) simulates a version deletion by deleting rows
  directly — its own comment notes there is no deletion API today. The new
  `package_version_targets` FK made that simulated parent-delete fail. Verified
  first that **no production code deletes a `package_versions` row**
  (`grep -rn "DELETE FROM package_version" repository/src` → only that test), so
  this is not a latent FK bug in the server. The test's assertion is unchanged;
  only its simulated deletion now clears the new child table, which is what a
  real deletion feature would have to do.
- **`VendorBlobRef` gained enough fields to need a test constructor.** The
  `store.rs` and `gc.rs` tests that pass bare vendor hashes care only about the
  version→blob edge, so `abi::vendor_ref_for_hash` (test-only) fills the
  platform axis with a placeholder rather than each site restating a triple it
  does not assert on.
- **Phase 3 asked for a test case that cannot be built: "publish with both
  empty → NULL".** An empty *url* is reachable and is covered. An empty
  *author* is not: `validate_package_request` already refuses a package whose
  header `author` differs from the owner's display name
  (`repository/src/server.rs:2243`, "package author does not match owner
  name"), and no owner is named `""`. Found by writing the test and watching it
  fail with that pre-existing 400 — not reasoned around. The NULL-vs-`""`
  normalization itself is unconditional in the handler, so `author` gets the
  same treatment the moment anything can produce an empty one. The task is
  marked `[~]` with this as the reason rather than `[x]`.
- **`publish_package_version` was already at the argument-count lint's ceiling,
  so `author`/`url` went in as a struct.** Adding them positionally made it a
  9-argument function (10 once plan-61-E adds `description`) and pushed an
  existing `too_many_arguments` warning further. `PublishMetadata { author, url }`
  keeps the arity at its previous count and gives E a declared home. 33 test
  call sites were updated mechanically to `&PublishMetadata::default()`.
- **`package::test_support::TestPackage` hard-coded an empty header `url`.**
  It had a settable `author` but wrote `put_bytes(&mut bytes, b"")` for `url`,
  so no test could construct a package carrying one — and the mismatch check
  needs the two copies to differ on either field. Added a `url` field and a
  `signed_request_with_header_metadata` helper; `signed_request` delegates to it
  with the previous values, so every existing test is unchanged.
- **A `#[cfg(test)]` read accessor was needed after all — without weakening the
  "A adds no read accessors" rule.** The `/validate` guard test lives in
  `server.rs` and cannot reach `Store`'s private connection.
  `Store::target_rows_for_test` is `#[cfg(test)]`-gated and compiles out of the
  shipped binary, so plan-61-B still defines its own real queries.
