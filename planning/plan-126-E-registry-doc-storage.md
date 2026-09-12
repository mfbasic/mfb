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
| plan-126-D complete (`mfb_wire` decodes section 17) | `grep -c '^pub fn read_package_doc_section' wire/src/docs.rs` → 1 (corrected from `grep -c read_package_doc_section … → 1`) | MET (measured 2026-09-12: exactly **1** definition; D landed as 1d8a900e7 + 4c34b4f8a). The original command counts **6**, not 1, because the function's name also appears in its doc comment and in tests. The requirement (`mfb_wire` decodes section 17) holds; only the expected count was wrong, so the command now counts definitions. |

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

- [x] Add the `package_version_docs` table to the schema batch in
      `repository/src/store.rs` — placed beside its sibling
      `package_version_blobs`, inside the single `migrate()` `execute_batch`, so
      `CREATE TABLE IF NOT EXISTS` creates it on fresh and existing databases alike.
      `package_version_id INTEGER PRIMARY KEY REFERENCES package_versions(id)`,
      `doc_section BLOB NOT NULL`. The SQL comment explains the side table (a 9–28
      KB `BLOB` column would bloat every `SELECT` on a hot table; absence is an
      absent row, not a `NULL`), the raw storage (`mfb_wire::docs` stays the only
      decoder), and per-version retention (yank fallback by row lookup, not an S3
      self-fetch).
- [x] Add `Store::put_version_docs` and `Store::latest_active_version_docs`
      returning `(version, doc_section)`, **built on
      `Store::latest_active_version`** — it calls that function, then looks up the
      row by `(ident, version)`, rather than repeating "latest" as its own SQL
      predicate. `put_version_docs` uses `INSERT OR IGNORE`, so a version's docs are
      never rewritten, which is what keeps Phase 3's backfill idempotent.
      `latest_active_version` is called before this function takes the connection
      lock: the lock is not re-entrant, and holding it across that call would
      deadlock.
- [x] Tests in `repository/src/store.rs` — **five**, the three planned plus two:
      `version_docs_round_trip_byte_for_byte` (including `0x00` and `0xFF` bytes a
      text column would mangle); `a_version_with_no_docs_row_yields_none` (plus an
      unknown ident); `a_yanked_newest_release_falls_back_to_the_older_active_releases_docs`
      (the planned acceptance test); **added**
      `an_undocumented_latest_release_never_borrows_an_older_releases_docs`, pinning
      the contract plan-126-F's Docs tab depends on; **added**
      `putting_version_docs_twice_keeps_the_first_row`, pinning the `INSERT OR
      IGNORE` idempotency Phase 3 relies on.

Acceptance: MET. `rustup run 1.96.0 cargo test -p mfb_repository --lib
--no-fail-fast` → **379 passed; 0 failed** (374 + 5).
The yanked-fallback test is **load-bearing, measured rather than assumed**: with
`latest_active_version_docs` temporarily swapped for a naive newest-row query
(`ORDER BY pv.created_at DESC, pv.id DESC LIMIT 1` with a `LEFT JOIN` to the docs
table and no state predicate), exactly **one** test went red —
`a_yanked_newest_release_falls_back_to_the_older_active_releases_docs` (`test
result: FAILED. 4 passed; 1 failed`) — and the other four stayed green. That is
precisely the test that proves the accessor is built on the plan-126-A selection.
`store.rs` was then restored from its backup and `filecmp` asserted it
byte-identical.
Commit: —

### Phase 2 — Capture at publish

- [x] In the publish handler, capture section 17 alongside the existing parses,
      keeping the best-effort posture: a parse error means "no documentation",
      never a rejected publish. **Refined from the plan:** the handler calls
      `mfb_wire::mfpc::read_section_table` → section 17 → `read_doc_table`, rather
      than `read_package_doc_section`. That function returns *decoded*
      `PackageDocs`, but Phase 1 stores the **raw** section bytes. So the handler
      keeps the raw slice and uses the decode only as a gate: bytes that fail to
      decode are not stored, and are never a rejection. Both `PublishMetadata`
      match arms carry `docs` explicitly — left to `..Default::default()`, the
      no-MANIFEST arm would silently drop documentation the publisher signed.
- [x] Write the row inside the same transaction as the version INSERT, not as a
      separate call afterwards. Implemented by adding `docs: Option<Vec<u8>>` to
      `PublishMetadata` and writing `package_version_docs` right after
      `tx.last_insert_rowid()` inside `publish_package_version`'s transaction.
      **Chosen by measurement over a new positional argument:** that would have
      touched **57** `publish_package_version(` call sites; the field touched
      **3** struct literals (`cargo check` reported exactly three
      missing-field errors — `backfill.rs:135`, `server.rs:5478`, `server.rs:6410`)
      plus the insert. The struct already carries section-derived publisher
      metadata (section 18's `description`), so section 17 fits its purpose.
- [x] Tests in `repository/src/server.rs`, driving the real `publish_package`
      handler with a signed artifact:
      `publishing_a_documented_package_stores_its_doc_section_byte_for_byte`,
      `publishing_an_undocumented_package_succeeds_and_stores_no_doc_row`, and
      `a_truncated_doc_section_still_publishes_and_records_nothing`. The last
      first asserts its fixture genuinely fails to decode, so it cannot pass
      vacuously, and also asserts the version itself landed.

Acceptance: MET.
`rustup run 1.96.0 cargo test -p mfb_repository --lib --no-fail-fast` → **382
passed; 0 failed** (379 + 3).
`cargo test --test cli_repo_publish --test cli_repo_install --test cli_repo_auth
--test cli_repo_governance` against a freshly rebuilt release `mfb-repo` → **4 / 7 /
9 / 6 passed, 0 failed**.
**The truncated-section test is load-bearing, measured:** deleting the handler's
single decode-gate line (`mfb_wire::docs::read_doc_table(section).ok()?;` — 1
match before, 0 after) turned exactly that test red (`test result: FAILED. 2
passed; 1 failed`), while the documented and undocumented tests stayed green.
`server.rs` was then restored and `cmp` confirmed it byte-identical to the
post-format backup. That gate is what stops a malformed doc table from being
recorded.
`cargo check --all-targets` → only the three pre-existing `unused axum::Json`
warnings.
Commit: f9e26140a

### Phase 3 — Backfill (largest blast radius: touches every stored blob)

- [x] Extend `backfill::run` to decode section 17 from each re-parsed blob and fill
      `package_version_docs` where the row is absent. Existing rows are left alone
      via `put_version_docs`'s `INSERT OR IGNORE`, which now **returns whether it
      inserted**, so `docs_filled` counts only real inserts and a second run reports
      zero. The fill runs **after** `report.updated += 1`, so a mismatched or
      unparseable blob — which the sweep leaves untouched — never gains a docs row.
- [x] Obey the module's two stated rules. A section 17 that is present but does
      not decode is **counted separately** (`docs_unparseable`), logged as a skip
      line, and not recorded. It is also folded into `skipped()`, so the
      subcommand exits non-zero (`main.rs` exits 1 on `report.skipped()`). A
      payload with no section 17, or one that is not a container at all, stays
      quiet — the same posture the loop already takes for sections 10 and 18.
- [x] Add the counters to `BackfillReport` (`docs_filled`, `docs_unparseable`) and a
      separate `render_text` line, `doc sections: N recorded, M undecodable`. It
      says "undecodable" rather than "unparseable" so it can never be confused with
      the blob counter above; the existing `"1 mismatched"` assertion is untouched.
- [x] Tests in `repository/src/backfill.rs`:
      `backfill_fills_doc_sections_and_is_idempotent` (fills the documented blob,
      leaves the undocumented one quietly alone, a second run records nothing, and
      the doc line reads `1 recorded` then `0 recorded`), and
      `a_malformed_doc_section_is_counted_skipped_and_not_recorded` (counted,
      `skipped()` true, no row, while the version's other metadata still
      backfills; its fixture first asserts it genuinely fails to decode).
- [x] Added task: doc sync. The `backfill-metadata` usage text in
      `repository/src/main.rs` said it populates "the author, url and
      native-target columns"; it now names documentation records and the
      undecodable-doc-section finding. `repository/DEPLOY.md` does not document the
      backfill output (`grep -n backfill repository/DEPLOY.md` → nothing), so it
      needed no change.

Acceptance: MET.
`rustup run 1.96.0 cargo test -p mfb_repository --lib --no-fail-fast` → **384
passed; 0 failed** (382 + 2).
**The idempotency counter is load-bearing, measured:** with `put_version_docs`'s
`Ok(inserted == 1)` swapped for `Ok(true)` (1 match before, 0 after), exactly
**one** of the ten doc tests went red —
`backfill_fills_doc_sections_and_is_idempotent` — and `store.rs` was restored
`cmp`-identical.
**Runtime proof, on a genuine upgrade.** The datapath is `/tmp/p126-repoC`, whose
registry database was created by a server built **before** plan-126-E and held
`alice#p126pkg@0.1.0` (the undocumented `init-pkg` template) with **no
`package_version_docs` table at all**. Starting the post-E `mfb-repo` on it ran
`migrate()`, which created the table (0 rows). Publishing `alice#p126pkg@0.2.0` —
the same package with a `DOC` block, built by `mfb repo publish` — recorded a
**112-byte** docs row at publish, proving Phase 2 on a real build. With the server
stopped, that row was deleted (`rows deleted: 1`) to recreate a pre-Phase-2
publish. Then `mfb-repo backfill-metadata`:
  run 1 → `doc sections: 1 recorded, 0 undecodable`, **exit 0**;
  run 2 → `doc sections: 0 recorded, 0 undecodable`, **exit 0**.
Final state: 0.2.0's row is back at exactly **112** bytes, byte count matching
what the server wrote, and 0.1.0 still has none.
Commit: 1940b4dd4

## Validation Plan

- **Tests:** DONE — **10**, not the planned 9. `repository/src/store.rs` has 5
  (round-trip, absent, yanked-fallback, plus never-borrow-older-docs and
  first-write-wins); `repository/src/server.rs` 3 (present / absent / truncated at
  publish); `repository/src/backfill.rs` 2 (fill + quiet-when-absent + idempotent
  in one, malformed-is-counted in the other). Three of them were shown
  load-bearing by mutation: the yanked-fallback selection, the publish decode gate,
  and the backfill's inserted-count.
- **Coverage check:** DONE. The repository crate is a workspace member, so its lib
  tests are in the `cargo test` denominator — **384** after this sub-plan (374
  before; the plan said 351). `scripts/artifact-gate.sh` covers none of this code
  and no 0-diff from it is cited as evidence here.
- **Runtime proof:** DONE, against a live `mfb-repo` whose database predates
  plan-126-E. The real `jwt` package, published from a `/tmp` copy of the committed
  `packages/jwt` source, stored a doc section of **27,951 B**. That is exactly the
  plan's measured figure, and **byte-identical** to section 17 extracted from the
  published blob itself (compared as bytes, not just lengths). The database file is
  `meta.db`, not `registry.db` (Corrections). The undocumented case was proven by
  `alice#p126pkg@0.1.0`, which has no row, because `dom.mfp` is not committed
  (Corrections). The backfill half is in Phase 3's acceptance: `1 recorded`, then
  `0 recorded`, both exit 0.
- **Storage sanity:** DONE. On that datapath: 28,063 doc bytes of 707,247 blob
  bytes = **4.0%**, and jwt alone 4.0% — not "wildly higher" than the plan's 4.9%
  aggregate, so the per-version decision stands (Open Decisions).
- **Doc sync:** DONE. `grep -n backfill repository/DEPLOY.md` → nothing; DEPLOY.md
  does not document the backfill output. The `backfill-metadata` usage text in
  `repository/src/main.rs` was the stale description and is updated (Phase 3).
- **Acceptance:** DONE, apart from the plan-wide final gate.
  `cargo test --test cli_repo_publish --test cli_repo_install --test cli_repo_auth
  --test cli_repo_governance` → 4 / 7 / 9 / 6 passed (Phase 2).
  `docker build --load -f repository/Dockerfile -t mfb-repo-p126:e .` → **exit 0**,
  compiling `mfb_wire v0.1.0 (/build/wire)` then `mfb_repository`.
  The whole-workspace `rustup run 1.96.0 cargo test --no-fail-fast` is follow-plan
  §5's final gate, run once for all letters.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Per-version storage, or latest-active only?** **RESOLVED: per version**, taking
  the recommendation. It was re-measured on real registry data rather than assumed
  from the tree census. On the runtime-proof datapath, the real `jwt` package's
  stored doc section is 27,951 B of a 703,235 B blob = **4.0%** (the plan's own jwt
  figure, exactly), and the whole datapath is 28,063 doc bytes of 707,247 blob
  bytes = **4.0%**. That is not "wildly higher", so the per-version decision stands.
  The counter-case caveat still applies: the share is size-dependent (the plan
  measured `libsnd` at 40%), so a registry dominated by tiny packages would show a
  larger ratio of a smaller absolute number. (§3)
- **Should `GET /packages/:ident` gain a docs presence flag?** **RESOLVED: no, not
  here**, taking the recommendation — it is plan-126-F's call, alongside its JSON
  parity route. No response field was added by this sub-plan. (§1)

## Corrections

- **The publish INSERT carried the second table write without restructuring —
  answering this section's own watch item.** The doc row is written inside
  `publish_package_version`'s existing transaction, right after
  `tx.last_insert_rowid()`. The only structural choice was how the bytes reach it:
  a new positional argument would have touched **57** call sites
  (`grep -rn "publish_package_version(" --include='*.rs' repository/src src tests`),
  a `docs` field on `PublishMetadata` touched the **3** literals `cargo check`
  reported. The field won.

- **The real doc-section share, measured on a populated datapath** — the other
  watch item: **4.0%** (see Open Decisions), consistent with the plan, so no
  revision.

- **The runtime-proof command queries a file that does not exist.** § Validation
  Plan says `sqlite3 <datapath>/registry.db`. `mfb-repo` takes its database path
  from `--dbpath`, and the registry here used `meta.db` (the name its own tests and
  the plan-126-A/C runtime proofs use). There is no `registry.db`. Queried
  `meta.db`.

- **Neither runtime-proof fixture is committed, so both were substituted.**
  - `packages/jwt/jwt.mfp` is a gitignored build artifact (plan-126-B § Verified
    properties). jwt was published from a `/tmp` copy of the **committed**
    `packages/jwt` source with `"ident": "alice#jwt"` added, since `mfb repo publish`
    requires an ident. The committed source was not modified.
  - `examples/browser/dom/dom.mfp` is not committed either
    (`git ls-files 'examples/browser/*/*.mfp'` → nothing). The property it was to
    prove — a package with no section 17 gets no row — was proven instead by
    `alice#p126pkg@0.1.0`, the `init-pkg` template (no `DOC` blocks), which has no
    row after both publish and backfill.

- **Phase 2 captures section 17 without calling `read_package_doc_section`.** That
  function returns *decoded* `PackageDocs`, but Phase 1 stores the **raw** bytes. So
  the handler calls `mfpc::read_section_table` → section 17 and keeps the raw
  slice, using `read_doc_table` only as a gate: bytes that fail to decode are not
  stored. `read_package_doc_section` remains the one-call path the plan's §1
  describes, for a decoded view.

- **Phase 3 changed Phase 1's `put_version_docs` API** from `Result<(), String>` to
  `Result<bool, String>`. The backfill needs to know whether a row was actually
  inserted, so a second run reports zero instead of re-claiming rows. The five
  Phase 1 tests only `.unwrap()` it and needed no edit.

- **Populations re-measured 2026-09-12** (plan figures in parentheses): repository
  crate lib tests at the start of this sub-plan **374** (351); after it **384**.

## Summary

Almost all of this is following a precedent that already exists four times over in
the publish handler and once in the backfill sweep. The two decisions that carry
real weight are storing raw bytes rather than a decoded shape — so `mfb_wire` stays
the only decoder — and writing the doc row inside the version's transaction, so a
crash cannot manufacture a package that looks undocumented forever. The
per-version-vs-latest-only tradeoff is recorded with its measurement rather than
assumed, because the ratio varies from 4% to 40% by package size.
