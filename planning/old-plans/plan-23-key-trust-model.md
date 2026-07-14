# plan-23: The key & trust model (final design)

Last updated: 2026-07-04
Overall Effort: x-large (split into lettered sub-plans before implementation)
Status: **DESIGN SETTLED 2026-07-04** — this document records the agreed design.
Supersedes plan-10-A's key phases (plan-10-A Phase A2 `/blob`+`/index` survives
unchanged). The server-held-ident code sitting **uncommitted** in the working
tree is the rejected precursor — discard it (§8).

---

## 1. The one rule

An Ed25519 keypair is two halves: the **private key** makes signatures; the
**public key** only checks them. Whoever holds a private key can do — and
forge — everything that key vouches for. Every design decision below is the
same question: *which private keys live where.*

## 2. The four keys

| Key | How many | Private key lives | Job |
|---|---|---|---|
| **server key** | 1 per registry | on the server (the only private key it holds) | signs attestations |
| **ident** | 1 per account | on **every linked machine** (copied at link time) | *is* the user's identity; signs proofs |
| **auth** | 1 per machine | that machine | logs into the registry API; nothing more |
| **signing** | 1 per package, one-off | exists for the duration of one build, then discarded | signs the `.mfp` |

Linked machines are **full equals** — there is no primary machine. The
account = owner name + ident keypair. Machines are just where the files sit.

Properties that fall out:

- The server holds no user private keys: it can never sign a package or a
  proof as any user. A full server compromise yields zero user keys.
- Forging a package requires **two independent credentials**: the ident
  private key (to sign the proof) *and* a live authenticated session (to get
  the attestation). Either alone is useless.
- Signing keys rotate **per package** — the original "rotate every 180 days"
  requirement is subsumed (maximally: a fresh key every build). Nothing in
  the system expires: proofs and attestations are notarized statements of
  fact at a moment (`issued`), true forever. Published packages never rot.

## 3. Flows

### 3.1 Register (first machine)

1. CLI generates **ident** and **auth** keypairs locally.
2. Sends both **public** keys + a proof-of-possession for each (signature
   over a server challenge; the register message gains a role discriminator
   so an auth proof cannot be replayed as an ident proof).
3. Server stores owner + both public keys. Privates never leave the machine.

Local store (per-repo hash dir, files 0600, base64url):

```
~/.mfb/<repo-hash>/keys/
    <owner>.auth.prv / .pub
    <owner>.ident.prv / .pub
```

### 3.2 Link a new machine (machines side by side)

1. New machine generates its **own auth keypair**, registers it to the
   account (authenticated by an existing session / pairing approval).
2. The link **copies the ident private key** to the new machine: a one-time
   pairing code shown on the old machine, typed on the new; the key crosses
   encrypted under a code-derived key (argon2id), relayed as a single-use,
   short-TTL blob the server cannot read. (QR/local-network transfer is an
   acceptable alternative transport.)
3. Done. Both machines are equal. No approvals, no ceremonies, ever again.

### 3.3 Build (`mfb build --sign <owner>`) — requires server reachable

1. Generate a **one-off signing keypair**.
2. `POST /signing` (authenticated session):
   request `{ owner, ident: "<owner>#<package>", version, signingFingerprint }`
   → server verifies session owner, logs the request, returns the signed
   **attestation** (§5). This pre-registers the exact key for the exact
   package+version before anything is published.
3. Locally build the **proof** (§5) and sign it with the **ident** key.
4. Write the header (§4); sign the header prefix with the one-off key;
   **discard the one-off private key**.

### 3.4 Publish

Server-side checks, in order (any failure refuses):

1. session owner == header owner; header `ident` starts with `<owner>#`.
2. attestation verifies under the server's own key; `repoFingerprint` is ours.
3. `attestation.ident == header.ident` and `attestation.version == header.version`
   (an attestation cannot be reused for another package or version).
4. `attestation.signingFingerprint == fp(header.signingKey)` — the package is
   signed by the key the server was told about, not a random key.
5. `attestation.identFingerprint == fp(header.identKey)`, and the attestation
   matches the server's **current** name↔ident binding (stale after an ident
   rotation → refuse; client refetches and rebuilds — rare and correct).
6. proof verifies under `header.identKey`; `proof.ident/version/owner/
   signingFingerprint/identFingerprint` all match the header.
7. `packageBinaryHash` recomputes over the payload; the package signature
   verifies under `header.signingKey`.

### 3.5 Install / verify (client, offline after download)

Anchors: the **server public key** (shipped/pinned once per registry) and the
owner's **identKey pinned in `project.json`** on first `pkg add` (existing
mechanism — the file-embedded key is never the trust root).

1. `header.identKey` == pinned ident key, else **Tampered**.
2. attestation verifies under the pinned server key; its `ident`, `version`,
   `identFingerprint`, `signingFingerprint` match the header, else **Tampered**.
3. proof verifies under `identKey`; fields match the header, else **Tampered**.
4. package signature verifies under `header.signingKey` over the signed
   prefix, else **Tampered**.
5. SHA-256(payload) == `packageBinaryHash`, else **Tampered**.
6. `signatureType == 0` (unsigned) stays allowed for local `file://`
   development packages only, as today.

### 3.6 Bad days

- **Machine lost/stolen**: revoke that machine's auth key. The thief holds a
  copy of the ident key, so also **rotate the ident**: new ident keypair
  signed by the old one (a chain link), server reissues bindings, consumers'
  tooling follows the signed chain and updates pins automatically. Old
  packages remain valid (their proofs/attestations were true when issued).
- **Ident lost entirely** (all machines + backups): re-anchor ceremony —
  after out-of-band verification the server binds the name to a fresh ident
  with **no chain link**; loud, logged, consumers get a hard warning instead
  of a silent swap. Survivable, deliberately not painless.
- **Server compromised**: attacker can mis-bind *names* for consumers who
  have not yet pinned the real ident, and can issue attestations — but
  cannot sign proofs, so cannot forge a package that passes §3.5 step 3 for
  any already-pinned consumer. Detection: the log (§7).

## 4. The `.mfp` header (container v-next)

```
magic              8 bytes        unchanged
containerMajor     u16            unchanged        (bump containerMinor for
containerMinor     u16            unchanged         the new layout)
binaryReprMajor    u16            unchanged
binaryReprMinor    u16            unchanged
flags              u32            unchanged

nameLength/name                   unchanged  ("<library_name>")
identLength/ident                 unchanged  ("<owner>#<package>")
versionLength/version             unchanged
authorLength/author               unchanged
urlLength/url                     unchanged

identKeyLength/identKey           ident PUBLIC key
signingKeyLength/signingKey       NEW: one-off PUBLIC key

proofLength/proof                 NEW: JSON, ident-signed at build time
proofSigLength/proofSig           NEW: 64-byte ident signature over proof

attestationLength/attestation         NEW: JSON, server-signed, fetched per build
attestationSigLength/attestationSig   NEW: 64-byte server signature

packageBinaryHash  byte[32]       NEW: SHA-256 of packageBinaryRepr
binaryReprLength   u64            unchanged (inside the signed prefix)

signatureType      u16            unchanged (1 = Ed25519, 0 = unsigned)
signatureLength    u32            unchanged
signature          byte[64]       made by the one-off signing key; signs
                                  everything above this point

packageBinaryRepr  byte[...]      unchanged
```

Removed fields: `identFingerprintLength/identFingerprint`,
`signingFingerprintLength/signingFingerprint` (derivable from the full keys).

Signature input (domain-tagged, prefix-style — replaces the zeroed-signature
whole-file hash):

```
"MFP-PACKAGE-v2\0" || SHA-256(header bytes [0 .. offset of signature))
```

The signed prefix includes `signatureType`/`signatureLength` and
`packageBinaryHash`/`binaryReprLength`; the payload is covered transitively
via `packageBinaryHash` (welds header to payload; header grafting is
impossible; the payload can be streamed/verified separately).

Proof and attestation signatures are domain-tagged the same way:
`"MFP-PROOF-v1\0" || proofBytes` and `"MFP-ATTEST-v1\0" || attestationBytes`.

The signed manifest inside the payload must repeat the header identity
(`ident`, `version`, `identKey`, `signingKey` fingerprints) and verifiers
reject a mismatch, as today.

## 5. The two JSON blobs

No `expires` anywhere — these are statements of fact at `issued`, valid
forever; freshness is enforced live at publish time (§3.4 step 5), never by
a clock inside a shipped file.

```
Proof (ident-signed, minted locally at build time):
{ "owner": "alice",
  "ident": "alice#toolbox",
  "version": "1.2.3",
  "identFingerprint": "<hex sha256 of identKey>",
  "signingFingerprint": "<hex sha256 of signingKey>",
  "issued": <UTC timestamp> }

Attestation (server-signed, fetched per build via POST /signing):
{ "repoFingerprint": "<hex sha256 of server public key>",
  "owner": "alice",
  "ident": "alice#toolbox",
  "version": "1.2.3",
  "identFingerprint": "<hex sha256 of identKey>",
  "signingFingerprint": "<hex sha256 of signingKey>",
  "issued": <UTC timestamp> }
```

Both blobs pin the exact package (`ident` + `version`) and the exact one-off
key, so a leaked one-off key + its paperwork is worth exactly one
already-published package — nothing.

## 6. What each key must NOT be able to do

- **auth**: cannot sign proofs or packages — a stolen login can request
  attestations (logged) but can never produce a package that verifies.
- **server key**: signs attestations only — can never produce a proof, so it
  can never impersonate a user to a consumer who has pinned that user.
- **ident**: never signs packages directly — only proofs; the per-package
  key does the byte-signing so the ident signature surface stays tiny.
- **signing (one-off)**: no standing power — dies at the end of the build.

## 7. Transparency log

Append-only, hash-chained record of: registrations, name bindings and
re-binds, `/signing` attestation requests, ident rotations/re-anchors,
publishes, revocations. Every forgery path that remains (§3.6 server
compromise; stolen auth requesting attestations) is forced to leave a signed
entry in this log *before* the act. Ties into plan-10-C C1; the `logEntry`
fields in the protocol become real here.

## 8. Gap vs. the tree today

| Piece | Today (`main`) | Target |
|---|---|---|
| Keys per user | 1 (`auth`), aliased as all roles | auth (per machine) + ident (per account) + one-off signing |
| Server-held privates | none | none — plus its own server key |
| `/keys/signing` | returns the auth key 3 ways | **replaced by `POST /signing`** (attestation issuance, §3.3) |
| `build --sign` | signs with the auth key, needs `/keys/signing` match | one-off key + local proof + fetched attestation |
| Header | identKey + 2 fingerprints; zeroed-sig whole-file hash | §4 layout; prefix signature; packageBinaryHash |
| Verify | pinned identKey verifies the package signature directly | §3.5 chain (server key + pinned ident anchors) |
| Machine story | single keypair per owner | link copies ident; per-machine auth; equal machines |

**Uncommitted working-tree changes (2026-07-04)** in
`repository/src/{store,server,client,local}.rs`: server-generated ident
private key stored in SQLite (`keys.private_key`), `/keys/signing` minting
signing keys server-side and returning the **private** key, +2 tests. That
is the rejected server-trusted model. **Disposition: discard** — revert
these files before sub-plan A lands. Do not commit.

## 9. Spec updates (required, ships with the phases)

Every phase updates its spec topics in the same change (house rule); the
full list, so none is missed:

- `src/docs/spec/package/01_container-format.md` — §4 header layout, removed
  fingerprint fields, prefix signature + domain tag, `packageBinaryHash`,
  proof/attestation blobs, container version bump, header/manifest match rule.
- `src/docs/spec/package/02_binary-representation.md` +
  `03_metadata-encoding.md` — manifest identity fields (identKey/signingKey
  fingerprints replace the old ident/signing fingerprint metadata).
- `src/docs/spec/package/12_verifier-rules.md` — §3.5 verification chain.
- `src/docs/spec/package-manager/01_repository-protocol.md` — register with
  two keys + role-separated proofs, `POST /signing`, publish checks (§3.4),
  auth-key add/revoke, link/pairing endpoints, ident rotation/re-anchor.
- `src/docs/spec/package-manager/02_key-store.md` — `<owner>.auth.*` /
  `<owner>.ident.*` layout, pairing-code transfer, per-machine sessions.
- `src/docs/spec/package-manager/03_signing.md` — full rewrite to §2/§5/§6
  (four keys, proof, attestation, two-credential property, signing domains).
- `src/docs/spec/package-manager/04_owner-names.md` — unchanged (verify).
- `src/docs/spec/tooling/cli-reference` topics — `mfb repo register`,
  `mfb repo link`, `mfb machine revoke`, `mfb key rotate`, `build --sign`.
- `src/docs/spec/diagnostics` `error-codes` — every new refusal in §3.4/§3.5
  gets a code; table is the build input for `errorCode::`.

## 10. Open decisions (recommendation first)

1. **Link transport** — recommend pairing-code + argon2id-encrypted relay
   blob (works headless); QR/local-network as a later nicety.
   DECISION: recommend pairing-code + argon2id-encrypted relay blob
2. **Old-reader behavior on the container bump** — recommend: bump
   `containerMinor`, and make the new reader reject older minors for
   *signed* packages (the old layout can't be verified under the new chain)
   while still reading unsigned local packages.
   DECISION:  containerMajor = 1 containerMinor = 0 no backwards compat needed
              Update mfb to verify version 1.0
3. **`repoFingerprint` distribution** — recommend: server public key shipped
   in client config per registry URL (`~/.mfb/<repo-hash>/server.pub`,
   fetched on first contact, pinned thereafter), fingerprint printed for
   out-of-band comparison.
   DECISION: agreed. Add a /ident route to get the server.pub as well
4. **Attestation for `pkg validate`** (pre-publish dry runs) — recommend:
   same `/signing` route, attestations are cheap and logged; no special case.
   DECISION: pkg validate <pkg> - should validate an existing packages, check the
             signatures, etc. is asking "is this package correct?". Its not meant
             to be a pre-signing.

## 11. Sub-plan documents

Split by effort into two sub-plans; this file is the overview/index holding
the design. Open the lettered file for phases, tasks, and acceptance; remove
each lettered document as it lands, and this index when the last one does.

| Doc | Effort | Bundles | Depends on |
|---|---|---|---|
| [plan-23-A](plan-23-A-trust-core.md) — Trust core | medium | reset + two-key register + server key/`/ident` · `POST /signing` + `build --sign` + v1.0 header · publish §3.4 + verify §3.5 | — |
| [plan-23-B](plan-23-B-machines-log.md) — Machines & log | medium | link (pairing transfer) + revoke · ident chain rotation + re-anchor · transparency log (subsumes plan-10-C C1) | A |

Cross-cutting test requirements (both docs carry them): the two-credential
negative cases (ident-only forgery fails at publish; auth-only forgery fails
verification) and a tampered-field sweep across every header field.

## 12. Summary

One account = a name + an ident keypair the user holds on every linked
machine (machines are equals; linking copies the key). Logins are cheap
per-machine auth keys that can't forge anything. Every package is signed by
a fresh one-off key vouched twice — by the user's ident (proof) and by the
server (attestation), both pinned to that exact package and version, both
timestamped facts that never expire. The server's only private key signs
attestations; it can never impersonate a user. Verifiers walk: pinned server
key → attestation → pinned ident → proof → one-off key → bytes; any swapped
byte or key breaks a link.
