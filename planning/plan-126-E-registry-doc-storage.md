# plan-126-E: Registry doc storage — extract at publish, backfill the rest

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-126-D

Captures MFPC section 17 at publish time into the registry's database, and extends
the existing `backfill-metadata` sweep to fill it in for versions published before
this landed. After this sub-plan the registry *has* every published package's
documentation; plan-126-F renders it.

Behavioral outcome: publishing a documented package records its doc section, and
`mfb-repo backfill-metadata` fills the same field for every already-published
version whose blob still parses. Nothing is rendered yet and no route changes.

References:

- `repository/src/server.rs:2861-2903` — the publish path's existing section
  parsing, the precedent this mirrors exactly.
- `repository/src/backfill.rs:1-22` — the sweep's two design rules ("one bad blob
  does not abandon the run"; "a mismatch is skipped and counted, never silently
  resolved"). Read them before extending it.
- `repository/src/blobstore.rs:505-535` — why lazy render-on-view was rejected.

## Prerequisites

See plan-126-A § Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-126-D complete (`mfb_wire` decodes section 17) | `grep -c read_package_doc_section wire/src/docs.rs` → 1 | NOT MET |

If plan-126-D is not complete, this sub-plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report
> the status of *all* prerequisites if you stop.

## 1. Goal

- A `package_version_docs` table holding one row per published version that carries
  a section 17, keyed to `package_versions(id)`.
- `POST /publish` extracts the section from the payload it already holds and writes
  the row in the same transaction as the version.
- `mfb-repo backfill-metadata` fills the table for pre-existing versions, obeying
  the sweep's existing skip-and-count rules.
- A `Store` accessor returning the doc section for a package's **latest active**
  version, built on `Store::latest_active_version` from plan-126-A.

### Non-goals (explicit constraints)

- **No route, response or page changes.** This sub-plan is storage only; every
  externally observable surface is identical afterwards. plan-126-F adds the UI.
- **No new network I/O on any request path.** The publish handler already has the
  payload in memory; nothing here fetches a blob during a request.
- **The publish accept/reject decision does not change.** A package whose section 17
  is malformed must still publish — the section "does not affect execution or the
  ABI" (`src/binary_repr/writer.rs:1112-1113`) and refusing a publish over
  documentation would be a new failure mode on a signed-payload path.
- **No change to any signed payload, hash, or transparency-log entry.**
- **`GET /packages/:ident` JSON is unchanged.** No doc field is added to it here.

## 2. Current State

### The publish path already does exactly this, four times

`repository/src/server.rs`'s publish handler holds the full `package.payload` and
parses it repeatedly:

| Parse | Site |
|---|---|
| `parse_vendor_blobs` | `:3163` (and `:2870`) |
| `parse_manifest_metadata` | `:2881` |
| `parse_package_description` | `:2903` |
| `abi_index_json` | `:3198` |

`parse_package_description` (plan-61-D/E) is the closest precedent: a single optional
MFPC section, parsed at publish, stored on the version row, `NULL` when absent.

### The storage precedent

`package_versions` (`repository/src/store.rs:438-450`) carries `description TEXT NULL`
at `:447`, added by `add_column_if_missing` at `:567` with the comment "NULL says
'not known' where '' would claim the publisher set it empty". It is written by the
version INSERT at `:1818-1828` and by an UPDATE at `:242-248`, and read back at
`:1458` and `:1707`.

### The backfill sweep already re-parses stored blobs

`repository/src/backfill.rs` (662 lines, 7 tests) exists precisely because
`author`/`url`/`description` were added after packages had been published. Its
`run` (`:58`) walks stored blobs via `BlobStore`, re-parses each, and fills what the
old publish path discarded. Its module doc (`:10-22`) states two rules any extension
must obey: one unparseable blob does not abandon the run, and a mismatch is skipped
and counted separately rather than silently resolved.

### Why not render lazily on first view

`BlobStore::get` returns `BlobFetch::Bytes` on the local backend but
`BlobFetch::Redirect(presigned)` on S3 (`repository/src/blobstore.rs:505-535`). A
page handler that reads the `.mfp` on demand would, in the production S3
configuration, have to fetch the registry's own blob back over HTTPS on an anonymous
unauthenticated route — plus cache invalidation and a thundering-herd path. Rejected.

### Measured populations

| What | Count | Command |
|---|---|---|
| Section-17 size, six documented packages | 9,027 – 27,951 B | MFPC section-table walk over each `.mfp` |
| Section 17 as a share of `.mfp` size, aggregate | 4.9% (91,792 B of 1,869,184 B) | sum of the six doc sections ÷ sum of the six file sizes |
| Section 17 share, smallest package (`libsnd`) | 40.0% | 15,440 ÷ 38,620 |
| Section 17 share, largest package (`jwt`) | 4.0% | 27,951 ÷ 701,834 |
| `backfill.rs` lines / tests | 662 / 7 | `wc -l repository/src/backfill.rs`; `grep -c '#\[test\]\|#\[tokio::test\]' …` |
| Existing `CREATE TABLE` statements in `store.rs` | 21 | `grep -c 'CREATE TABLE IF NOT EXISTS' repository/src/store.rs` |
| Repository crate lib tests | 351 | `grep -rc '#\[test\]\|#\[tokio::test\]' repository/src/*.rs repository/src/web/*.rs \| awk -F: '{s+=$2} END {print s}'` |

### Verified properties

- **Publishing already tolerates a section that fails to parse.** Read
  `repository/src/abi.rs:306-315` (`abi_index_json` → `{}` on error) and `:214-217`
  (`parse_package_description` → `Ok(None)` for a non-container payload). The
  publish path treats optional-section parse failure as "absent", never as a
  rejection. The doc section must follow the same posture.
- **`backfill.rs` reaches the blob store, not the network.** Read its imports
  (`:23-27`): `BlobFetch`, `BlobKind`, `BlobStore`, `package`, `PublishMetadata`,
  `Store`. Extending it needs no new dependency.
- **Three of nine `.mfp` files in the tree carry no section 17**
  (`examples/browser/{display,dom,fetch}`), so the absent-row case is the normal
  case for undocumented packages, not an edge case.

## 3. Design Overview

**Store the raw section-17 bytes, not a decoded structure.** `mfb_wire` is the single
decoder (plan-126-D); storing decoded rows would create a second representation that
must migrate whenever the wire format is extended. A `BLOB` costs one decode per page
render, on data already in the local database.

**A separate table, not a column on `package_versions`.** At 9–28 KB a `BLOB` column
would bloat every `SELECT` against a table read on the search, detail, index and
audit paths. A side table also makes "this version has no documentation" an absent
row rather than a NULL to interpret.

```sql
CREATE TABLE IF NOT EXISTS package_version_docs (
    package_version_id INTEGER PRIMARY KEY REFERENCES package_versions(id),
    doc_section BLOB NOT NULL
);
```

**Store per version; serve only the latest active.** Keeping the section for every
version is the design that stays correct when the newest release is yanked and the
selection falls back to an older one — with a per-version store that fallback is a
row lookup; with a latest-only store it is a blob fetch, reintroducing exactly the
S3 problem this sub-plan rejects. The cost is measured: section 17 is **4.9%** of
aggregate `.mfp` size across the six documented packages, and the registry already
retains every `.mfp` indefinitely. The serving surface (plan-126-F) still exposes
only the latest active version.

**Where correctness risk concentrates:** the publish transaction. The doc row must be
written in the same transaction as the version row, or a crash between them leaves a
version whose docs never appear and which no backfill run distinguishes from a
genuinely undocumented package. Land it inside the existing INSERT path
(`repository/src/store.rs:1818-1828`), not as a follow-up call.

**Byte-identity is not this sub-plan's gate** — nothing here touches codegen and
`scripts/artifact-gate.sh` will report `diffs=0` regardless. The gate is the
repository crate's tests plus `tests/cli/cli_repo_publish.rs`.

**Rejected alternative — extract on demand and cache.** See § Current State; the S3
redirect makes the server fetch its own blob over HTTPS.

**Rejected alternative — have the publisher upload a rendered doc artifact.** That
would put publisher-authored HTML into the registry, which the CSP design at
`repository/src/web/mod.rs:9-27` exists to prevent.

## Compatibility / Format Impact

- New table `package_version_docs`, created by `CREATE TABLE IF NOT EXISTS` on
  startup like the other 21. Existing databases gain it empty; `backfill-metadata`
  fills it.
- `BackfillReport` gains counters for docs filled and doc sections skipped. This
  changes `mfb-repo backfill-metadata`'s printed report (`render_text`,
  `repository/src/backfill.rs:153`).
- No HTTP route, response body, signed payload or `.mfp` format change.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` plus one line for partial; `- [x] ~~text~~ —
> moot: <evidence>` for moot. Fill `Commit:` the moment a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Schema and accessors, no writers

- [ ] Add the `package_version_docs` table to the schema batch in
      `repository/src/store.rs` (beside the other `CREATE TABLE IF NOT EXISTS`
      statements at `:431-533`), with a comment explaining why it is a side table
      and why the bytes are stored raw.
- [ ] Add `Store::put_version_docs(&self, package_version_id: i64, section: &[u8])`
      and `Store::latest_active_version_docs(&self, ident: &str) -> Result<Option<(String, Vec<u8>)>, String>`
      returning `(version, doc_section)` for the latest active version, built on
      `Store::latest_active_version` from plan-126-A.
- [ ] Tests in `repository/src/store.rs`: round-trip a section; a version with no
      docs row yields `None`; **a package whose newest version is yanked returns the
      older active version's docs**, not the yanked one's, and not `None`.

Acceptance: `rustup run 1.96.0 cargo test -p mfb_repository --no-fail-fast` passes,
including the yanked-fallback test — which is what proves the accessor is built on
the plan-126-A selection rather than on `ORDER BY created_at LIMIT 1`.
Commit: —

### Phase 2 — Capture at publish

- [ ] In the publish handler, call
      `mfb_wire::docs::read_package_doc_section(&package.payload)` alongside the
      existing four parses (`repository/src/server.rs:2861-2903`, `:3163-3198`),
      keeping the best-effort posture: a parse error means "no documentation", never
      a rejected publish.
- [ ] Write the row inside the same transaction as the version INSERT
      (`repository/src/store.rs:1818-1828`), not as a separate call afterwards.
- [ ] Tests in `repository/src/server.rs`: publishing a payload with a valid section
      17 stores it and it reads back byte-identical; publishing a payload **without**
      section 17 stores no row and still succeeds; publishing a payload whose section
      17 is truncated still succeeds with no row stored (the negative that proves
      documentation cannot break a publish).

Acceptance: `rustup run 1.96.0 cargo test -p mfb_repository --no-fail-fast` passes and
`tests/cli/cli_repo_publish.rs` is green. The truncated-section test is the important
one: it proves a malformed doc table cannot reject a signed package.
Commit: —

### Phase 3 — Backfill (largest blast radius: touches every stored blob)

- [ ] Extend `backfill::run` (`repository/src/backfill.rs:58`) to decode section 17
      from each re-parsed blob and fill `package_version_docs` where the row is
      absent, leaving existing rows alone so the sweep stays idempotent.
- [ ] Obey the module's two stated rules (`:10-22`): a blob whose doc section does
      not parse is **skipped and counted separately**, never silently treated as
      absent — an unparseable doc section in a stored, signed blob is a finding an
      operator must see.
- [ ] Add the counters to `BackfillReport` (`:29-48`) and to `render_text` (`:153`).
- [ ] Tests in `repository/src/backfill.rs`, following the shape of the existing 7:
      the sweep fills docs and is idempotent on a second run; a version whose blob
      carries no section 17 is quietly left alone (mirroring
      `backfill_fills_descriptions_and_stays_quiet_about_packages_that_have_none`
      at `:284`); a blob with a **malformed** section 17 is counted as skipped and
      its row is not written.

Acceptance: `rustup run 1.96.0 cargo test -p mfb_repository --no-fail-fast` passes;
running `mfb-repo backfill-metadata` twice against a datapath containing one
documented and one undocumented package reports the same filled count on the first
run and zero on the second, and its text report names the doc counters.
Commit: —

## Validation Plan

- **Tests:** `repository/src/store.rs` (round-trip, absent, yanked-fallback),
  `repository/src/server.rs` (present / absent / malformed at publish),
  `repository/src/backfill.rs` (fill, idempotent, quiet-when-absent, skip-malformed).
- **Coverage check:** the repository crate is a workspace member, so its 351 lib
  tests are in the `cargo test` denominator. `scripts/artifact-gate.sh` covers none
  of this — do not cite a 0-diff as evidence for this sub-plan.
- **Runtime proof:** against a local `mfb-repo` — publish `packages/jwt/jwt.mfp`
  (27,951 B section measured) and confirm `sqlite3 <datapath>/registry.db 'SELECT
  length(doc_section) FROM package_version_docs'` reports 27951 exactly. Then publish
  `examples/browser/dom/dom.mfp` (no section 17) and confirm no second row appears.
- **Storage sanity:** confirm the aggregate 4.9% figure holds on the test datapath by
  comparing `SUM(length(doc_section))` against the total blob bytes — if it is
  wildly higher, the per-version decision should be revisited in Corrections.
- **Doc sync:** `repository/DEPLOY.md` if it documents the backfill command's output;
  check with `grep -n backfill repository/DEPLOY.md`.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast`;
  `tests/cli/cli_repo_publish.rs`; `docker build -f repository/Dockerfile .`.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Per-version storage, or latest-active only?** Recommended **per version**, for
  the yank-fallback correctness reason in §3; the measured cost is 4.9% of aggregate
  `.mfp` bytes on data the registry already keeps forever. The alternative — keep
  only the latest active version's section and delete on each publish — halves an
  already-small number and reintroduces a blob fetch on the yank path. Note the
  measured counter-case: for the smallest package (`libsnd`, 38 KB) the doc section
  is 40% of the file, so the ratio is size-dependent and worth re-measuring on real
  registry data before treating 4.9% as general. (§3)
- **Should `GET /packages/:ident` gain a docs presence flag?** Recommended **no** here
  — it is plan-126-F's call, alongside the JSON parity route. Deciding it in this
  sub-plan would add a response field with no consumer. (§1)

## Corrections

<!-- Fill in during execution. Watch for: the real doc-section share on a populated
     datapath (the 4.9% is six packages in this tree, not a registry census), and
     whether the publish INSERT can genuinely carry a second table write in the same
     transaction without restructuring `store.rs:1818-1828`. -->

## Summary

Almost all of this is following a precedent that already exists four times over in
the publish handler and once in the backfill sweep. The two decisions that carry
real weight are storing raw bytes rather than a decoded shape — so `mfb_wire` stays
the only decoder — and writing the doc row inside the version's transaction, so a
crash cannot manufacture a package that looks undocumented forever. The
per-version-vs-latest-only tradeoff is recorded with its measurement rather than
assumed, because the ratio varies from 4% to 40% by package size.
