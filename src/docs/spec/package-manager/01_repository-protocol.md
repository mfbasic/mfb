# Repository Protocol

The wire protocol between the `mfb` client and a repository (registry) service.
It backs four commands: `mfb repo register`, `mfb repo auth`, `mfb pkg publish`,
and `mfb build --sign`. (`mfb pkg add` does not use this protocol: it accepts
only `file://` URLs and copies the `.mfp` into `packages/`
locally.[[src/cli/pkg.rs:add_package]][[src/manifest/package.rs:package_file_url_path]])
The client side is the `mfb_repository` crate's `client`
module; the reference server is the same crate's `server` module. This topic
owns the HTTP surface (endpoints, methods, JSON bodies), the challenge-response
authentication flow, the JWT session token, and the validate-then-publish
sequence. Key generation and signing domains are owned by *signing*; on-disk key
and session storage is owned by *key-store*.

All mutating requests are `POST` with a JSON body and expect a JSON response
(`GET` serves `/health` and `/ident`). Binary fields (public keys, proofs,
nonces, signatures, package artifacts) are **base64url, no padding**. Hashes
and key fingerprints are lowercase **hex**.

## Transport and Endpoint Base

The base URL comes from the `MFB_REPO_URL` environment variable, falling back to
`DEFAULT_REPO_URL` = `http://127.0.0.1:7777`.[[repository/src/client.rs:repo_url_from_env]][[repository/src/lib.rs:DEFAULT_REPO_URL]]
Every request URL is `format!("{base}{path}")` with a single trailing slash
trimmed from the base, so `MFB_REPO_URL=https://repo.example/` and the path
`/publish` produce `https://repo.example/publish`.[[repository/src/client.rs:post_json]]

`post_json` is the single transport helper. On a 2xx it deserializes the typed
response. On a non-success status it reads the body, and if the body parses as
`{"error": "..."}` it returns that string verbatim; otherwise it returns
`repository request failed with status <status>: <body>`. A connection failure
returns `failed to connect to repository service: <err>`.[[repository/src/client.rs:post_json]][[repository/src/server.rs:ErrorResponse]]

The reference server ships as the `mfb-repo` binary: `mfb-repo --dbpath
<db_path> --datapath <data_path> [--listen <addr:port>]`. It listens on
`127.0.0.1:7777` by default, prints `MFB_REPO_LISTEN=<actual addr>` once bound,
and keeps its state as a SQLite database at `<db_path>` plus a blob directory
at `<data_path>`. On first run it generates its own Ed25519 **server keypair**
— the only private key the server ever holds; the private half never appears
in any response.[[repository/src/main.rs:parse_args]][[repository/src/server.rs:serve]][[repository/src/store.rs:open_repository]]

### Encoding

| Field class | Encoding | Helper |
| --- | --- | --- |
| public/private keys, proofs, nonces, signatures, artifact bytes | base64url **no pad** | `crypto::encode_bytes` / `crypto::decode_bytes` (`URL_SAFE_NO_PAD`) |
| content hash, key fingerprint | lowercase hex | `crypto::fingerprint` (SHA-256) / `content_hash_hex` |
| session token | compact JWS (three base64url segments) | `jsonwebtoken` |

[[repository/src/crypto.rs:encode_bytes]][[repository/src/crypto.rs:fingerprint]]

## Endpoint Summary

| Path | Method | Auth | Request | Response | Backs |
| --- | --- | --- | --- | --- | --- |
| `/health` | GET | none | — | `{"ok": true}` | (liveness) |
| `/ident` | GET | none | — | `ServerIdentResponse` | server-key pinning |
| `/accounts/register` | POST | proof signatures | `RegisterRequest` | `RegisterResponse` | `repo register` |
| `/auth/challenge` | POST | fingerprint match | `ChallengeRequest` | `ChallengeResponse` | `repo auth` (step 1) |
| `/auth/login` | POST | challenge signature | `LoginRequest` | `LoginResponse` | `repo auth` (step 2) |
| `/signing` | POST | session token | `SigningRequest` | `SigningResponse` | `build --sign` |
| `/keys/rotate` | POST | session + old-ident signature | `RotateRequest` | `RotateResponse` | `key rotate` |
| `/log/checkpoint` | GET | none | — | `CheckpointResponse` | checkpoint pin |
| `/log/proof/<index>` | GET | none | `?size=N` | `InclusionProofResponse` | inclusion proofs |
| `/log/consistency` | GET | none | `?from=M&to=N` | `ConsistencyProofResponse` | append-only audit |
| `/log/publish` | GET | none | `?ident=&version=` | `LogEntry` | `pkg verify --proof` |
| `/idents/<owner>` | GET | none | — | `IdentChainResponse` | pin-follow (`pkg verify`) |
| `/machines/link` | POST | session token | `LinkStartRequest` | `LinkStartResponse` | `repo link --start` |
| `/machines/link/fetch` | POST | pairing code + proof | `LinkFetchRequest` | `LinkFetchResponse` | `repo link` |
| `/machines/revoke/challenge` | POST | none (challenge issuance) | `RevokeChallengeRequest` | `ChallengeResponse` | `machine revoke` (step 1) |
| `/machines/revoke` | POST | ident signature | `RevokeRequest` | `RevokeResponse` | `machine revoke` (step 2) |
| `/validate` | POST | session token | `PackageArtifactRequest` | `ValidatePackageResponse` | `pkg publish` (step 1) |
| `/publish` | POST | session token | `PackageArtifactRequest` | `PublishPackageResponse` | `pkg publish` (step 2) |

[[repository/src/server.rs:serve]]

JSON field names use camelCase on the wire (set by `#[serde(rename)]`), even
though the Rust fields are snake_case. The tables below give the wire names.

## Server Identity — `GET /ident`

Returns the registry's own public key so clients can pin it as `server.pub` on
first contact (see *key-store*). The fingerprint is the same value attestations
later name as `repoFingerprint`.[[repository/src/server.rs:server_ident]]

```json
{
  "serverKey": "<base64url server public key>",
  "serverFingerprint": "<hex sha256 of server public key>"
}
```

The client verifies that `serverFingerprint` matches the key it decodes, pins
the key on first use, and hard-fails if a later fetch disagrees with the pinned
key.[[repository/src/client.rs:ensure_server_key]][[repository/src/local.rs:pin_server_key]]

## Owner Registration — `/accounts/register`

Backs `mfb repo register <owner_name>`. The client validates the owner name,
generates **two** fresh Ed25519 keypairs locally — the machine's `auth` key and
the account's `ident` key — and builds one proof-of-possession per key over the
role-discriminated registration message (signing domain `mfb-repo-register-v1`;
the role string is inside the signed bytes, so an auth proof can never be
replayed as an ident proof). Both keypairs are **written to local storage
first**, then the request is posted. If the POST fails the client **removes the
just-written keypairs** (rollback), so a failed registration leaves no local
keys. Private keys never leave the machine.[[repository/src/client.rs:register]][[repository/src/crypto.rs:registration_message]]

Request `RegisterRequest`:[[repository/src/server.rs:RegisterRequest]]

```json
{
  "owner": "alice",
  "authKey": "<base64url auth public key>",
  "identKey": "<base64url ident public key>",
  "proofs": {
    "auth": "<base64url signature over registration_message(auth)>",
    "ident": "<base64url signature over registration_message(ident)>"
  }
}
```

Response `RegisterResponse`:[[repository/src/server.rs:RegisterResponse]]

```json
{
  "owner": "alice",
  "authFingerprint": "<hex sha256 of auth public key>",
  "identFingerprint": "<hex sha256 of ident public key>"
}
```

The server decodes both keys and proofs, verifies each proof against its own
role's registration message, and records the owner with both public keys
(roles `auth` and `ident`, both `status='current'`). A duplicate owner name
yields a `409 Conflict` (message contains `already in use`); malformed input or
an invalid/role-swapped proof yields
`400`.[[repository/src/server.rs:register]][[repository/src/server.rs:conflict_or_bad_request]]
The CLI prints `Registered owner <owner> with auth fingerprint <fp> and ident
fingerprint <fp>`.[[src/cli/repo.rs:run_repo_command]]

## Challenge-Response Authentication

Backs `mfb repo auth <owner_name>`. Two round trips: obtain a nonce, then prove
possession of the private key by signing it.

### Step 1 — `/auth/challenge`

The client reads the local private key, derives the public key, optionally
cross-checks it against the stored public key (mismatch ⇒ `mismatched local key
fingerprint`), computes the SHA-256 fingerprint, and requests a
challenge.[[repository/src/client.rs:auth]]

If the local private key is **missing**, the client still probes
`/auth/challenge` with an empty fingerprint: if the server replies `unknown
owner` that error is surfaced; otherwise the original local-read error is
returned. This distinguishes "you never registered" from "your key is gone".[[repository/src/client.rs:auth]]

Request `ChallengeRequest`:[[repository/src/server.rs:ChallengeRequest]]

```json
{ "owner": "alice", "authFingerprint": "<hex fingerprint>" }
```

Response `ChallengeResponse`:[[repository/src/server.rs:ChallengeResponse]]

```json
{
  "challengeId": "<opaque id>",
  "nonce": "<base64url random bytes>",
  "expiresAt": 1700000000
}
```

The server first checks the owner exists (an unknown owner yields `400 unknown
owner`, so the client's missing-key probe still works), then challenges the
**specific machine's** auth key by fingerprint — an account holds one current
auth key per linked machine (plan-23 §2), so auth-key resolution is always
fingerprint-scoped; no current key with that fingerprint yields `400
mismatched local key fingerprint` (which is also what a **revoked** machine
sees). The nonce is 32 random bytes and the challenge expires **300 seconds**
after issue.[[repository/src/server.rs:challenge]][[repository/src/store.rs:create_auth_challenge]]

### Step 2 — `/auth/login`

The client decodes the nonce, builds the challenge message (signing domain
`mfb-repo-auth-v1`, binding `challengeId` and `nonce`), signs it with the private
key, and posts the login. On success it **writes the returned session token to
local storage** keyed by owner.[[repository/src/client.rs:auth]][[repository/src/crypto.rs:challenge_message]]

Request `LoginRequest`:[[repository/src/server.rs:LoginRequest]]

```json
{ "challengeId": "<from challenge>", "signature": "<base64url signature>" }
```

Response `LoginResponse`:[[repository/src/server.rs:LoginResponse]]

```json
{
  "sessionToken": "<JWT>",
  "owner": "alice",
  "expiresAt": 1700003600
}
```

The server completes the challenge (verifying the signature over the challenge
message). A replayed/already-consumed challenge yields `409` (message contains
`reused challenge`); an unknown challenge id yields `400 unknown challenge` and
a lapsed one `400 expired challenge`. Completion is single-use: the challenge
row is marked used in the same transaction that accepts the
signature.[[repository/src/server.rs:login]][[repository/src/server.rs:conflict_or_bad_request]][[repository/src/store.rs:complete_challenge]]
The CLI prints `Authenticated owner <owner> until <expiresAt>`.[[src/cli/repo.rs:run_repo_command]]

### Session Token (JWT)

`sessionToken` is an HS256 JWT signed with the server secret. Lifetime is **3600
seconds** (`exp = iat + 3600`). Claims:[[repository/src/server.rs:SessionClaims]][[repository/src/server.rs:login]]

| Claim | Meaning |
| --- | --- |
| `sub` | owner display name |
| `owner_id` | internal owner row id |
| `auth_fingerprint` | hex fingerprint of the auth key at issue time |
| `iat` / `exp` | issued-at / expiry (unix seconds; `exp - iat == 3600`) |
| `jti` | random UUID; also recorded server-side as a session row |

Server verification (`verify_session_token`) requires three things: a valid
HS256 signature under the server secret, `exp` not in the past
(`validate_exp`), and a `jti` that still exists in the session table. Failure of
signature or expiry returns `expired or malformed session token`; an unknown
`jti` returns `unknown session token`.[[repository/src/server.rs:verify_session_token]]

## Attestation Issuance — `POST /signing`

Used by `build --sign` (plan-23 §3.3): an authenticated build pre-registers
its **one-off signing key** for one exact package+version and receives the
server-signed **attestation** naming it. Requires a session
token.[[repository/src/client.rs:request_attestation]]

Request `SigningRequest`:[[repository/src/server.rs:SigningRequest]]

```json
{
  "owner": "alice",
  "ident": "alice#toolbox",
  "version": "1.2.3",
  "signingFingerprint": "<hex sha256 of the one-off public key>",
  "sessionToken": "<JWT>"
}
```

Response `SigningResponse`:[[repository/src/server.rs:SigningResponse]]

```json
{
  "owner": "alice",
  "attestation": "<the exact attestation JSON the server signed>",
  "attestationSignature": "<base64url 64-byte Ed25519 signature>"
}
```

Server checks, each a `400` refusal: the session verifies and its `sub` equals
the requested owner (an **exact, case-sensitive** string compare against the
registered display form, unlike the case-folded ident-owner check at publish
time); the session's `owner_id`/`auth_fingerprint` still match the owner's
current auth key; `ident` is `<owner>#<package>` whose owner part case-folds
to the session owner; `version` is 1–64 bytes; `signingFingerprint` is 64
lowercase hex characters; the owner has a current ident key. The request is
**recorded before the server signs** (`signing_requests`), so a stolen auth
session requesting attestations always leaves a trace; the transparency log
(plan-23-B) builds on this. The attestation JSON and signature domain are
specified in *signing*. The signature is made with the server's own keypair —
the key served by `GET /ident`.[[repository/src/server.rs:signing]][[repository/src/store.rs:record_signing_request]]

The client verifies the returned signature against its pinned `server.pub`
before using the attestation, and refuses an attestation that does not pin the
requested package or that names a different ident key than the machine
holds.[[repository/src/client.rs:request_attestation]][[src/cli/build.rs:load_build_signing_info]]

## Transparency Log — `/log/*`

The registry keeps an append-only, RFC 6962 Merkle-hashed record of every
state change (plan-23 §7): **every forgery path that remains — a compromised
server mis-binding names, a stolen auth session requesting attestations — is
forced to leave a signed entry in this log before the act.**

Entry kinds and the operations that append them (each appends **exactly
one** entry, inside the same transaction as the state change):

| kind | appended by |
| --- | --- |
| `register` | `/accounts/register` |
| `attestation` | `/signing` |
| `publish` | `/publish` |
| `link` | `/machines/link/fetch` (new auth key registered) |
| `revoke` | `/machines/revoke` |
| `rotate` | `/keys/rotate` |
| `reanchor` | `mfb-repo reanchor` (operator) |

[[repository/src/store.rs:append_log_tx]]

Hashing is RFC 6962: leaf hash `SHA-256(0x00 || payload)`, node hash
`SHA-256(0x01 || left || right)`; the empty tree hashes to `SHA-256("")`.
Entry indexes are dense and monotonic. The `/publish` response's `logEntry`
is the real `{ "index": <n>, "leafHash": "<hex>" }` of the publish
entry.[[repository/src/log.rs:root]]

* `GET /log/checkpoint` → `{"size", "rootHash": "<hex>", "signature"}` — the
  signed tree head. The signature is by the server key over
  `"mfb-log-checkpoint-v1\0" || size (LE u64) || root`.
  [[repository/src/server.rs:log_checkpoint]]
* `GET /log/proof/<index>[?size=N]` → `{"index", "size", "leafHash",
  "path": ["<hex>", …]}` — the RFC 6962 audit path for the entry in the tree
  of `size` (default: current). [[repository/src/server.rs:log_inclusion_proof]]
* `GET /log/consistency?from=M[&to=N]` → `{"from", "to", "path"}` — the
  consistency proof between two tree sizes. [[repository/src/server.rs:log_consistency_proof]]
* `GET /log/publish?ident=<i>&version=<v>` → the publish entry's
  `{"index", "leafHash"}`. [[repository/src/server.rs:log_publish_entry]]

Client behaviour: the last-seen checkpoint is pinned per repository
(`~/.mfb/<repo-hash>/checkpoint`, `"<size> <root-hex>"`). Every checkpoint
fetch verifies the signature under the pinned server key and enforces
append-only growth — a smaller size is a **ROLLBACK** and the same size with
a different root is a **FORK**, both hard `REGISTRY_LOG_ROLLBACK` errors that
never re-pin. `mfb pkg publish` refuses to upload before a verified
checkpoint fetch and, after publishing, verifies its own publish entry's
inclusion proof against a fresh checkpoint. `mfb pkg verify --proof`
additionally demands a verifying inclusion proof for each Verified
dependency's publish entry.
[[repository/src/client.rs:fetch_checkpoint]][[repository/src/client.rs:verify_publish_inclusion]]

## Ident Rotation — `/keys/rotate` + `GET /idents/<owner>`

Backs `mfb key rotate <owner>` (plan-23-B2, lost/stolen machine: the thief
holds a copy of the ident key, so revoke the machine's auth key **and**
rotate the ident). The new ident is chained to the old by an **old-ident
signature** over the rotation message, so consumers can follow the succession
without trusting the server:

```text
"mfb-repo-ident-rotate-v1\0" || owner || "\0" || oldFingerprint || "\0" || newPublicKey
```

[[repository/src/crypto.rs:ident_rotation_message]]

Request `RotateRequest` (session-authenticated — a rotation needs both the
old ident key and a live session):[[repository/src/server.rs:rotate_ident]]

```json
{
  "owner": "alice",
  "newIdentKey": "<base64url new ident public key>",
  "chainSignature": "<base64url OLD-ident signature over the rotation message>",
  "possessionProof": "<base64url NEW-ident registration proof (role ident)>",
  "sessionToken": "<JWT>"
}
```

The server verifies the chain signature under the **current** ident key and
the possession proof under the new key, marks the old ident `past`, inserts
the new ident as `current`, and records the signed chain link. Response:
`{"owner", "identFingerprint"}` (the new fingerprint). Subsequent
attestations name the new ident; an attestation issued before the rotation
names a `past` ident and is refused at publish (§3.4 step 5) — the client
refetches and rebuilds.[[repository/src/store.rs:rotate_ident]]

The client installs the new ident keypair locally on success. Other linked
machines still hold the old (now `past`) private key and must **re-link**
(`repo link`) — the new private key is never distributed automatically,
because rotations happen precisely when a machine holding the old key is no
longer trusted.[[repository/src/client.rs:rotate_ident]]

`GET /idents/<owner>` serves the current binding plus the chain, oldest link
first:[[repository/src/server.rs:ident_chain]]

```json
{
  "owner": "alice",
  "identKey": "<base64url current ident public key>",
  "identFingerprint": "<hex>",
  "chain": [
    { "oldKey": "<base64url>", "newKey": "<base64url>",
      "signature": "<base64url old-key signature>", "issued": <unix> }
  ]
}
```

Consumer pin-follow (`mfb pkg verify`): when an installed package names an
ident that differs from the pinned one, the client fetches the chain,
verifies every link signature locally, and — only if the pinned key chains to
the package's key — rewrites the `project.json` pin with a notice
(`notice: owner <o> rotated their ident; updated the pinned identKey ...`).
Packages published under the old ident still verify against the old pin
offline (proofs and attestations are statements of fact at `issued`). An
ident change with **no chain link** from the pin is the re-anchor (or
impersonation) case: a hard `PACKAGE_IDENT_REANCHORED` error telling the user
to verify out-of-band; the pin is never updated
silently.[[src/cli/pkg.rs:follow_rotated_pin]][[repository/src/client.rs:follow_ident_chain]]

### Re-anchor ceremony (operator action)

Total ident loss (all machines + backups) is survivable but deliberately not
painless: the registry **operator** — after out-of-band verification — binds
the name to a fresh ident with **no chain link**, using the server binary
directly (`mfb-repo reanchor --dbpath <db> --datapath <data> --owner <owner>
--ident-key <base64url>`); there is intentionally no HTTP route. Consumers
holding the old pin fail hard with the re-anchor warning above until they
re-verify and re-add.[[repository/src/store.rs:reanchor_ident]][[repository/src/main.rs:parse_reanchor_args]]

## Machine Link — `/machines/link` + `/machines/link/fetch`

Backs `mfb repo link` (plan-23 §3.2). Linked machines are **full equals**:
linking copies the account ident private key to the new machine, encrypted
under a one-time pairing code the server never sees. The relay blob is
single-use and short-TTL, and the server cannot read it.

Old machine (`repo link --start <owner>`, session-authenticated): generates
the pairing code (25 base32 characters in five groups, ~125 bits — displayed
to the user, never transmitted), seals `identPrv || identPub` with
ChaCha20-Poly1305 under an argon2id key derived from the code, and posts
`LinkStartRequest`:[[repository/src/server.rs:link_start]][[repository/src/crypto.rs:seal_pairing_blob]]

```json
{
  "owner": "alice",
  "lookup": "<hex sha256 of 'mfb-pairing-lookup-v1\\0' || code>",
  "blob": "<base64url: 12-byte nonce || ciphertext+tag>",
  "salt": "<base64url argon2id salt>",
  "sessionToken": "<JWT>"
}
```

Response: `{"owner": "alice", "expiresAt": <unix>}`. The blob row lives **600
seconds**, is deleted on first fetch, and expired rows are swept on insert. A
pending pairing with the same lookup yields `409` (`already pending`). The
`lookup` is a one-way hash of the code, so the relaying server can neither
read the blob nor derive its key.[[repository/src/store.rs:store_pairing_blob]]

New machine (`repo link <owner>`, types the code): generates its **own auth
keypair**, builds the role-separated registration proof, and posts
`LinkFetchRequest`:[[repository/src/server.rs:link_fetch]]

```json
{
  "owner": "alice",
  "lookup": "<hex, derived from the typed code>",
  "authKey": "<base64url new auth public key>",
  "proof": "<base64url signature over registration_message(auth)>"
}
```

Presenting the correct code-derived lookup **is** the pairing approval: the
server verifies the proof (before consuming the blob, so a malformed request
cannot burn a pending pairing), consumes the blob (single use — the stored
ciphertext is destroyed as it is handed out), registers the new auth key on
the account, and returns
`{"owner", "blob", "salt", "authFingerprint"}`. The client decrypts with the
typed code (a wrong code fails the AEAD tag), cross-checks the ident keypair,
and writes all four key files. An unknown, used, or expired lookup yields
`400 unknown, used, or expired pairing code`.[[repository/src/client.rs:link_fetch]][[repository/src/store.rs:take_pairing_blob]]

After a link the new machine is an equal: it opens its own sessions and runs
the full build/sign/publish path with no involvement from the old machine.

## Auth-Key Revocation — `/machines/revoke`

Backs `mfb machine revoke <owner> <auth-fingerprint>` (plan-23 §3.6, lost
machine). Authority is the **ident key alone** — an auth session must not
suffice (a thief with a stolen laptop has one), and no session is required (a
lapsed session must not block a revocation). Two round trips:

1. `POST /machines/revoke/challenge` `{"owner": "alice"}` → a standard
   `ChallengeResponse` issued against the owner's **ident**
   key.[[repository/src/store.rs:create_ident_challenge]]
2. `POST /machines/revoke`:[[repository/src/server.rs:revoke_machine]]

```json
{
  "challengeId": "<from step 1>",
  "authFingerprint": "<hex fingerprint of the key being revoked>",
  "identSignature": "<base64url signature over revocation_message>"
}
```

The signed bytes bind the challenge **and** the fingerprint being revoked, so
a signature can neither be replayed nor redirected at a different machine's
key:

```text
"mfb-repo-revoke-v1\0" || challengeId || "\0" || nonce || "\0" || authFingerprint
```

[[repository/src/crypto.rs:revocation_message]]

On success the key's status flips to `revoked` and **every session opened
with it is closed** in the same transaction: the revoked machine can no longer
log in (`mismatched local key fingerprint` at challenge time), and its
existing session tokens fail with `unknown session token`. Response:
`{"owner", "authFingerprint", "revoked": true}`. A fingerprint that names no
current auth key yields `400`.[[repository/src/store.rs:revoke_auth_key]]

## Package Artifact Requests

`/validate` and `/publish` share one request body, `PackageArtifactRequest`,
built from a `PackageArtifact` plus the session token. The `artifact` is the
whole `.mfp` file base64url-encoded.[[repository/src/client.rs:package_request]][[repository/src/server.rs:PackageArtifactRequest]]

```json
{
  "ident": "alice#mypkg",
  "version": "1.2.3",
  "artifact": "<base64url .mfp bytes>",
  "contentHash": "<hex>",
  "identFingerprint": "<hex>",
  "signingFingerprint": "<hex>",
  "sessionToken": "<JWT>"
}
```

### `/validate`

Returns `ValidatePackageResponse`:[[repository/src/server.rs:ValidatePackageResponse]]

```json
{
  "valid": true,
  "contentHash": "<hex>",
  "abiIndex": {},
  "diagnostics": []
}
```

The server verifies the session token, base64url-decodes and parses the `.mfp`
(container v1.0, hard), recomputes the content hash, and accumulates
`diagnostics`. `valid` is true iff `diagnostics` is empty. An artifact that
fails `.mfp` parsing is reported the same way — `200` with `valid: false`, an
**empty** `contentHash`, and the parse error as the sole diagnostic — not as an
HTTP error.

The checks run in the plan-23 §3.4 order, each with a distinct diagnostic:

0. Wire integrity — request `contentHash`/`ident`/`version` match the parsed
   package, and request `identFingerprint`/`signingFingerprint` match the
   fingerprints derived from the header keys.
1. Session/owner — the ident is `<owner>#<package>`; the session owner
   (case-folded) matches the ident owner; the session's
   `owner_id`/`auth_fingerprint` still match the owner's current auth key;
   `author` matches the registered display name; the package is
   Ed25519-signed (unsigned packages can never be published).
2. The attestation verifies under the **server's own key** and its
   `repoFingerprint` is ours.
3. `attestation.ident`/`version` pin this exact package (an attestation cannot
   be reused for another package or version).
4. `attestation.signingFingerprint` equals the fingerprint of the header
   `signingKey` — the package is signed by the key the server was told about.
5. `attestation.identFingerprint` equals the fingerprint of the header
   `identKey`, and that fingerprint matches the server's **current**
   name↔ident binding (a stale attestation from before an ident rotation is
   refused; the client refetches and rebuilds).
6. The proof verifies under the header `identKey` and its
   `owner`/`ident`/`version`/`identFingerprint`/`signingFingerprint` all match
   the header.
7. `packageBinaryHash` recomputes over the payload, and the package signature
   verifies under the header `signingKey` over the signed prefix.

Finally the `ident@version` must not already be
published.[[repository/src/server.rs:validate_package_request]]

The forgery property this enforces (plan-23 §2): producing a package that
passes requires the ident private key (step 6) **and** a live authenticated
session (steps 2–5 — attestations are only issued to sessions). Either
credential alone fails a step.

### `/publish`

Returns `PublishPackageResponse`:[[repository/src/server.rs:PublishPackageResponse]]

```json
{
  "ident": "alice#mypkg",
  "version": "1.2.3",
  "hash": "<hex>",
  "publishedAt": 1700000000,
  "state": "available",
  "blobStored": true,
  "logEntry": "publish:<uuid>"
}
```

`state` is the constant `available` — the state recorded on the new
`package_versions` row.[[repository/src/store.rs:publish_package_version]]

`/publish` re-runs the **full validation** internally first; if `valid` is false
it returns `400 package validation failed: <diagnostics joined by "; ">`. On
success it writes the artifact blob to `<hash>.mfp` (only if not already
present — `blobStored` reflects whether a new write occurred) and records the
package version.[[repository/src/server.rs:publish_package]]

## `pkg publish`: Validate-then-Publish

`mfb pkg publish <owner_name> <package>` is a build-then-two-call sequence:[[src/cli/pkg.rs:publish_package_project]]

1. Validate the package `project.json`; require kind `package`.
2. `build --sign <owner>` in `Validate` mode to produce the signed `<name>.mfp`.
3. Read and re-parse the `.mfp`; compute the content hash and assemble a
   `PackageArtifact` from the package metadata.
4. Call **`/validate`** (`client::validate_package`); print the report; abort
   with `package validation failed` if `valid` is false.
5. Call **`/publish`** (`client::publish_package`); print `Published
   <ident>@<version> as <hash>`.

The client never publishes a package that did not validate; the server enforces
the same invariant independently by re-validating inside `/publish`.[[repository/src/client.rs:validate_package]][[repository/src/client.rs:publish_package]]

## See Also

* ./mfb spec package-manager signing — keypair generation, signing domains, and the package signature this protocol verifies
* ./mfb spec package-manager key-store — on-disk keypair and session-token storage (the rollback target and session source)
* ./mfb spec package-manager owner-names — owner-name validation and case folding applied before every request
* ./mfb spec package container-format — the `.mfp` bytes carried in `artifact` and parsed during validation
* ./mfb spec tooling cli-reference — `repo register`/`repo auth`/`pkg publish` command surface and exit codes
* ./mfb spec tooling project-manifest — the `project.json` that `pkg publish` reads before building
