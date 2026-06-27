# MFBASIC Package Registry — Design Document v0.1

## Goals

Avoid the failure modes of PyPI and npm specifically:

- **No arbitrary code execution at install time.** `.mfp` is portable typed binary representation with no native pointers; there is no install-script equivalent. Native bindings only enter through the declared `NATIVE_LINK_TABLE`, which is auditable.
- **No silent diamond-dependency chaos.** The resolver picks exactly one version per package *identity* — but "identity" is defined carefully (see Resolution, below) so this rule doesn't deadlock on legitimate major-version splits.
- **No dependency-confusion attacks.** Resolution is keyed off a `source` locator + content hash recorded in the lockfile, not off the bare import name. A bare import name is never globally unique and never has to be.
- **No silent compromise.** Every publish, key registration, key revocation, and ownership transfer is written to an append-only transparency log from day one.
- **No name-resurrection attacks.** Once a package identity or owner handle has published anything, it is permanently retired if abandoned — never recycled.

---

## 1. Naming & Identity Model

Two layers, deliberately decoupled:

**Layer 1 — Import name.** What source code writes after `IMPORT`. Matches existing identifier rules (`[A-Za-z_][A-Za-z0-9_]*`, ASCII, ≤255 bytes). Chosen freely by the package author. **Not required to be globally unique.** Two unrelated packages can both be named `geometry`.

**Layer 2 — ident.** Human-readable lookup key, used for *resolving a version range against the registry index*:

```
<owner>#<package>
```

- `<owner>` — a registered account or org handle (case-folded for uniqueness, original casing preserved for display).
- `<package>` — publisher-chosen slug, may differ from the `.mfp` header's `name`.
- `#` is safe as a separator since idents never appear in MFBASIC source — only in `project.json`, the lockfile, and CLI commands.

Not a URL, and not required to encode where the bytes live — that's what content addressing is for.

**Content addressing.** The actual unit of download/cache/reproducibility is the `.mfp` **content hash** defined by `package_format.md`: `SHA-256` over the entire package file with only the signature bytes zeroed. The registry index maps `ident@version → hash`. A locked install skips the index entirely and fetches by hash from any source — registry, mirror, local cache — that has that blob.

**`project.json` entry:**
```json
{
  "name": "shape",
  "ident": "<owner>#shape",
  "version": "2.1.0",
  "pin": false,
  "source": "registry:mfb"
}
```
`version` is the requested concrete version. `pin: true` makes it exact; otherwise it is an ABI anchor. `source` names which registry's index to consult for this `ident` (`mfb` = default public registry; self-hosted registries get their own locally-configured aliases — no URL needed in the manifest).

**Lockfile entry:**
```json
{
  "ident": "<owner>#shape",
  "version": "2.3.1",
  "hash": "<hash>"
}
```
(`version` is the selected version for human readability/debugging; `hash` is authoritative.)

**Major versions are not baked into the ident grammar.** A breaking v2 either continues under the same ident — in which case the resolver's single-version-per-identity rule (§13.1) applies as written, and a genuine v1/v2 diamond is a real conflict, surfaced with the conflicting requirers named — or the publisher chooses a new ident (`<owner>#shape2`) if they want both lines to coexist in a graph. This is a publisher convention, not a registry-imposed scheme.

**Reserved namespace:** `std#*` (covering `io`, `math`, `thread`, `errorCode`, etc.) is permanently un-registerable as an owner handle. Resolution step 1 (built-in package) short-circuits before consulting `project.json` at all.

**Identity permanence:** once `<owner>#<package>` has had a successful publish, that exact ident can never be reassigned — even if the owning account is deleted. Deleted handles become visible tombstones. Closes the PyPI-`ctx` / RubyGems-abandoned-gem class of attack.

**Typosquat policy (v1):** warn-only. At publish, run edit-distance check against high-traffic owner handles/package names; notify the publisher (and optionally the near-match owner). No hard blocking — avoids land-grab incentives and false-positive support load.

**CLI:**
- `mfb pkg add shape` — search by package name across all owners. If exactly one `<owner>#shape` exists, add it; if multiple, list them for interactive selection. Defaults to the latest version.
- `mfb pkg add <owner>#shape` — latest version from that specific owner.
- `mfb pkg add <owner>#shape@<version>` — use that concrete version as the ABI anchor.
- `mfb pkg add <owner>#shape@<version> --pin` — pin that exact version.
- `mfb pkg install` — resolves from `project.json` + lockfile. If the lockfile has an `ident`, fetch by `hash` directly (no index lookup). Otherwise resolve via the index and write the lockfile. Runs automatically after `add`.

---

## 2. Account Model

- **Account** = key-owned identity record. Registration is open to anyone and does not require a username/password, email, phone number, OAuth account, or approval step.
- Personal **owner handle** is reserved at registration time after the registry verifies proof-of-possession for the submitted public keys. Handles are case-folded for uniqueness, original casing is preserved for display, and `std` is permanently reserved.
- **Key-only authentication.** Login uses an authentication key challenge/response. The registry never stores passwords and never accepts password fallback for publishing authority.
- **Orgs** are additional owner handles with their own member list and roles (owner / admin / publisher). Layer 2 identity doesn't distinguish personal vs. org owners — same namespace, same rules. Org membership changes are signed by an authorized current ident key and written to the transparency log.
- **Recovery is explicit key rotation.** Losing every private key means losing the ability to administer the handle unless the account has previously configured additional recovery/auth keys or org maintainers. The registry should not support support-ticket-mediated account takeover.
- **Publish tokens** are optional delegated credentials for CI. They are minted only by an authenticated account, automatically constrained to that owner or one of that owner's packages, short default TTL, individually revocable, and never bypass the package's current ident/signing-key checks.
- **Ownership transfers** are explicit two-sided actions (initiator + acceptor), signed by both parties' current ident keys and written to the transparency log — never a support-ticket-mediated process.

---

## 3. Trust & Signing

- Key terms are deliberately not overloaded:
  - **Ident** (`ident`) — package identity, formatted as `<owner>#<package>`.
  - **Ident key** (`identKey`) — owner identity public key. This binds an owner handle to the key authority that controls that owner.
  - **Ident fingerprint** (`identFingerprint`) — fingerprint of the ident key.
  - **Signing key** (`signingKey`) — package-release public key.
  - **Signing fingerprint** (`signingFingerprint`) — fingerprint of the signing key.
  - **Auth key** (`authKey`) — registry login public key.
  - **Auth fingerprint** (`authFingerprint`) — fingerprint of the auth key.
- Each account/owner has three distinct Ed25519 key roles:
  - **Auth key** — authenticates the client to the registry with challenge/response login. This key authorizes sessions but does not sign package content.
  - **Ident key** — binds an owner handle to the current public identity for that owner. This is the key clients and transparency-log auditors use to confirm that a package was published by the owner currently controlling the ident.
  - **Signing key** — signs package releases for the owner that registered it. Signing authority is derived from the owner account; a key registered under one owner can never sign for another owner.
- Private keys are generated and stored by the client. The registry stores public keys, key fingerprints, owner-derived authority, key status (`current`, `past`, or `revoked`), rotation timestamps, revocation timestamps, and transparency-log entries only. The registry must not generate, return, escrow, or recover private signing keys; a CLI may generate local private keys before calling registration, but the API receives only public keys plus proof-of-possession signatures.
- Registration records the initial auth key, ident key, signing key, and proofs that the registrant controls all three corresponding private keys.
- Registration and rotation do not accept user-supplied signing scopes. The registry automatically binds each signing key to the authenticated owner, and publish-time ident checks enforce that the owner part of `<owner>#<package>` matches that binding.
- Key rotation changes the current ident key and current signing key for the owner. A rotation request must be made from an authenticated session and signed by the current ident key. It includes the replacement public keys plus proof-of-possession signatures for the replacement private keys. The old keys become `past` keys immediately and cannot be used for new publishes, but they remain stored permanently for historical verification.
- At publish time, the registry verifies all of the following:
  - the authenticated session maps to the owner part of the submitted `<owner>#<package>` ident;
  - the `.mfp` header's `ident` matches the route/request ident exactly;
  - the `.mfp` header's `identFingerprint` matches the current ident key for that owner;
  - the `.mfp` has `signatureType = 1` (`Ed25519`); unsigned packages are rejected by `registry:mfb`;
  - the release signature verifies with the current non-revoked signing public key for that owner;
  - the signing key is authorized by the current ident key and belongs to the owner in the package ident.
  **Unrecognized, revoked, stale, or wrong-owner keys are rejected outright** — never silently accepted as "first seen, trust it."
- At install/verification time, a package signed by a current key verifies normally. A package signed by a `past` key verifies only if the package's logged publish timestamp is earlier than that key's rotation timestamp. Clients should surface that state in output, e.g. `Verified with old signing key rotated on 2026-06-14`. A package signed after the recorded rotation timestamp with that old key is invalid.
- **Blobs are immutable, including signatures.** Published `.mfp` blobs are never re-signed after key rotation. Key history exists so old blobs can still be verified against their original signatures. The content hash is computed from the package bytes with only the signature bytes zeroed, so magic, container version, binary representation version, flags, signature type, signature length, metadata, binary representation length, and binary representation are all part of the content identity. `/blob/<hash>` still returns the original immutable package bytes that were published.
- **Revocation:** immediate stop on new authentication or publishes from that key, logged with timestamp. Already-published versions aren't auto-invalidated (avoids a self-inflicted outage); reviewed individually and moved to `blocked` if malicious.
- **Transparency log** (Merkle-tree, CT/Rekor-style), built from v1: every account registration, login-key registration, ident/signing-key rotation, key revocation, publish, release-state change, ownership transfer, identity, version, content hash, signing fingerprint, and timestamp. Clients pin the last-seen checkpoint (no rollback). Lets any maintainer audit "did I really publish this?" and makes registry-level compromises detectable rather than invisible.
- **Index signing root-of-trust:** v1 uses an offline registry root key, not only an online index key. The root metadata binds a stable registry ID (for example `registry:mfb`) to the root public key, the allowed online snapshot/timestamp keys, signature thresholds for registry metadata, metadata expiration rules, and the current root version. Clients must be configured with the expected registry ID and root key fingerprint before trusting any index response.
- **Online index keys:** short-lived online snapshot/timestamp keys sign mutable registry metadata. Snapshot metadata signs the current index state; timestamp metadata signs freshness for the snapshot. Clients reject expired metadata, metadata signed by keys not delegated from the offline root, registry-ID mismatches, rollback to an older root/snapshot/timestamp version, and any index entry whose signature chain does not validate to the configured offline root.
- **Log proofs for index entries:** every published version listed in the index must carry a transparency-log inclusion proof for the publish event. During resolution, clients verify the publish inclusion proof and a consistency proof from their last-seen checkpoint to the checkpoint referenced by the signed timestamp/snapshot metadata. First-time installs still trust the configured registry root, but they no longer trust an unauthenticated online index key alone.
- **Threshold/multi-sig** for critical-tier packages (N-of-M identity/signing-key approvals required to publish): deferred to v2.
- **First-install trust gap, stated plainly:** a client resolving a never-before-seen identity trusts the configured registry root and the signed, logged index state under that root. A compromised online snapshot/timestamp key alone should not be enough to create a silent first-install compromise, because the client also verifies delegation from the offline root, publish inclusion proof, checkpoint consistency proof, metadata freshness, and registry-ID binding.

---

## 4. Resolution & Lockfile

- Dependency graph nodes are idents (`<owner>#<package>`). For each ident, collect all requested ABI anchors and pins from every package that depends on it; resolve to one selected version per ident.
- A resolution failure produces a diagnostic naming the conflicting requirers directly, including the symbol ABI hashes that disagree or the pinned versions that cannot both be selected.
- `mfb.lock` records exact selected versions, requested versions, pins, source aliases, content hashes, key metadata, package/container versions, binary representation versions, ABI metadata, native metadata hashes, and transitive dependencies. Its schema is specified in `lockfile.md`.
- Re-resolving without `update` must reproduce byte-identical locks. A locked `install` with a current lockfile fetches by `hash` alone and does not consult the registry index.

---

## 5. Registry API & Storage (sketch)

- **Base implementation plan**: `repo-base.md` specifies the first repository
  implementation pass. It is limited to registration and authentication and
  creates an independent Rust project under `repository/**` with an executable
  named `mfb-repo`.
- **Server storage root**: `./mfb-repo --path <repo_path>` owns all
  server-side repository state under `<repo_path>`. The base layout is:
  `<repo_path>/meta.db` for SQLite3 metadata and
  `<repo_path>/packages/<hash>.mfp` for immutable package blobs. Registration
  and authentication must create `meta.db` and `packages/`, but they must not
  implement publishing in the base pass.
- **Client key/session locations**: `mfb repo register <owner_name>` creates a
  local keypair at `~/.mfb/keys/<owner_name>.pub` and
  `~/.mfb/keys/<owner_name>.prv`. `mfb repo auth <owner_name>` stores the
  one-hour server-signed JWT session at
  `~/.mfb/session/<owner_name>.ses`, one session file per owner.
- **Blob storage**: content-addressed, immutable, keyed by the `.mfp` content hash defined in `package_format.md`. **Write-once, permanent** — once a `.mfp` is released under a hash, `GET /blob/<hash>` works forever; the blob store has no delete path for normal operation. Infinitely cacheable — any CDN or third-party mirror can serve blobs without being trusted, since the client verifies hash + signature regardless of source.
- **Published version immutability:** after a successful publish, the mapping `<owner>#<package>@<version> → contentHash` is immutable forever. The hash for an existing version may never change, the package blob may never be replaced, and the package is never re-signed in place. A publisher that needs different bytes must publish a new version.
- **Index/metadata service**: maps `<owner>#<package>@<version> → hash`, plus release state, signature, signing fingerprint, and transparency-log reference for each version. Version records are append-only for identity/version/hash; mutable metadata such as description, README, links, advisory text, deprecation message, and release state may change without changing the blob. Small, mutable, short-cache.
- **Release states:**
  - `available` — normal release; eligible for ABI-compatible resolution.
  - `deprecated` — still eligible for resolution, but clients warn and show the replacement/advisory message when available.
  - `yanked` — not eligible for floating/latest/ABI-compatible resolution. Exact pinned resolution may still select it with a warning, and existing lockfiles continue to install by hash.
  - `blocked` — malware or similarly urgent safety response. New resolution and unlocked install reject it; locked install should fail unless an explicit emergency override policy is configured locally.
  - `legal-tombstoned` — narrow, audited legal/takedown process. The index keeps a tombstone record with ident, version, prior hash when legally allowed, reason class, timestamp, and log reference. Blob access follows the separately audited legal process rather than ordinary maintainer metadata controls.
- **Core endpoints**:
  - `POST /accounts/register` — open registration. It never returns private keys.
    ```json
    {
      "owner": "alice",
      "authKey": "<ed25519-public-key>",
      "identKey": "<ed25519-public-key>",
      "signingKey": "<ed25519-public-key>",
      "proofs": {
        "auth": "<signature-over-registration-challenge>",
        "ident": "<signature-over-registration-challenge>",
        "signing": "<signature-over-registration-challenge>"
      }
    }
    ```
    Response:
    ```json
    {
      "owner": "alice",
      "authFingerprint": "<fingerprint>",
      "identFingerprint": "<fingerprint>",
      "signingFingerprint": "<fingerprint>",
      "logEntry": "<transparency-log-entry>"
    }
    ```
  - `POST /auth/challenge` — starts key-based login.
    ```json
    {
      "owner": "alice",
      "authFingerprint": "<fingerprint>"
    }
    ```
    Response:
    ```json
    {
      "challengeId": "<opaque-id>",
      "nonce": "<random-nonce>",
      "expiresAt": "<timestamp>"
    }
    ```
  - `POST /auth/login` — completes login by verifying the auth key signature over the challenge.
    ```json
    {
      "challengeId": "<opaque-id>",
      "signature": "<auth-key-signature-over-nonce>"
    }
    ```
    Response:
    ```json
    {
      "sessionToken": "<short-lived-token>",
      "owner": "alice",
      "expiresAt": "<timestamp>"
    }
    ```
  - `POST /keys/rotate` — authenticated key rotation. The registry makes the replacements current for future publishes and logs the rotation.
    ```json
    {
      "owner": "alice",
      "newIdentKey": "<ed25519-public-key>",
      "newSigningKey": "<ed25519-public-key>",
      "proofs": {
        "newIdent": "<signature-over-rotation-challenge>",
        "newSigning": "<signature-over-rotation-challenge>"
      },
      "rotationStatement": {
        "previousIdentFingerprint": "<fingerprint>",
        "previousSigningFingerprint": "<fingerprint>",
        "newIdentFingerprint": "<fingerprint>",
        "newSigningFingerprint": "<fingerprint>",
        "reason": "routine-rotation"
      },
      "rotationSignature": "<current-ident-key-signature-over-rotation-statement>"
    }
    ```
    Response:
    ```json
    {
      "owner": "alice",
      "identFingerprint": "<fingerprint>",
      "signingFingerprint": "<fingerprint>",
      "rotatedAt": "<timestamp>",
      "pastIdentFingerprint": "<fingerprint>",
      "pastSigningFingerprint": "<fingerprint>",
      "logEntry": "<transparency-log-entry>"
    }
    ```
  - `GET /index/<owner>#<package>` — version list + metadata. The literal `#` is percent-encoded as `%23` in HTTP paths.
    ```json
    {
      "ident": "alice#shape",
      "versions": [
        {
          "version": "2.3.1",
          "hash": "<content-hash>",
          "publishedAt": "<timestamp>",
          "state": "available",
          "identFingerprint": "<fingerprint>",
          "signingFingerprint": "<fingerprint>",
          "signingKeyStatus": "current",
          "signingKeyRotatedAt": null,
          "abiIndex": {},
          "logEntry": "<transparency-log-entry>"
        }
      ]
    }
    ```
  - `GET /blob/<hash>` — returns the `.mfp` bytes for the content hash.
  - `POST /validate` — authenticated preflight check for a package artifact. Runs the same ident, current ident key, signing key, hash, `ABI_INDEX`, native metadata, and policy checks as publish, but does not write the blob store, index, metadata service, or transparency log.
    ```json
    {
      "ident": "alice#shape",
      "version": "2.3.1",
      "artifact": "<base64url-no-pad-mfp-bytes-or-upload-reference>",
      "contentHash": "<mfp-content-hash>",
      "identFingerprint": "<fingerprint>",
      "signingFingerprint": "<fingerprint>",
      "sessionToken": "<short-lived-token>"
    }
    ```
    Response:
    ```json
    {
      "valid": true,
      "contentHash": "<mfp-content-hash>",
      "abiIndex": {},
      "diagnostics": []
    }
    ```
  - `POST /publish` — authenticated publish. Runs the same checks as `POST /validate`, writes the content-addressed blob if absent, writes mutable index/metadata for `<owner>#<package>@<version>`, and records the publish in the transparency log.
    ```json
    {
      "ident": "alice#shape",
      "version": "2.3.1",
      "artifact": "<base64url-no-pad-mfp-bytes-or-upload-reference>",
      "contentHash": "<mfp-content-hash>",
      "identFingerprint": "<fingerprint>",
      "signingFingerprint": "<fingerprint>",
      "sessionToken": "<short-lived-token>"
    }
    ```
    Response:
    ```json
    {
      "ident": "alice#shape",
      "version": "2.3.1",
      "hash": "<content-hash>",
      "publishedAt": "<timestamp>",
      "state": "available",
      "blobStored": true,
      "logEntry": "<transparency-log-entry>"
    }
    ```
  - `POST /release-state` — authenticated release-state change for `<owner>#<package>@<version>`. Maintainers may set `available`, `deprecated`, or `yanked`; they may not set `blocked` or `legal-tombstoned`. State changes update mutable metadata and are recorded in the transparency log, but they never change the version's content hash and never delete `/blob/<hash>`.
    ```json
    {
      "ident": "alice#shape",
      "version": "2.3.1",
      "hash": "<content-hash>",
      "state": "yanked",
      "reason": "maintainer-request",
      "message": "Use 2.3.2 instead.",
      "stateStatementSignature": "<current-ident-key-signature-over-state-change-statement>"
    }
    ```
    Response:
    ```json
    {
      "ident": "alice#shape",
      "version": "2.3.1",
      "hash": "<content-hash>",
      "state": "yanked",
      "metadataChanged": true,
      "blobDeleted": false,
      "logEntry": "<transparency-log-entry>"
    }
    ```
  - `GET /root.json` — signed registry root metadata. Contains the registry ID, root public key fingerprints, delegated snapshot/timestamp keys, metadata signature thresholds, expiration policy, and root metadata version. Clients verify this against their configured registry ID and pinned root fingerprint.
  - `GET /snapshot.json` — online-signed snapshot metadata for the current index state. Contains index metadata hashes, snapshot version, expiration, transparency-log checkpoint reference, and signatures from snapshot keys delegated by the offline root.
  - `GET /timestamp.json` — online-signed freshness metadata for the current snapshot. Contains snapshot hash/version, expiration, transparency-log checkpoint reference, and signatures from timestamp keys delegated by the offline root.
  - `GET /log/checkpoint`, `GET /log/proof/<entry>` — transparency log access
- **Mirroring/federation**: because blobs are self-verifying (hash + signature), any mirror can serve them regardless of which registry's index pointed there. Self-hosted/internal registries are just additional `source` aliases configured locally — they don't need to be embedded in the ident itself.

---

## 6. Policy Summary

| Question | Decision |
|---|---|
| Major version = new identity? | No — ident grammar is `<owner>#package`; publishers use a separate ident (e.g. `<owner>#shape2`) by convention if v1/v2 must coexist |
| Transparency log? | Yes, from v1 |
| Offline root keys for index? | Yes, from v1; registry ID is bound to the offline root, which delegates online snapshot/timestamp keys |
| Threshold/multi-sig for critical packages | Deferred to v2 |
| Typosquat detection | Warn-only at v1 |
| Owner handle registration | Open key-based registration; no username/password, email, phone, or OAuth required |
| Unsigned packages | `registry:mfb` rejects them; install rejects them unless `path:`/`file:` source is explicitly allowed by `packagePolicy.allowUnsignedLocal` and recorded in `mfb.lock` |
| Release states | `available`, `deprecated`, `yanked`, `blocked`, `legal-tombstoned` |
| Yanking | Maintainer yanks keep the version record and hash; exact pinned resolution remains allowed with a warning |
| Blob vs. metadata lifecycle | `<ident>@<version> → contentHash` and `/blob/<hash>` are immutable after publish; descriptions/advisories/release state are mutable metadata |
| Name/identity recycling | Never, once published |

---

## 7. Lockfile Format

The lockfile format lives in `lockfile.md`. The registry design depends on these lockfile properties:

- Locked packages are fetched and verified by the `.mfp` content hash plus the immutable package signature.
- Registry and mirror endpoint URLs are local configuration; the lockfile stores source aliases, idents, and hashes.
- The lockfile records the transparency-log checkpoint, registry root fingerprint/version, snapshot/timestamp metadata versions, key metadata, selected versions, requested versions, publish timestamps, key rotation timestamps when a past key is used, pin state, ABI metadata, native metadata hashes, package/container versions, binary representation versions, and transitive dependency cache.
- When a locked package verifies with a `past` signing key, `install` must confirm `publishedAt < signingKeyRotatedAt` and should report that explicitly, e.g. `Verified with old signing key rotated on 2026-06-14`. If `publishedAt` is missing or is not earlier than the rotation timestamp, verification fails.
- `install` with a current lockfile does not resolve or update versions. `update` is the explicit operation that re-runs resolution.

---

## 8. Dependency Version Resolution (ABI-Generation Model)

### 8.1 The core idea

`project.json`'s `packages[].version` (and the `@<version>` in `mfb pkg add`) is **not** a semver range and **not** a pin. It's a **resolution anchor**: "find me whatever is ABI-compatible with this version, at its best available point."

This replaces the `^`/`~`/`>=`/wildcard range syntax in `project.md` §7 entirely. There is exactly one form: a concrete version string (`"2.1.0"`).

### 8.2 The `ABI_INDEX` section

Each published `.mfp` carries an `ABI_INDEX`: a map from each exported declaration's identity to a hash of that declaration's full public ABI shape. This is computed by the compiler at build time. It's not a single per-package number; it's one hash per exported symbol.

ABI v1 covers all caller-visible exported surface, including:

- exported functions and subs;
- exported record field names, types, order, mutability, and defaults;
- exported union member identities, tags, order, and defaults;
- exported enum member names and ordinals/discriminants;
- exported constants and their types and compile-time-visible values;
- exported global `LET`/`MUT` shape, including mutability and declared type;
- exported native wrapper function signatures and caller-visible native boundary behavior;
- exported resource ownership, borrow/consume behavior, close behavior, and sendability flags;
- caller-visible effect flags such as isolation or other call-site restrictions.

If a declaration is visible to consumers, v1 must either hash its full public shape or reject publishing the package as having incomplete ABI metadata.

Two versions **agree** on a symbol iff they carry the same hash for that symbol. A version `V` is a **valid substitute** for version `U` (from the perspective of a consumer built against `U`) iff:

```
ABI_INDEX(V) ⊇ ABI_INDEX(U)
```

— every `(symbol, hash)` pair present in `U`'s index is also present in `V`'s, with the *same* hash. `V` may freely contain additional symbols `U` didn't have.

This subsumes the single-integer "ABI generation" idea with something more precise and more tolerant of real-world history: it's a per-symbol compatibility relation rather than a global linear sequence, so it works fine even for non-linear cases — e.g. a maintained `1.x` patch branch alongside `2.x` development. A `1.4.3` patch release is a valid substitute for `1.4.0` as long as `ABI_INDEX(1.4.3) ⊇ ABI_INDEX(1.4.0)`, independent of whatever `2.x` is doing.

`ABI_INDEX` is mirrored into the registry index alongside `{ version, hash, ... }` so resolution (§8.3) doesn't require downloading candidate blobs. A `mfb pkg check-abi` command lets a publisher diff their working tree's `ABI_INDEX` against the latest published version *before* publishing, so an unintended breaking change isn't a surprise — and the diff names the exact symbol(s) that changed.

### 8.3 Resolution algorithm (`add` / `update` only)

**Single dependency.** Given a request `<ident>@<anchor-version>` (or no version, meaning "latest"):

1. Fetch `ABI_INDEX(anchor-version)`.
2. Among published versions `V` with `ABI_INDEX(V) ⊇ ABI_INDEX(anchor-version)`, select the highest eligible version.
3. Write that version + its content hash to `project.json`/lockfile.

This means: explicitly adding/updating a dependency always gets you the most up-to-date *non-breaking* release — bugfixes and additions — but never silently substitutes something that changed or dropped a symbol the anchor version had.

Release state affects eligibility:

- `available` versions are eligible.
- `deprecated` versions are eligible, but clients warn and include the deprecation message.
- `yanked` versions are not eligible for latest/floating/ABI-compatible resolution. They may be selected only by exact pinned resolution, with a warning, so a maintainer request does not make the version disappear for users who deliberately pin it.
- `blocked` and `legal-tombstoned` versions are not eligible for resolution.

**Diamond — multiple dependents on the same ident.** If dependent `X` anchors at `A_x` and dependent `Y` anchors at `A_y` (newer or older than each other, doesn't matter which):

1. Compute the union requirement `R = ABI_INDEX(A_x) ∪ ABI_INDEX(A_y)`.
2. If `R` contains two different hashes for the *same* symbol name (i.e. `A_x` and `A_y` disagree about that symbol's signature), that's an immediate, precisely-named conflict: *"X requires `foo` as defined in `<ident>@A_x`; Y requires `foo` as defined in `<ident>@A_y`; these signatures differ."*
3. Otherwise, select the version `V` (typically highest available) with `ABI_INDEX(V) ⊇ R`. That single `V` satisfies both `X` and `Y` simultaneously — they were never actually in conflict, just anchored at different points that happen to share a compatible future.
4. If `R` is internally consistent but no published `V` covers it yet, the diagnostic names exactly which symbol(s) are missing/changed relative to which dependent's requirement — not a generic "no compatible version."

**Crossing incompatibilities is always explicit.** If `update` would otherwise have to drop a symbol from `R` to find any candidate at all, it stops and shows the generated diff (which symbols would be lost/changed) rather than picking something anyway — this is the moment a human looks at what changed.

**Exact pinning** (`mfb pkg add <ident>@<version> --pin`) bypasses the `ABI_INDEX` comparison entirely and locks that literal version+hash — for reproducing a known environment or deliberately avoiding even compatible updates.

### 8.4 No hidden auto-updates

`mfb pkg install` **never** runs resolution. With a lockfile present (and `projectHash` matching), it fetches by `hash` only — full stop. Resolution (§8.3) only runs from `add`, `update`, or first-time `install` with no lockfile. There is no "check for updates and pull them in" behavior anywhere in `install`; that's only ever `update`, only ever explicit, and only ever visible as a diff to the lockfile before it's written.

### 8.5 Pre-1.0 / `0.x` packages

Semver convention treats all of `0.x` as unstable with no compatibility guarantee, which in practice means tooling either ignores that or treats every `0.x.y` as its own world. Here, it doesn't matter — `ABI_INDEX` is computed from the actual interface regardless of the version number's leading digit, so `0.x` packages get the same accurate per-symbol compatibility tracking as `1.x+`. This removes one of the more common "why did my build break on a patch bump" complaints in ecosystems that special-case `0.x`.

### 8.6 Residual risk, stated plainly

`ABI_INDEX` catches *signature*-level breakage, not *behavioral* breakage with an unchanged signature (a function that now returns subtly different results for the same inputs, with the same hash). That class of break can still pass an `ABI_INDEX` superset check. Two mitigations already in place cover this reasonably well: `install` never moves without `update`, so it can't happen silently on a teammate's machine or in CI; and `update` is a visible, reviewable diff, so a human sees the version jump (and can check the changelog) before it's committed to the lockfile.

### 8.7 Alignment with `project.md` §7

`project.md` uses the same model: `packages[].version` is a single concrete requested version string, and `packages[].pin` controls exact selection. Range syntax such as `^`, `~`, inequalities, and wildcards is invalid.

---

## Open for future passes

- Exact legal-tombstone audit procedure and public transparency fields for emergency takedowns
- Critical-package threshold values (download count / dependent count)
- Federation/mirror discovery protocol details
