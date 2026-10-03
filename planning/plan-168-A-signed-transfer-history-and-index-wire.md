# plan-168-A: Signed transfer history, publisher identity, and the `/index` transfer wire

Last updated: 2026-10-03
Overall Effort: x-large (1d–3d across A, B, C, D)
Effort: large (3h–1d)
Depends on: nothing

plan-168 fixes bug-792: after `alice#toolbox` moves from alice to bob, bob cannot
publish a release, because build, publish validation, proof/attestation
verification and the package index all tie identity to the ident PREFIX owner.
The fix must not let the registry's word alone grant a publish right that clients
see. A client accepts bob's releases only after it verifies a signed transfer
chain. The chain has four parts:

- alice's offer signature, under a key on alice's signed rotation chain;
- bob's identity, via the registry name binding (the same trust-on-first-use
  `mfb pkg add` already uses);
- bob's accept signature;
- bob's own rotation chain from there on.

Releases made before the transfer must keep verifying.

**This sub-plan** builds the server-side record the clients will verify:

- an append-only, signed transfer history;
- per-release publisher identity and transfer epoch;
- v2 transfer messages that cannot be replayed;
- the additive `/index` fields that serve the chain.

It also fixes two transfer defects found while planning: `mfb repo transfer`
always signs as the ident-prefix owner, and `/packages/<ident>` shows the current
owner's name next to the prefix owner's key.

Outcome: after `mfb repo transfer alice#toolbox bob` and
`mfb repo transfer-accept alice#toolbox@bob`, `GET /index/alice#toolbox` serves one
`transfers` link. Its `offerSignature` verifies under alice's ident key, and its
`acceptSignature` under bob's, over the v2 messages. Every `versions[]` entry
carries `publisher` and `epoch`. Publish authorization is unchanged here (B).

References:

- `bugs/bug-792-transferred-package-recipient-cannot-publish.md` (the defect, the reproduction probe, the root cause)
- `bugs/completed/bug-494-registry-release-state-signing-prefix-authz.md` (signing/release-state already use stored ownership)
- `src/docs/spec/package-manager/01_repository-protocol.md` (§Install Path `/index`, §Accounts — Transfers, current-implementation deviation paragraph)
- `src/docs/spec/package-manager/03_signing.md` (domain-separated message table)
- `src/docs/spec/tooling/07_cli-reference.md` (`repo transfer` rows)
- `.ai/specifications.md` (Hard Completion Gate), `.ai/net-tls.md` (client transport rules)

## Prerequisites

These are a precondition on all of plan-168. Sub-plans B, C and D point here.

| Must be true | Command | Status |
|---|---|---|
| bug-792 is still open (not already fixed) | `ls bugs/bug-792-*` → `bugs/bug-792-transferred-package-recipient-cannot-publish.md` | MET |
| The transfer protocol is still v1 (this plan replaces it) | `grep -c 'transfer-offer-v1' repository/src/crypto.rs` → `1` | MET |
| No other plan-168 sub-plan has started | `grep -l 'Commit: [0-9a-f]' planning/plan-168-*` → no output | MET |

Everything below assumes these hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the current status of *all*
> prerequisites.

## 1. Goal

- `GET /index/<owner>#<package>` for a transferred package serves a `transfers`
  array. Each signed link verifies under its from-owner's and to-owner's ident
  keys over the v2 offer/accept messages. Every version carries
  `publisher: {owner, identFingerprint}` and `epoch`.
- The v2 messages bind `(ident, sequence, fromOwner, fromIdentFingerprint, toOwner, releaseDigest)`.
  A recorded link cannot be replayed at another position in the chain.
- `transfer_history` cannot be updated or deleted. SQLite triggers abort the
  statement.
- `mfb repo transfer` signs as the package's **current** owner, so a second
  transfer (bob → carol) works from the CLI.

### Non-goals (explicit constraints)

- No change to who may publish, or to `/validate` and `/publish` diagnostics. That is B.
- No client-side chain verification (C) and no `signing_ident` change (D).
- The existing `IndexResponse` / `IndexVersion` JSON fields keep their names,
  types and meanings. `owner`, `identKey`, `identFingerprint` and
  `nameBindingSignature` still describe the ident-PREFIX owner (§Compatibility).
- No change to `.mfp` bytes, proofs, attestations, the name-binding message, or
  the transparency-log leaf format of existing kinds.
- No change to `index_canonical_hash` (signed snapshot). The spec already records
  that the client does not bind `/index` to it.
- No change to the `/packages/<ident>/audit` response shape.

## 2. Current State

- **Schema migration mechanism.** `repository/src/store.rs:migrate` runs one
  `CREATE TABLE IF NOT EXISTS` batch. It then adds columns idempotently with
  `repository/src/store.rs:add_column_if_missing`, which swallows only the
  "duplicate column" error. There is no `user_version`. Precedents: the
  plan-61-A `author/url/description` columns and bug-583 `approval_key`, both
  in `migrate`.
- **Transfers.** `store.rs:create_transfer_offer` UPSERTs one `transfer_offers`
  row per package (`UNIQUE(package_id)`, `ON CONFLICT(package_id) DO UPDATE`).
  `store.rs:accept_transfer` sets `accepted_at` and rewrites
  `packages.owner_id`. Neither takes a signature. The handlers
  `repository/src/server.rs:transfer_offer` / `transfer_accept` verify the
  ident signature over `crypto.rs:transfer_offer_message(ident, from, to)` /
  `transfer_accept_message(ident, to)`, then call the store with names only.
  The signatures are dropped. Log payloads are `{"ident","from","to"}` and
  `{"ident","to"}`.
- **Replayability of v1.** The v1 messages contain no sequence and no key
  fingerprint (verified by reading `crypto.rs:transfer_offer_message`). After
  alice→bob then bob→alice, a stored alice→bob pair is byte-identical to a third
  alice→bob transfer.
- **Index.** `server.rs:package_index` resolves `owner_with_ident_key(owner_part)`,
  the PREFIX owner. It signs `crypto.rs:name_binding_message(prefix_display, fp)`
  and serves `IndexVersion {version, hash, publishedAt, state, abiIndex, logEntry}`.
  `repository/src/client.rs:fetch_index_with_key` requires
  `fold_owner(response.owner) == fold_owner(requested prefix)` (bug-581 route
  binding), then verifies the binding.
- **Publish record.** `store.rs:publish_package_version` stores
  `(package_id, version, hash, state, abi_index, author, url, description)`.
  There is no publisher key and no epoch. The publisher is always the session
  owner (`owner_id` argument).
- **CLI transfer.** `src/cli/pkg.rs:transfer_offer` takes `from_owner` from the
  ident prefix. `client.rs:transfer_offer` then reads that owner's local ident
  key and session. A current owner who is not the prefix owner (bob, after
  alice→bob) cannot offer.
- **Package page.** `store.rs:package_detail` reads the owner display from
  `packages.owner_id` (current owner). `server.rs:package_detail` reads
  `identKey` from `owner_with_ident_key(owner_part)` (prefix owner). After a
  transfer the page pairs "bob" with alice's key.
- **Rotation chain primitive.** `client.rs:follow_ident_chain(owner, pinned, chain)`
  verifies `ident_rotation_message` links from `pinned` forward and returns the
  newest key. `server.rs:ident_chain` serves `GET /idents/<owner>`, which is not
  server-signed.

### Measured populations

| What | Count | Command |
|---|---|---|
| v1 message builder uses in server.rs (2 production + 10 in 4 tests) | 12 | `grep -c 'transfer_offer_message\|transfer_accept_message' repository/src/server.rs` |
| v1 message builder uses in client.rs (2 production + 2 in `transfer_offer_and_accept_are_each_ident_signed`) | 4 | `grep -c 'transfer_offer_message\|transfer_accept_message' repository/src/client.rs` |
| server.rs tests using v1 messages | 4 (`ownership_transfer_is_two_sided_and_rebinds_the_package`, `transfer_and_org_endpoints_reject_malformed_and_wrong_signatures`, `publish_aborts_the_staged_blob_when_the_ident_moved_owner`, `a_transferred_package_is_governed_by_its_current_owner`) | `grep -n 'async fn' repository/src/server.rs` read around each `transfer_offer_message` hit |
| store transfer API call sites (`create_transfer_offer(` / `accept_transfer(`) | server.rs 2, store.rs 17 | `grep -rn 'create_transfer_offer(\|accept_transfer(' repository/src src tests \| grep -v 'pub fn' \| awk -F: '{print $1}' \| sort \| uniq -c` |
| `publish_package_version(` call sites (why the publisher rides in `PublishMetadata`, not a new parameter) | 60 (store 36, server 15, backfill 8, gc 1) | `grep -rn 'publish_package_version(' repository/src src tests \| grep -v 'pub fn' \| awk -F: '{print $1}' \| sort \| uniq -c` |
| `IndexVersion {` struct literals | 6 (server.rs:1949, client.rs:2587, client.rs:2598 `..with_abi`, pkg.rs:2371, pkg.rs:2708, resolve.rs:1252), 5 need the new fields | `grep -n 'IndexVersion {' repository/src/*.rs src/cli/*.rs \| grep -v 'struct\|impl\|->'` |
| `IndexResponse {` struct literals | 3 (server.rs:1964, pkg.rs:2384, resolve.rs:1273) | `grep -n 'IndexResponse {' repository/src/*.rs src/cli/*.rs \| grep -v struct` |
| Legacy accepted transfers on the hosted registry | UNMEASURED (no access to the production DB) | Phase 1 first task: operator runs `sqlite3 <db> "SELECT COUNT(*) FROM transfer_offers WHERE accepted_at IS NOT NULL"` |

### Verified properties

- **Every pre-fix release was published by the ident-prefix owner.**
  `server.rs:validate_package_request` adds the diagnostic "session owner does
  not match package ident owner" unless `fold_owner(prefix) == fold_owner(session)`.
  `server.rs:publish_package` refuses any non-empty diagnostics.
  `store.rs:publish_package_version` resolves the package row
  `WHERE ident = ?1 AND owner_id = ?2` with the session owner. So the publisher
  of every existing row is the prefix owner, and the prefix owner also held
  `packages.owner_id` at publish time. This is how the backfill is derived.
- **Every pre-fix release has epoch 0, except after a transfer back.** A pre-fix
  recipient can never publish (the bug). A former owner cannot publish either,
  because `publish_package_version` needs the current `owner_id`. The one
  exception is a transfer back to the prefix owner (alice→bob→alice), after which
  alice publishes again. Legacy history keeps only the latest offer row per
  package (`UNIQUE(package_id)`), so the true epoch cannot be reconstructed. §4.4
  makes legacy links chain terminators, so no client check depends on a legacy
  epoch.
- **No production code deletes `package_versions`.**
  `grep -rn 'DELETE FROM package_versions' repository/src` hits only the test at
  `store.rs` (`a_shared_blob_survives_removing_one_of_its_versions`). The
  cumulative release digest therefore never shrinks.
- **The `IndexResponse` / `IndexVersion` derives have no `deny_unknown_fields`**
  (read `server.rs` struct definitions). Older clients ignore added fields.
- **`PublishMetadata` is constructed with `..Default::default()`** in
  `server.rs:publish_package`, so a new defaulted field changes no call site.

## 3. Design Overview (the whole of plan-168; B–D refer here)

The pieces, in landing order:

1. **(A) Signed record.** v2 messages, `transfer_history` (append-only), offer
   terms on `transfer_offers`, and publisher plus epoch per version. `/index`
   serves them, and the CLI signs v2.
2. **(B) Server authorization.** Publish, validate and token scope follow the
   stored owner. `verify_proof` / `verify_attestation` bind the signed `owner`
   to the release `author` instead of the prefix.
3. **(C) Client verification.** One pure verifier in
   `repository/src/trust.rs` walks pin → rotations → signed links → publisher.
   `pkg add` / `pkg verify` / resolve / install / `mfb info` use it, and the
   `project.json` pin and lock key become "the verified identKey of the
   installed release".
4. **(D) CLI build relaxation + end to end.** `signing_ident` accepts any
   well-formed ident (the registry's ownership check gates the attestation).
   D adds the full CLI scenario and removes the spec's deviation paragraph.

**Correctness risk is concentrated in C**, the verifier: a wrong rule there either
re-strands releases or accepts a forged owner. It lands after the record it
verifies exists (A) and after publish works (B), behind adversarial unit tests.
**Design uncertainty is concentrated in A**: whether the v2 terms can be computed
by both sides from the index alone. A's Phase 2 proves that first, with the real
CLI signer against the real server.

**Byte-identity is not a gate for any plan-168 letter.** No codegen changes. The
gates are repository-crate tests and the `cli_repo_*` integration tests.

### Rejected alternatives

- **Serve the recipient's key as the index `identKey`.** This breaks
  verification of pre-transfer releases (bug-792 Fix Design), and older
  clients' `fetch_index_with_key` owner check (`fold(owner) == fold(prefix)`)
  would reject every transferred index.
- **Let clients trust `packages.owner_id`** (a server-asserted current owner).
  This breaks the trust rule: a compromised registry could grant anyone publish
  rights.
- **Keep v1 messages and just store the signatures.** v1 links are replayable at
  a later chain position (§2).
- **A new "transfer terms" endpoint.** Not needed: the sequence is
  `transfers.len()` and the digest is computed from `versions[]`, both already
  served by `/index` after this letter.
- **A new `publish_package_version` parameter for the publisher key.** It would
  touch 60 call sites. A defaulted `PublishMetadata` field touches none.
- **Backfill legacy publisher keys from stored blobs** (extending
  `backfill-metadata`). Clients verify the downloaded header `identKey`, never
  the index fingerprint (C), so the index value is informational. Legacy rows
  serve `identFingerprint: null`.
- **Rename the ident on transfer.** This would break every dependent's
  `packages[]` entry and lock (bug-792 Fix Design forbids it).

## 4. Detailed Design

### 4.1 v2 messages (`repository/src/crypto.rs`)

```text
transfer_offer_message_v2  = "mfb-repo-transfer-offer-v2\0"  || ident || "\0" || seq || "\0"
                             || fromOwner || "\0" || fromIdentFingerprint || "\0" || toOwner || "\0" || releaseDigest
transfer_accept_message_v2 = "mfb-repo-transfer-accept-v2\0" || (same field list)
release_digest(versions)   = hex(SHA-256( concat over (version, hash) sorted by (version, hash) of
                             version || "\0" || hash || "\n" ))
```

- `seq` is the decimal ASCII count of accepted transfers before this one.
- Owner names are the registered **display** forms (`owner_display`), not the
  typed form. The client reads them from `/index`, which name-binds display forms.
- `releaseDigest` covers **every** version of the ident at offer time
  (cumulative, all states).
- Delete `transfer_offer_message` / `transfer_accept_message` (v1). The server
  stops accepting them (Open Decision A-D3). Their test uses migrate to v2 with
  unchanged assertions.

### 4.2 Schema (`store.rs:migrate`)

New table in the batch:

```sql
CREATE TABLE IF NOT EXISTS transfer_history (
  id INTEGER PRIMARY KEY,
  package_id INTEGER NOT NULL REFERENCES packages(id),
  sequence INTEGER NOT NULL,
  from_owner_id INTEGER NOT NULL REFERENCES owners(id),
  from_ident_key_id INTEGER NULL REFERENCES keys(id),   -- NULL only for legacy rows
  to_owner_id INTEGER NOT NULL REFERENCES owners(id),
  to_ident_key_id INTEGER NULL REFERENCES keys(id),     -- NULL only for legacy rows
  release_digest TEXT NULL,                             -- NULL only for legacy rows
  offer_signature BLOB NULL,                            -- NULL only for legacy rows
  accept_signature BLOB NULL,                           -- NULL only for legacy rows
  offered_at INTEGER NOT NULL,
  accepted_at INTEGER NOT NULL,
  UNIQUE(package_id, sequence)
);
CREATE TRIGGER IF NOT EXISTS transfer_history_no_update BEFORE UPDATE ON transfer_history
  BEGIN SELECT RAISE(ABORT, 'transfer_history is append-only'); END;
CREATE TRIGGER IF NOT EXISTS transfer_history_no_delete BEFORE DELETE ON transfer_history
  BEGIN SELECT RAISE(ABORT, 'transfer_history is append-only'); END;
```

Columns added with `add_column_if_missing`:

- `transfer_offers`: `sequence INTEGER NULL`, `from_ident_key_id INTEGER NULL`,
  `release_digest TEXT NULL`, `offer_signature BLOB NULL`. NULL means a legacy
  pending offer, which cannot be accepted (§4.3).
- `package_versions`: `publisher_owner_id INTEGER NULL`,
  `publisher_ident_key_id INTEGER NULL`, `transfer_epoch INTEGER NOT NULL DEFAULT 0`.

Idempotent backfill statements, run after the column additions:

- `UPDATE package_versions SET publisher_owner_id = (SELECT o.id FROM owners o JOIN packages p ON p.id = package_versions.package_id WHERE o.owner_folded = lower(substr(p.ident, 1, instr(p.ident, '#') - 1))) WHERE publisher_owner_id IS NULL`.
  SQLite `lower()` is ASCII-only, which equals `fold_owner` (`to_ascii_lowercase`)
  because owner names are ASCII (`validate_owner_name`).
- `INSERT INTO transfer_history (package_id, sequence, from_owner_id, to_owner_id, offered_at, accepted_at) SELECT package_id, 0, from_owner_id, to_owner_id, created_at, accepted_at FROM transfer_offers t WHERE accepted_at IS NOT NULL AND offer_signature IS NULL AND NOT EXISTS (SELECT 1 FROM transfer_history h WHERE h.package_id = t.package_id)`.
  This adds one unsigned legacy link per legacy-accepted package (Open Decision A-D2).

### 4.3 Store API (`repository/src/store.rs`)

- `pub struct TransferTerms { sequence: i64, current_owner: OwnerRecord, current_ident: KeyRecord, release_digest: String }`
- `pub fn transfer_terms(&self, ident) -> Result<Option<TransferTerms>, String>`.
  The sequence is the count of `transfer_history` rows. The owner and key are
  the stored owner's current ident. The digest is `crypto::release_digest` over
  all versions.
- `create_transfer_offer(ident, from_owner, to_owner, sequence, release_digest, offer_signature)`.
  Inside the existing writing transaction (bug-274 re-check kept), recompute the
  terms. Refuse with `transfer terms are stale (sequence/releases changed); re-run the offer`
  on mismatch. The UPSERT also writes `sequence, from_ident_key_id, release_digest, offer_signature`.
- `pending_offer(ident, to_owner) -> Option<PendingOffer {sequence, from_owner_display, from_ident_fingerprint, release_digest, offer_signature}>`.
  The server reads it to verify the accept signature.
- `accept_transfer(ident, to_owner, to_ident_key_id, accept_signature)`. Inside
  the transaction:
  - keep the stale-offer guard `o.from_owner_id = p.owner_id`;
  - refuse a legacy offer (`offer_signature IS NULL`) with
    `this offer predates signed transfers; the owner must re-offer`;
  - recompute the terms and require them equal to the stored offer terms (same
    sequence, same digest, offerer's key still current);
  - `INSERT` the `transfer_history` row, rewrite `packages.owner_id`, close the offer;
  - log `transfer-accept` with the payload extended to
    `{"ident","to","sequence"}`. This is a new log payload field, which the spec
    documents.
- `transfer_history(ident) -> Vec<TransferRecord>` ordered by `sequence`. It
  includes from/to display names, key public bytes (or `None`), digest,
  signatures and timestamps.
- `PublishMetadata` gains `publisher_ident_fingerprint: Option<String>`
  (defaulted). `publish_package_version` writes `publisher_owner_id = owner_id`,
  `publisher_ident_key_id` (looked up by fingerprint among `owner_id`'s ident
  keys; an unknown fingerprint is an error), and `transfer_epoch` = COUNT of the
  package's `transfer_history` rows, all inside its transaction.
  `server.rs:publish_package` sets the fingerprint from the validated header.
- `list_package_versions` additionally returns `publisher_display`,
  `publisher_ident_fingerprint`, and `transfer_epoch`.

### 4.4 Wire (`repository/src/server.rs`)

`IndexResponse` gains `#[serde(default)] pub transfers: Vec<TransferLink>`:

```json
{ "sequence": 0, "fromOwner": "alice", "fromIdentKey": "ed25519:<b64>|null",
  "toOwner": "bob", "toIdentKey": "ed25519:<b64>|null", "releaseDigest": "<hex>|null",
  "offerSignature": "<b64url>|null", "acceptSignature": "<b64url>|null",
  "offeredAt": 1700000000, "acceptedAt": 1700000100,
  "toBinding": { "owner": "bob", "identKey": "ed25519:<b64 current>",
                 "identFingerprint": "<hex>", "nameBindingSignature": "<b64url>" } }
```

`toBinding` is the to-owner's **current** ident plus a fresh server
`name_binding_message` signature. Without it a client cannot trust any
intermediate owner's key without a second unsigned `/idents` fetch.

A link with null signatures is a **legacy link**: a chain terminator (C §4).

`IndexVersion` gains:

- `#[serde(default)] publisher: Option<Publisher {owner, identFingerprint: Option<String>}>`
- `#[serde(default)] epoch: u32`

`package_detail` serves `identKey` / `identFingerprint` of the **stored** owner
(`package_owner`), which fixes the mixed page.

### 4.5 Clients signing v2 (`repository/src/client.rs`, `src/cli/pkg.rs`)

- `client::transfer_offer(repo_url, paths, ident, to_owner)`:
  1. `fetch_index` (verified).
  2. The current owner is the last link's `toBinding.owner`, or `index.owner`.
  3. The local ident key for that owner must fingerprint-equal the served
     current binding, otherwise error `this machine does not hold <owner>'s current ident`.
  4. `sequence = transfers.len()`, `release_digest(versions)`.
  5. Sign v2 and POST `TransferOfferRequest` with the new `sequence` and
     `releaseDigest` fields.
- `client::transfer_accept` does the same with `fromOwner` / `fromIdentFingerprint`
  taken from the same index view, and signs the accept v2.
- `src/cli/pkg.rs:transfer_offer` drops the prefix-derived `from_owner`.

## Compatibility / Format Impact

- **`/index` JSON:** additive only (`transfers`, `versions[].publisher`,
  `versions[].epoch`).
  - An older client ignores them and keeps verifying untransferred packages and
    pre-transfer releases exactly as today.
  - For a transferred package's post-transfer release, an older client compares
    the header key with its prefix-owner pin and refuses. Fail-closed, never a
    silent accept.
  - A newer client talking to an older server sees empty defaults, which reads as
    "no transfers, publisher = origin, epoch 0". That is exactly what a pre-fix
    server's data means (§Verified properties).
- **Transfer requests:** `TransferOfferRequest` gains `sequence` and `releaseDigest`.
  The signatures change domain to `-v2`. An older CLI's transfer is refused with
  `invalid transfer offer ident signature` (A-D3).
- **Log:** `transfer-accept` payload gains `sequence`. Leaf hashing is unchanged.
- **Database:** additive table, triggers, columns, and idempotent backfill. No
  data is rewritten except NULL → backfilled publisher ids.
- **Unchanged:** `.mfp` bytes, proof/attestation JSON, the name-binding message,
  `IndexResponse.owner` / `identKey` semantics, and `index_canonical_hash`.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` = partial (say what remains); moot tasks are struck
> through with evidence, never deleted; fill each `Commit:` line when it lands.
> **An unticked box means NOT DONE.**

### Phase 1: Schema, history store, publisher record

Lands alone: nothing reads the new columns yet, and the existing transfer behavior
is preserved behind the new store signatures.

- [ ] Ask the operator for the legacy count (§Measured populations) and record it
      in Corrections. A-D2's choice does not depend on it, but the A-D2 follow-up
      (operator ratification) does.
- [ ] `repository/src/store.rs:migrate`: add the `transfer_history` table, the two
      triggers, the column additions, and the two backfill statements (§4.2).
- [ ] `repository/src/crypto.rs`: add `release_digest`,
      `transfer_offer_message_v2` and `transfer_accept_message_v2` (§4.1).
- [ ] `repository/src/store.rs`: `TransferTerms`, `transfer_terms`, the new
      `create_transfer_offer` / `accept_transfer` signatures, `pending_offer`,
      `transfer_history`, the `PublishMetadata.publisher_ident_fingerprint` write
      path, and the `list_package_versions` extension (§4.3).
- [ ] Migrate the 17 store.rs call sites of `create_transfer_offer(` /
      `accept_transfer(` to the new signatures through a `#[cfg(test)]` helper
      `offer_and_accept_for_test(store, ident, from_keys, to_keys)` that signs real
      v2 messages. Assertions stay unchanged: this is an API migration, not a
      re-baseline (`a_stale_offer_cannot_rebind_a_package_after_ownership_moved`
      and `transfer_offer_and_accept_error_branches` keep every assertion).
- [ ] Tests (`store.rs` tests):
  - `transfer_history_is_append_only_and_sequenced`: two transfers give sequences
    0 and 1, and an `UPDATE` / `DELETE` aborts with "append-only".
  - `stale_terms_refuse_offer_and_accept`: a version published between offer and
    accept refuses the accept.
  - `legacy_pending_offer_cannot_be_accepted`.
  - `migrating_a_legacy_database_records_legacy_transfers_and_publishers`: a
    hand-built pre-plan schema, as in
    `migrating_a_legacy_database_adds_the_metadata_columns_and_target_table`,
    ends with an unsigned seq-0 history row, publisher = prefix owner, epoch 0,
    and re-running `migrate` is a no-op.
  - `publish_records_publisher_and_epoch`.

Acceptance: history is recorded with signatures, cannot be mutated, and legacy
databases migrate deterministically.
  Check: `cargo test -p mfb_repository --lib -- store::tests::transfer store::tests::migrating store::tests::publish_records store::tests::legacy store::tests::stale store::tests::a_stale_offer` → `test result: ok`, 0 failed, and every named new test listed as `ok` (est. 3–5 min, compile-dominated; guess).
Commit: —

### Phase 2: Server handlers and the v2 signers (the uncertainty experiment)

Proves the premise that both sides can compute identical terms from `/index`
alone, using the real CLI signer against the real server.

- [ ] `repository/src/server.rs`: `TransferOfferRequest` gains `sequence: i64`
      and `#[serde(rename = "releaseDigest")] release_digest: String`.
      `transfer_offer`:
  - `session_and_ident(from_owner)`;
  - require `request.from_owner == current owner display` (exact); on mismatch,
    the error names the display form;
  - verify the v2 offer signature over (request terms + the offerer's current
    fingerprint);
  - `create_transfer_offer` with the signature.
- [ ] `transfer_accept`: `pending_offer(ident, to_owner)`, verify the v2 accept
      signature over the stored terms, then
      `accept_transfer(..., to_ident_key_id, signature)`.
- [ ] Delete the v1 builders from `crypto.rs`. Migrate the 4 server.rs tests and
      the 1 client.rs test that use them to v2 signing. Assertions stay the same,
      except that `transfer_and_org_endpoints_reject_malformed_and_wrong_signatures`
      gains one case: a v1-domain signature is refused.
  - Proof per AGENTS.md "never edit a test": (1) the tests were written for
    plan-10-D1's v1 protocol (`git log -S transfer_offer_message`). (2) They
    protect "both halves are ident-signed". (3) No spec or test depends on the v1
    bytes beyond the `03_signing.md` table row updated in Phase 4. (4) The v1
    message is proven replayable (§2), so its exact bytes are the defect.
- [ ] `repository/src/client.rs:transfer_offer` / `transfer_accept`: implement
      §4.5. `src/cli/pkg.rs:transfer_offer` drops the prefix owner.
- [ ] Tests:
  - `server.rs` `a_second_transfer_is_offered_by_the_current_owner` (alice→bob→carol;
    sequences 0 and 1; bob's offer accepted, alice's refused).
  - `a_replayed_link_is_refused`: re-POST the recorded seq-0 offer bytes after
    bob→alice; refused as stale/invalid.
  - `client.rs` `transfer_offer_and_accept_are_each_ident_signed` (migrated):
    asserts the v2 bytes.

Acceptance: a real CLI transfer round-trips under v2, and a replayed link is refused.
  Check: `cargo test -p mfb_repository --lib -- transfer` → `test result: ok`, 0 failed (est. 3–5 min; guess), and `cargo test --test cli_repo_governance repo_ownership_transfer_is_two_sided_and_rebinds_the_package` → `1 passed` (est. 8–15 min on a cold release build. Nothing smaller drives the shipped `mfb` signer against the shipped `mfb-repo` verifier, and a mismatch between those two is exactly the failure this phase must catch).
Commit: —

### Phase 3: `/index` transfer wire and the package page fix

- [ ] `server.rs`: add the `TransferLink`, `OwnerBinding` and `Publisher`
      structs. Add the `IndexResponse.transfers`, `IndexVersion.publisher` and
      `IndexVersion.epoch` fields (all `#[serde(default)]`). `package_index`
      fills them from `transfer_history` and the extended version rows, and
      signs a `toBinding` per link.
- [ ] Update the 5 `IndexVersion {` and 2 non-server `IndexResponse {` test
      literals (pkg.rs:2371, pkg.rs:2708, pkg.rs:2384, resolve.rs:1252,
      resolve.rs:1273, client.rs:2587) with the default values. These literal
      updates change no assertions.
- [ ] `server.rs:package_detail`: serve the stored owner's ident key.
- [ ] Tests (`server.rs`):
  - `the_index_serves_a_verifiable_transfer_chain`: after alice→bob, the link's
    offer signature verifies under alice's key, the accept under bob's, and the
    `toBinding` under the server key; versions carry `publisher.owner == "alice"`,
    `epoch == 0`.
  - `a_legacy_transfer_is_served_unsigned`.
  - `the_package_page_pairs_the_current_owner_with_their_key`.
  - `client.rs` `an_older_index_without_transfer_fields_deserializes` (JSON
    without the new keys gives an empty/None/0 result).

Acceptance: `/index` serves a chain whose every signature verifies, and an older
client's JSON shape still parses.
  Check: `cargo test -p mfb_repository --lib -- the_index_serves a_legacy_transfer the_package_page an_older_index package_index` → `test result: ok`, 0 failed (est. 3–5 min; guess).
Commit: —

### Phase 4: Spec sync for A's contracts

- [ ] `src/docs/spec/package-manager/01_repository-protocol.md`:
  - endpoint table rows for the transfer routes (request fields);
  - §Install Path index JSON plus prose for `transfers` / `publisher` / `epoch` /
    `toBinding` and legacy links;
  - §Accounts "Ownership transfer": v2 messages, sequence, digest, append-only
    history, stale-terms and legacy-offer refusals;
  - log-kind table note on `transfer-accept` payload;
  - remove the "`/index` still returns that prefix owner's ident key" clause from
    the deviation paragraph, replaced by "serves the signed transfer chain;
    clients do not yet verify it" until C.
  - Cite `[[repository/src/store.rs:accept_transfer]]`,
    `[[repository/src/crypto.rs:transfer_offer_message_v2]]`,
    `[[repository/src/server.rs:package_index]]`.
- [ ] `src/docs/spec/package-manager/03_signing.md`: replace the two
      `-transfer-*-v1` table rows with the `-v2` rows and their field lists.
- [ ] `src/docs/spec/tooling/07_cli-reference.md`: the `repo transfer` row now
      states "run by the package's current owner".

Acceptance: the spec renders with resolving citations and names no v1 transfer domain.
  Check: `cargo test --bin mfb spec` → `test result: ok`, and `grep -rn 'transfer-offer-v1\|transfer-accept-v1' src/docs/spec` → no output (est. 3–6 min; guess).
Commit: —

## Validation Plan

- Tests: Phases 1–3 add store, server and client tests, including the negative
  cases (replay, stale terms, legacy offer, trigger abort, v1 refusal).
- Coverage check: every new test lives in `repository/src/**` `#[cfg(test)]`,
  which `cargo test -p mfb_repository` runs (workspace member per root
  `Cargo.toml` `members`).
- Runtime proof: Phase 2's `cli_repo_governance` transfer test drives the shipped
  binaries.
- Doc sync: Phase 4.
- Final gate (once, after Phase 4): `cargo test -p mfb_repository --no-fail-fast > /tmp/168A.log 2>&1; echo EXIT=$?` → `EXIT=0`, then `cargo test --test cli_repo_governance > /tmp/168A-gov.log 2>&1; echo EXIT=$?` → `EXIT=0` (est. 15–25 min; guess). The whole-workspace suite runs once, at the end of plan-168-D. A touches no codegen, so the artifact gate cannot be affected.

## Open Decisions

- **A-D1, `/index` backward compatibility.** Recommended: additive fields, with
  top-level `owner` / `identKey` / `nameBindingSignature` still meaning the
  ident-prefix owner. Old clients keep working where they can and fail closed
  where they cannot. Alternative: redefine `owner` / `identKey` as the current
  owner. Rejected, because every older client's bug-581 owner check would then
  refuse every transferred index, including pre-transfer releases. (§4.4, §Compatibility)
- **A-D2, legacy (pre-plan) accepted transfers have no signatures.**
  Recommended: migrate them to unsigned seq-0 links that clients treat as chain
  terminators. Releases before the link verify exactly as today; nothing after
  it verifies (none can exist, per the bug). If the measured count is > 0, file a
  follow-up for an operator-witnessed ratification flow. Alternative: let the
  former owner co-sign a ratification now. It needs a new endpoint and
  authorization from an account that no longer owns the package, so it is
  deferred. (§4.2)
- **A-D3, v1 transfer requests from older CLIs.** Recommended: refuse. Accepting
  them would create unverifiable links again. Alternative: accept v1 and record
  it as a legacy link. Rejected, because it re-strands the recipient. (§4.1)
- **A-D4, sub-plan ordering.** The `/index` wire lands in A, not in its own
  letter, because the v2 signer computes its terms from `/index`. Splitting them
  would leave `mfb repo transfer` broken between letters. (§3)

## Corrections

(Filled in during execution.)

## Summary

The engineering risk in A is narrow: get the v2 terms identical on both sides,
and make the history truly append-only. Phase 2 tests the first with the real
binaries; the second is enforced by triggers. Publishing, client trust and the
build are untouched until B–D.
