# plan-49: repository blob garbage collection

Last updated: 2026-07-16
Effort: large (4h–1d)
Depends on: plan-48-A (the `package_version_blobs` reachability table and the
`PUT /blob` orphan case this collects)

The `mfb-repo` registry has **never deleted a package blob**. That was a
defensible non-decision when a blob was a `.mfp` of compiled IR — small,
one per version, always referenced. plan-48 breaks all three properties at once:
vendored native libraries are orders of magnitude larger, there are up to seven
per binding (one per target slot, plan-46-B §4.2), and `PUT /blob` can leave
blobs that **nothing ever references** when a publish is abandoned between the
upload and the commit.

This plan gives the registry a way to reclaim them: mark reachable blobs from
live package versions, sweep the rest after a grace period, and expose it as an
operator command rather than an automatic background process.

The single behavioral outcome: `mfb-repo gc` reports exactly which blobs are
unreachable and how many bytes they hold, and with `--delete` removes them —
while a blob referenced by any live version, or younger than the grace period,
is never touched.

References (read first):

- `repository/src/store.rs` — `package_blobs` (`:235-239`), `package_versions`
  (`:224-233`), the `INSERT OR IGNORE` blob row (`:1112`), `reap_expired`
  (`:1754-1771`) — the existing reaper, and the trap described in §2.2.
- `repository/src/blobstore.rs` — `BlobStore` enum (`:143`), `exists` (`:172`),
  `abort` (`:219`, today's *only* deletion), `blob_ref` (`:163`), S3 impl
  (`:251+`).
- `repository/src/main.rs` — `parse_args` and the existing operator subcommands
  (`reanchor` is the model for a new one).
- `plan-48-A` §2.4 (why there is no GC), §4.3 (the orphan case), §4.5
  (`package_version_blobs`, the reachability table this plan consumes).

## 1. Goal

- `mfb-repo gc` (dry run by default) lists every unreachable blob with its hash,
  size, age, and backing location, plus a total.
- `mfb-repo gc --delete` removes them from the blob store and the
  `package_blobs` table, in that order, and reports what it freed.
- A blob referenced by **any** live `package_versions` row is never deleted, even
  if the version is yanked (§3.2).
- A blob younger than a grace period is never deleted, regardless of
  reachability — this is what makes the sweep safe against an in-flight publish
  (§3.1).
- Running `gc` concurrently with a publish cannot delete that publish's blobs.

### Non-goals (explicit constraints)

- **Not automatic.** No background sweep, no `reap_expired` integration. Deleting
  package content is irreversible and operator-triggered; see §3.3.
- No refcounting. A refcount column would have to be maintained transactionally
  on every publish and would drift on any bug; reachability is recomputed from
  truth each run (§3.4).
- No change to the publish path, the blob API, the trust model, or any client
  behavior. `gc` is purely an operator-side reclaim.
- No deletion of anything but package blobs. `reap_expired`'s existing scope
  (auth challenges, sessions, pairing blobs) is untouched.
- No quota or admission-control change. `MAX_VERSIONS_PER_OWNER` /
  `PUBLISH_PER_OWNER_MAX` (`server.rs:607-611`) stay as they are.

## 2. Current State

### 2.1 Nothing deletes a blob

`package_blobs` (`store.rs:235-239`) is `hash TEXT PRIMARY KEY, path TEXT NOT
NULL, created_at INTEGER NOT NULL`. **No refcount, no reverse index, no kind.**
The insert is `INSERT OR IGNORE` (`store.rs:1112`), so N versions sharing a
content hash collapse to one row with nothing recording N.

The only blob deletion anywhere in the codebase is `abort()` on a failed publish
(`blobstore.rs:219-231`). Growth is bounded only *admissionally*, by
`MAX_VERSIONS_PER_OWNER = 10_000` and `PUBLISH_PER_OWNER_MAX = 30`/60s
(`server.rs:607-611`). Blobs are permanent by design.

### 2.2 `reap_expired` is not what it looks like

`reap_expired` (`store.rs:1754-1771`) deletes `auth_challenges`, revokes
`sessions`, and deletes **`pairing_blobs`**. That last one is easy to misread —
and the reaper comment at `server.rs:624-625` invites the misreading. `pairing_blobs`
are **machine-pairing auth ephemera**, not package content. Nothing in the reaper
touches `package_blobs`.

Do not extend `reap_expired` for this. It runs automatically; package GC must
not (§3.3).

### 2.3 What plan-48 changes

- **Size:** a `.mfp` is compiled IR. A vendored native library is a shared object
  — routinely 1-2 MB, potentially up to plan-48-A's 64 MiB body cap.
- **Count:** up to 7 vendor blobs per binding version (plan-46-B §4.2's target
  matrix), versus one `.mfp`.
- **Reachability:** the new one. Every blob today is referenced by the
  `package_versions` row created in the same transaction. plan-48-A §4.3's
  `PUT /blob` accepts a blob **before** any version references it, so a publisher
  who uploads and then abandons (network failure, failed validation, `^C`) leaves
  a blob nothing will ever name. plan-48-A bounds this with a rate limit and adds
  `package_version_blobs` (§4.5) so reachability is computable — but explicitly
  leaves the collecting to this plan.

### 2.4 The reachability data exists

plan-48-A §4.5 adds, written inside the publish transaction:

```sql
CREATE TABLE package_version_blobs (
  package_version_id INTEGER NOT NULL REFERENCES package_versions(id),
  hash TEXT NOT NULL REFERENCES package_blobs(hash),
  PRIMARY KEY (package_version_id, hash)
);
```

Nothing reads it there. **This plan is its first consumer** — which is the whole
reason plan-48-A adds it up front rather than leaving the edges unrecorded for
every package published before GC exists.

## 3. Design Overview

Mark-and-sweep, run on demand:

1. **Mark** — the reachable set is `package_versions.hash` ∪
   `package_version_blobs.hash` over live versions.
2. **Sweep** — every `package_blobs` row not in that set, and older than the
   grace period, is a candidate.
3. **Report or delete** — dry run by default; `--delete` removes from the blob
   store then the DB.

### 3.1 The grace period is the concurrency design

There is no lock between `PUT /blob` and `POST /publish`. A publisher can upload
five blobs, and a `gc` running one second later would see all five as unreachable
— because they *are*, until the publish lands. Deleting them would break a
publish in flight, and the client would get a bewildering "blob missing" from
plan-48-A §4.4's own referential check.

A **grace period** (default: 24h, on `package_blobs.created_at`) makes this safe
without any locking: a blob is only ever collected long after any plausible
publish has finished or died. The cost is that genuine garbage lingers for a day,
which is exactly the right trade — storage is cheap, deleting a live publisher's
blob is not.

This is why the design does **not** need a lock, a lease, or a two-phase
protocol. Prefer the boring mechanism.

### 3.2 Yanked versions keep their blobs

`release_state_changes` (`store.rs:241-245`) can move a version to a yanked
state. A yanked version is **still reachable** and its blobs must survive: yanking
is a "do not resolve this by default" signal, not a deletion, and existing
lockfiles pin the hash and must keep installing. Only a version row that no
longer exists releases its blobs.

If version *deletion* is ever added, it becomes this plan's real client — and the
grace period covers it for free.

### 3.3 Operator-triggered, not automatic

`gc` is a subcommand (`mfb-repo gc`), modeled on the existing `reanchor` operator
command, not a hook in `reap_expired`.

Deleting package content is irreversible and, unlike expiring a session, has no
"it will be re-created" fallback. A registry operator should decide when to run
it, see what it would do first, and be able to run the dry form freely. An
automatic sweeper that is wrong once destroys artifacts that a lockfile somewhere
still pins. Dry-run-by-default with an explicit `--delete` is the same shape as
the rest of the destructive-operation surface.

### 3.4 Recompute reachability; do not refcount

Rejected alternative: a `refcount` column on `package_blobs`, incremented on
publish and decremented on version removal. Rejected — it must be maintained
transactionally at every mutation site, it drifts permanently on any bug or crash
between the two writes, and a drifted refcount either leaks forever (harmless but
undetectable) or **deletes a live blob** (catastrophic and silent). Mark-and-sweep
recomputes from the truth on every run, so it is self-correcting: a bug in one
run is fixed by the next.

The cost is a full scan of `package_blobs` per run. At `MAX_VERSIONS_PER_OWNER =
10_000` that is trivial, and this is an operator command, not a request path.

## 4. Detailed Design

### 4.1 The reachable set

```sql
SELECT hash FROM package_versions
UNION
SELECT hash FROM package_version_blobs;
```

Both halves are required and neither is redundant:
`package_versions.hash` is the `.mfp` itself; `package_version_blobs.hash` is its
vendor blobs. A `.mfp` blob is *not* in `package_version_blobs` (it is not a
vendor blob), so omitting the first half would delete every package.

Write that sentence into a test, not just a comment.

### 4.2 Candidate selection

```sql
SELECT hash, path, created_at FROM package_blobs
WHERE hash NOT IN (<reachable set>)
  AND created_at < :now - :grace_seconds
ORDER BY created_at;
```

`created_at` is already on the row (`store.rs:237`) — no schema change is needed
for the grace period.

### 4.3 Sizes for the report

`package_blobs` has no size column. Two options:

- **Stat the backing store** per candidate (`fs::metadata` locally, `head_object`
  on S3). Accurate; costs one call per candidate, which is fine for an operator
  command and lets the report show real reclaimed bytes.
- **Add a `size` column** written at stage time. Free at report time, but needs a
  schema change and back-fills as unknown for every existing row.

Recommend stat-on-demand: no migration, no back-fill hole, and the cost lands on
a command that already runs rarely.

### 4.4 Delete order: store first, then DB

For each candidate, delete from the blob store, then delete the `package_blobs`
row. This ordering matters and is the inverse of the publish path's
(stage → row → promote, `server.rs:1806-1841`):

- **Store then DB:** a crash in between leaves a DB row pointing at a missing
  object. The next `gc` re-lists it as a candidate and the delete is idempotent;
  a `GET` for it 404s — which is already the correct answer, since it was
  unreachable.
- **DB then store:** a crash in between leaves an object with no row —
  invisible to every future `gc`, unreclaimable forever. Strictly worse.

Neither is atomic and neither needs to be; pick the failure that is
self-healing.

Deletion must tolerate an already-absent object (the local backend's `abort`
already does, `blobstore.rs:219-231`) — treat "not found" as success.

### 4.5 The `gc` subcommand

```
mfb-repo gc --dbpath <db> --datapath <data> [--grace-hours N] [--delete] [--json]
```

- Default: dry run. Prints each unreachable blob (hash, size, age, `blob_ref`)
  and a total count/bytes, then exactly what `--delete` would reclaim.
- `--delete`: performs §4.4 and reports what was freed.
- `--grace-hours`: default 24 (§3.1). A value of 0 must be **refused** — it
  removes the only concurrency protection the design has (§3.1). If an operator
  genuinely needs an immediate sweep of a quiesced registry, that is a distinct,
  explicitly-named flag, not a `0`.
- `--json`: machine-readable, for operators who script it.

Follow `parse_args` (`main.rs`) and the `reanchor` subcommand's shape. Note
`--datapath` is only the blob directory; metadata is always local (`store.rs:65-121`),
and an `s3://` datapath skips local dir creation — `gc` must work in both modes.

## Compatibility / Format Impact

- **Additive and operator-only.** One new subcommand. No schema change (§4.2/§4.3
  use existing columns). No protocol, client, `.mfp`, or trust-model change.
- **A registry that never runs `gc` behaves exactly as today** — which is the
  correct default for an existing deployment.
- Deleting a blob is visible to clients only as a `GET /blob/<hash>` 404, and only
  for hashes that no live version references.

## Phases

### Phase 1 — reachability + dry-run report

The whole value of the plan without any destructive operation. Independently
landable and independently useful: an operator can see the problem before
anything can delete.

- [x] Add the reachable-set query (§4.1) and candidate selection (§4.2) to
      `repository/src/store.rs`. → `unreachable_blobs` / `reachable_blobs` /
      `forget_blob`, over a new `BlobRow`. A negative or overflowing grace period
      is refused rather than wrapping into the future.
- [x] Add `mfb-repo gc` (dry run only) per §4.5, with size stat (§4.3), for both
      the local and S3 backends. → new `repository/src/gc.rs` (`run` +
      `render_text`/`render_json`), `BlobStore::size` (`fs::metadata` /
      `head_object`), and `parse_gc_args` in `main.rs`.
- [x] Tests: a blob referenced by a live version is **never** a candidate; a
      `.mfp` blob is never a candidate (§4.1 — the query-half regression); a
      blob referenced only by a **yanked** version is never a candidate (§3.2); an
      unreferenced blob **inside** the grace window is not a candidate; the same
      blob **outside** it is; a blob shared by two versions is not a candidate
      when only one is removed. → `store.rs` tests
      `a_published_mfp_blob_is_never_a_candidate`,
      `candidates_are_unreferenced_blobs_past_the_grace_period`,
      `a_yanked_versions_blobs_stay_reachable`,
      `a_shared_blob_survives_removing_one_of_its_versions`, plus
      `gc::tests::grace_window_shields_a_fresh_orphan`. Both guards were
      mutation-checked: dropping the `package_versions` half of the union fails
      6 tests, and defeating the grace filter fails the window test.

Acceptance: on a registry with a deliberately orphaned blob (upload via
`PUT /blob`, never publish), `mfb-repo gc` lists exactly that blob and nothing
else, and lists nothing at all before the grace period elapses. **MET** —
`tests/repo_acceptance.rs::repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable`
does exactly this against a live `mfb-repo`, and it was reproduced by hand in
both local-datapath and MinIO S3 modes.
Commit: (this change)

### Phase 2 — `--delete`

- [x] Implement §4.4 (store then DB, idempotent on missing objects) behind
      `--delete`; refuse `--grace-hours 0` (§4.5). → `BlobStore::delete` treats
      "not found" as success on both backends; `--grace-hours 0`/negative/
      overflowing is refused at the argument boundary, before the store opens.
- [x] Tests: `--delete` removes exactly the Phase-1 candidate set and frees the
      backing bytes; a `GET` for a deleted hash 404s; `gc --delete` run twice is
      a no-op the second time; an interrupted delete (row present, object gone)
      is re-collected cleanly by the next run (§4.4); every reachable blob still
      downloads afterward. → `gc::tests::{delete_reclaims_the_orphan_and_spares_live_blobs,
      interrupted_delete_is_recollected}`, the acceptance test's 404 + second-sweep
      assertions, and `s3_backend.rs::s3_gc_reclaims_only_the_orphan` against a
      real bucket.
- [x] Doc: extend the operator documentation and
      `src/docs/spec/package-manager/01_repository-protocol.md`'s server section
      with the `gc` subcommand, the grace period's role (§3.1), and the explicit
      statement that yanked versions retain their blobs (§3.2). → new spec
      section "Blob garbage collection", a "Reclaiming abandoned uploads" section
      in `repository/DEPLOY.md`, and corrections to the two spec claims that said
      the registry never collects and that `package_version_blobs` has no reader.

Acceptance: an orphaned blob is reclaimed and every live package still installs
and builds — verified by a real `pkg add` against the swept registry, not just
unit tests. **MET** — the acceptance test re-adds and rebuilds both published
packages after the sweep, and asserts the surviving vendor blob still serves its
exact bytes; the same was done by hand against an S3-backed registry.
Commit: (this change)

## Outcome

Landed as designed, with two deviations worth recording:

- **Reachable-byte accounting is `--json`-only.** The plan's open decision
  recommended "yes, in `--json` at minimum". It is *only* there, because the
  totals cost one `head_object` per **reachable** blob — on a large registry that
  dwarfs the candidate scan, to answer a question the operator did not ask. The
  text report states the reachable *count*, which is free.
- **`repository/tests/s3_backend.rs` had to be repaired first.** It had not
  compiled since plan-48 added `BlobKind`, and its final assertion ("an aborted
  blob should be gone") had been made false by bug-276 R4, which deliberately
  stopped S3 `abort` from deleting — the test could not catch the change because
  it no longer built. It now asserts the documented behavior and covers `gc`
  against a live bucket. The object bug-276 R4 knowingly leaves behind is exactly
  what this plan reclaims.

Version deletion remains out of scope, and §3.2's "only a removed version
releases its blobs" is proven by a test that removes one of two versions sharing
a vendor blob.

## Validation Plan

- Tests: per phase; the ones that matter most are the **negative** ones — a live
  blob must never be collected. Weight them accordingly.
- Runtime proof: run against a real `mfb-repo` in both local-datapath and MinIO
  S3 modes (`--s3-endpoint`): publish two packages, orphan a blob via `PUT` with
  no publish, age the row, `gc --delete`, then confirm both packages still
  `pkg add` + build while the orphan is gone.
- Doc sync: the repository-protocol spec's server section; `.ai/specifications.md`
  obligation.
- Acceptance: `cargo test -p mfb_repository` green including `--features s3`;
  `scripts/test-accept.sh` green (no client-facing change is expected — that is
  itself the assertion).

## Open Decisions

- **Grace period default.** 24h is proposed as comfortably longer than any
  plausible publish. A registry with slow, huge uploads might want 72h.
  Recommend: 24h default, `--grace-hours` to override, `0` refused (§4.5).
- **Whether `gc` should also report *reachable* bytes**, so an operator can see
  total store size and what fraction is garbage. Cheap (the scan is already
  happening) and likely the first thing anyone asks after "what can I delete?".
  Recommend: yes, in `--json` at minimum.
- **Version deletion** is the obvious follow-on: `gc` is the mechanism that would
  make it actually reclaim space, and §3.2's "only a removed version releases its
  blobs" is written to accommodate it. Out of scope here — it is a policy and
  trust question (a deleted version breaks every lockfile pinning it), not a
  storage one.

## Summary

The registry's "blobs are permanent" non-decision was fine for small, always-
referenced `.mfp` files and stops being fine the moment plan-48 puts multi-megabyte
native libraries in the same store — with, for the first time, a way to create
blobs that nothing references.

The mechanism is deliberately boring: mark from truth, sweep with a grace period,
operator-triggered, dry-run by default. The grace period does the work a lock
would otherwise do (§3.1); recomputing beats refcounting because it is
self-correcting (§3.4); and deleting from the store before the DB picks the
self-healing failure over the unreclaimable one (§4.4).

The risk here is entirely one-sided: collecting a live blob is catastrophic and
silent, while failing to collect garbage costs disk. Every default in this plan
leans that way on purpose.
