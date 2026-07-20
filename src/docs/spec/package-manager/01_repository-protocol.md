# Repository Protocol

The wire protocol between the `mfb` client and a repository (registry) service.
It backs five commands: `mfb repo register`, `mfb repo auth`, `mfb pkg publish`,
`mfb build --sign`, and — for registry idents — `mfb pkg add`. `mfb pkg add`
accepts either a `file://…​.mfp` URL (copied into `packages/` locally, no
protocol) or an `<owner>#<package>[@version]` ident, which resolves `GET
/index`, downloads `GET /blob/<hash>`, and runs the full verification chain before
installing.[[src/cli/pkg.rs:add_package]][[src/manifest/package.rs:package_file_url_path]]
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
Every request URL is `format!("{base}{path}")` with any trailing slashes
trimmed from the base, so `MFB_REPO_URL=https://repo.example/` and the path
`/publish` produce `https://repo.example/publish`.[[repository/src/client.rs:post_json]]

`post_json` is the single transport helper. On a 2xx it deserializes the typed
response. On a non-success status it reads the body, and if the body parses as
`{"error": "..."}` it returns that string verbatim; otherwise it returns
`repository request failed with status <status>: <body>`. A connection failure
returns `failed to connect to repository service: <err>`.[[repository/src/client.rs:post_json]][[repository/src/server.rs:ErrorResponse]]

The reference server ships as the `mfb-repo` binary: `mfb-repo --dbpath
<db_path> --datapath <data_path> [--listen <addr:port>] [--s3-endpoint <url>]`.
It listens on `127.0.0.1:7777` by default, prints `MFB_REPO_LISTEN=<actual
addr>` once bound, and keeps its state as a SQLite database at `<db_path>` plus
package blobs at `<data_path>`. `<data_path>` is either a local directory or an
`s3://<bucket>/<prefix>` URL selecting the S3 blob backend (built with `cargo
build -p mfb_repository --features s3`); `--s3-endpoint` overrides the endpoint
for S3-compatible stores (MinIO/R2/Ceph) and is only valid with an `s3://` data
path. The metadata database always stays on local disk. On first run it
generates its own Ed25519 **server keypair** — the only private key the server
ever holds; the private half never appears in any
response.[[repository/src/main.rs:parse_args]][[repository/src/server.rs:serve]][[repository/src/store.rs:open_repository]][[repository/src/blobstore.rs:BlobBackend]]

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
| `/index/<owner>#<package>` | GET | none | — | `IndexResponse` | `pkg add` (registry) |
| `/blob/<hash>` | GET | none | — | raw blob bytes (or `302` to a presigned URL in S3 mode) | `pkg add` (registry) |
| `/blob/<hash>` | HEAD | none | — | `200` if present, `404` otherwise (no body) | `pkg publish` (dedup probe) |
| `/blob/<hash>` | PUT | `Authorization: Bearer` session token | raw bytes (`application/octet-stream`) | `201` stored, `200` already present | `pkg publish` (vendor blobs) |
| `/release-state` | POST | session + ident signature | `ReleaseStateRequest` | `ReleaseStateResponse` | `pkg release-state` |
| `/orgs/members` | POST | session + ident signature | `OrgMemberRequest` | `OrgMemberResponse` | `org grant`/`org remove` |
| `/tokens` | POST | session + ident signature | `TokenIssueRequest` | `TokenIssueResponse` | `token issue` |
| `/tokens/revoke` | POST | session + ident signature | `TokenRevokeRequest` | `TokenRevokeResponse` | `token revoke` |
| `/packages/transfer/offer` | POST | session + ident signature | `TransferOfferRequest` | `TransferResponse` | `pkg transfer` |
| `/packages/transfer/accept` | POST | session + ident signature | `TransferAcceptRequest` | `TransferResponse` | `pkg transfer-accept` |
| `/root.json` | GET | none | — | `RootResponse` | `repo trust` |
| `/snapshot.json` | GET | none | — | `SignedMetadataResponse` | `repo trust` |
| `/timestamp.json` | GET | none | — | `SignedMetadataResponse` | `repo trust` |
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

## Operational Hardening

Independent of the protocol, the reference server applies several operational
safeguards:

- **SQLite WAL + busy timeout** — readers no longer block on the writer at the
  database level, and brief writer contention waits rather than failing, so
  concurrent publishes and reads do not serialize behind one global write
  lock.[[repository/src/store.rs:open_repository]]
- **Background reaping** — a timer sweeps expired challenges, expired sessions,
  and expired pairing blobs, and prunes the rate-limiter map, so stale rows
  never accumulate. `pairing_blobs` are machine-pairing auth ephemera, **not**
  package content: nothing automatic ever deletes a package blob. Reclaiming
  those is the operator-triggered
  [`mfb-repo gc`](#blob-garbage-collection--mfb-repo-gc-operator-action).[[repository/src/store.rs:reap_expired]]
- **Rate limiting** — an in-memory sliding-window limiter caps abusive bursts
  on `register`/`challenge`/`login`/`signing` (a `429` when exceeded), keeping
  the transparency log spam-free.[[repository/src/server.rs:RateLimiter]]
- **Request-size cap** — inline base64 artifacts are bounded (64 MiB), so a
  single upload cannot exhaust server memory.[[repository/src/server.rs:MAX_BODY_BYTES]]
- **Typosquat warning** — `POST /publish` returns warn-only `warnings` naming
  existing idents within edit distance 1 of the published one; it never blocks
  the publish.[[repository/src/store.rs:typosquat_candidates]]

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
auth key per linked machine, so auth-key resolution is always
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

Used by `build --sign`: an authenticated build pre-registers
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
 builds on this. The attestation JSON and signature domain are
specified in *signing*. The signature is made with the server's own keypair —
the key served by `GET /ident`.[[repository/src/server.rs:signing]][[repository/src/store.rs:record_signing_request]]

The client verifies the returned signature against its pinned `server.pub`
before using the attestation, and refuses an attestation that does not pin the
requested package or that names a different ident key than the machine
holds.[[repository/src/client.rs:request_attestation]][[src/cli/build.rs:load_build_signing_info]]

## Transparency Log — `/log/*`

The registry keeps an append-only, RFC 6962 Merkle-hashed record of every
state change: **every forgery path that remains — a compromised
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
| `org-role` | org role grant/remove |
| `token-issue` | token issue |
| `token-revoke` | token revoke |
| `transfer-offer` | package transfer offer |
| `transfer-accept` | package transfer accept |
| `release-state` | release-state change |

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

Backs `mfb key rotate <owner>` (lost/stolen machine: the thief
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

Backs `mfb repo link`. Linked machines are **full equals**:
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
pending pairing with the same lookup yields `400` (`already pending`; only
`already in use` and `reused challenge` are elevated to `409`). The
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

Backs `mfb machine revoke <owner> <auth-fingerprint>` (lost
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

## Install Path — `/index` and `/blob`

The install path makes a published package retrievable. `mfb pkg
add <owner>#<package>[@version]` resolves the index, pins the ident, downloads
the blob, and runs the verification chain — nothing is installed
until every link verifies.[[src/cli/pkg.rs:add_package_from_registry]]

### Package Index — `GET /index/<owner>#<package>`

The `#` is percent-encoded (`%23`) in the request path. Returns the published
version list plus the owner's current ident key and a **server-signed name
binding**, so a first `add` pins the ident against a registry-authenticated
anchor rather than a bare field. An unknown owner or malformed ident yields
`400`.[[repository/src/server.rs:package_index]]

```json
{
  "ident": "alice#toolbox",
  "owner": "alice",
  "identKey": "ed25519:<base64url current ident public key>",
  "identFingerprint": "<hex>",
  "nameBindingSignature": "<base64url server signature>",
  "serverFingerprint": "<hex>",
  "versions": [
    { "version": "1.2.3", "hash": "<hex>", "publishedAt": 1700000000,
      "state": "available", "abiIndex": {},
      "logEntry": { "index": 7, "leafHash": "<hex>" } }
  ]
}
```

The name binding is a server signature over `"mfb-repo-name-binding-v1\0" ||
owner || "\0" || identFingerprint` (signing domain
`mfb-repo-name-binding-v1`).[[repository/src/crypto.rs:name_binding_message]]
The client verifies it under the pinned `server.pub`, cross-checks that
`identFingerprint` is the fingerprint of `identKey`, and only then trusts the
key as the anchor to pin.[[repository/src/client.rs:fetch_index]] `abiIndex` is
currently an empty object; release `state` values other than `available` may
appear in later revisions. There are **no** key-rotation/window fields: one-off
signing keys have no status.

The client picks the requested version (any non-`blocked`/`legal-tombstoned`
state) or, for a floating add, the newest version whose state is `available`
or `deprecated`.[[src/cli/pkg.rs:select_index_version]]

### Blob — `GET`/`HEAD`/`PUT /blob/<hash>`

Blobs are content-addressed by SHA-256 and come in two **kinds**, which select
the stored object's name so an operator listing the datapath sees honest
filenames:

| kind | stored as | holds |
| --- | --- | --- |
| `package` | `<hash>.mfp` | a published package artifact |
| `native` | `<hash>.bin` | one vendored native library file (plan-48-A) |

The kind is recorded in `package_blobs.kind` (defaulting to `package`, so blobs
predating the column need no migration), and `GET`/`HEAD` learn it from that
primary-key lookup — which also lets an unknown hash `404` without touching the
storage backend. Both kinds are served by the *same* `GET` path with the same
shape.[[repository/src/blobstore.rs:BlobKind]]

Serves the content-addressed blob with immutable, long-cache
headers (`Cache-Control: public, max-age=31536000, immutable`). The hash must be
64 lowercase hex characters (else `400`); an absent blob is `404`. With the
local backend the server streams the bytes, recomputing the SHA-256 and refusing
to serve it if it does not match the hash (`500`, blob-store corruption
defense). With the S3 backend the server instead answers `302` with a `Location`
of a short-lived presigned URL (`Cache-Control: no-store`), so the bytes never
transit the app server and the bucket can stay private; the corruption re-check
moves to the client. Either way the client independently re-checks the
downloaded bytes against the requested
hash.[[repository/src/server.rs:package_blob]][[repository/src/client.rs:fetch_blob]][[repository/src/blobstore.rs:BlobStore]]

The publish path stages the blob, commits the `package_versions` row, and only
then promotes it to servable, so a failed transaction leaves no orphan blob and
a served blob always has a committed version row. The local backend stages to a
temp file and promotes with an atomic rename; the S3 backend PUTs the immutable
content-addressed object (unreachable until the committed index row exposes its
hash) and deletes it on
failure.[[repository/src/server.rs:publish_package]][[repository/src/blobstore.rs:BlobStore]]

#### `HEAD /blob/<hash>` — the dedup probe

`200` if a servable blob exists, `404` otherwise; no body and no auth. It
discloses only whether a content hash the caller **already possesses** is
present — exactly what `GET` already reveals — and the design already relies on
hashes being unguessable. A publisher uses it to skip re-uploading an unchanged
library on every version bump.[[repository/src/server.rs:head_blob]]

#### `PUT /blob/<hash>` — uploading a vendored native library

Stores raw native-library bytes as a `native` (`<hash>.bin`) blob.

- **Auth — the one header-borne credential.** Every other authenticated route
  carries its session JWT in a `sessionToken` **body field**, which a raw-body
  `PUT` cannot do. This route alone takes `Authorization: Bearer <token>`,
  verified with the same session check (`exp` *and* a live `jti` session row).
  This is a deliberate, documented exception to the body-field convention, not
  an oversight.
- **Body.** Raw bytes, `application/octet-stream`, subject to the shared 64 MiB
  body cap (`413` above it). Raw bytes lift the effective ceiling from ~48 MiB
  (base64) to a full 64 MiB. Streaming and presigned `PUT` are deliberately not
  offered: the upload is proxied so the server can verify the hash before the
  bytes land.
- **Verification before storage.** The server computes `sha256(body)`; if it
  does not equal the `<hash>` in the path it answers `400` and stores
  **nothing**. The store is content-addressed, so this is the invariant that
  keeps it honest.
- **Idempotent.** An already-present blob answers `200` without re-staging; a
  fresh store answers `201`. Racing uploads of identical bytes are harmless.
- **Ordering.** stage → `package_blobs` row → promote, aborting on failure —
  the same protocol publish uses, preserving the "no servable orphan"
  invariant.
- **Rate limited** per owner, alongside the `/validate` and `/publish` caps. A
  blob accepted here is not referenced by anything until a publish names it, so
  without a cap an authenticated publisher could fill the datapath faster than
  an operator could sweep it. Reclaiming what an abandoned upload leaves behind
  is [`mfb-repo gc`](#blob-garbage-collection--mfb-repo-gc-operator-action),
  which runs on demand and never automatically.[[repository/src/server.rs:put_blob]]

#### Publish refuses a dangling vendor hash

A package's section-10 `NATIVE_LIBRARY_TABLE` names each vendored library by
SHA-256. `/validate` and `/publish` parse that table and require **every**
`vendor` locator's blob to already exist; a missing one is reported as a
validation diagnostic and the publish is refused (`400`), naming the hash, the
logical library, and the source filename. Combined with the client uploading
blobs **before** the `.mfp`, this makes "a published package never references a
blob that does not exist" a guarantee rather than a convention.

The registry needs no new trust to do this: section 10 lives inside the payload
that `packageBinaryHash` welds to the package signature, so the vendor hashes
are transitively authenticated and a blob fetched by one of them needs only to
be re-hashed — the same argument the registry already makes for the ABI index.
The publish transaction also records the version→blob edges in
`package_version_blobs`; those edges are what
[`mfb-repo gc`](#blob-garbage-collection--mfb-repo-gc-operator-action) reads to
tell a vendored library that is still in use from one an abandoned upload left
behind.[[repository/src/abi.rs:parse_vendor_blobs]][[repository/src/server.rs:validate_package_request]]

Section 10 is bounded before any of that work happens: a table declaring more
than **1024 entries** or more than **4096 locators in total** is rejected as
malformed, and the existence probes run over the *distinct* hashes rather than
once per locator. Both counts are raw `u32`s in an attacker-supplied payload and
each probe is a blob-store round trip (a `head_object` on the hosted backend), so
without those bounds a single `/validate` or `/publish` from any self-registered
owner could fan out to roughly a million backend operations. The limits sit far
above any real table — one entry per logical library, one locator per supported
platform triple.[[repository/src/abi.rs:read_native_vendor_locators]]

### Vendor blobs on install

A binding that vendors native libraries carries only their **hashes** in its
`.mfp`, never their bytes. After `pkg add`/`pkg install` has fetched and fully
verified the `.mfp` — and therefore trusts section 10 — it downloads every
`vendor` blob the table names via the same `GET /blob/<hash>`, which re-hashes
the bytes against the content address before returning them.

Files land at **`<project>/packages/<name>.vendor/<source>`**, one directory per
package, written with the same stage-verify-rename discipline as the `.mfp`
itself (an exclusively created `.part` file, then a rename — so a pre-planted
symlink at the destination is replaced, never written through). This is
deliberately *not* `<project>/vendor/`, which belongs to the consumer's own
`libraries` section and must never be overwritten by an imported package, and
deliberately per-package, because two packages may each vendor a same-named file
with different bytes.

`source` arrives from the `.mfp` and is untrusted: it is re-validated as a bare
filename (no separators, no `..`, no NUL) before it becomes a path. A blob the
registry does not have is `PACKAGE_VENDOR_BLOB_MISSING` (`6-605-0010`); one whose
bytes do not match the signed table is `PACKAGE_VENDOR_BLOB_HASH_MISMATCH`
(`6-605-0011`). Either is fatal and leaves nothing usable on disk. Every vendor
blob in the table is downloaded — not just the host target's — so a later
cross-compile and an offline build both work.

A `pkg add file://…` is a local copy with no registry to fetch from, so a package
that vendors native libraries is refused outright rather than installed in a
silently unusable state.[[src/cli/pkg.rs:install_vendor_blobs]]

### Blob garbage collection — `mfb-repo gc` (operator action)

```
mfb-repo gc --dbpath <db> --datapath <data> [--s3-endpoint <url>]
            [--grace-hours <n>] [--delete] [--json]
```

Reclaims blobs that **no live package version references**. Those exist because
`PUT /blob` accepts a blob before anything names it: a publisher who uploads and
then abandons the publish — network failure, failed validation, `^C` — leaves
bytes nothing will ever reference. It is a **dry run** unless `--delete` is
given, and it is never automatic: nothing in the server's periodic reaper (which
expires auth challenges, sessions, and pairing blobs) touches package content.
Deleting package content is irreversible and has no "it will be re-created"
fallback, so an operator decides when it runs and can always see what it would
do first.[[repository/src/gc.rs:run]][[repository/src/main.rs:parse_gc_args]]

Reachability is **recomputed from the tables on every run**, never refcounted: a
refcount must be maintained transactionally at every mutation site and drifts
permanently on any crash between its two writes — and a drifted one either leaks
forever or deletes a live blob. The reachable set is

```sql
SELECT hash FROM package_versions UNION SELECT hash FROM package_version_blobs;
```

and **both halves are required**. `package_versions.hash` is the `.mfp` artifact
itself, which is not a vendor blob and so never appears in
`package_version_blobs`; omitting that half would report every published package
as collectable.[[repository/src/store.rs:unreachable_blobs]]

Three rules bound what can ever be deleted:

- **Nothing a live version references**, including a **yanked** one. Yanking is a
  "do not resolve this by default" signal, not a deletion — existing lockfiles
  pin the hash and must keep installing. Only a version row that no longer
  exists releases its blobs.
- **Nothing younger than the grace period** (default 24h on
  `package_blobs.created_at`, `--grace-hours`). There is no lock between
  `PUT /blob` and `POST /publish`, so a publisher's uploaded blobs genuinely
  *are* unreachable until the publish lands; the grace window is what makes the
  sweep safe against a publish in flight, in place of a lock, a lease, or a
  two-phase protocol. **`--grace-hours 0` is refused**, because it removes that
  protection entirely.
- **Nothing outside `package_blobs`.** Auth challenges, sessions, and pairing
  blobs belong to the periodic reaper and are untouched here.

Deletion removes the **backing object first, then the `package_blobs` row** —
the inverse of the publish path's stage → row → promote. A crash in between then
leaves a row pointing at a missing object, which the next run re-lists and
re-collects idempotently, and whose `GET` already 404s correctly because it was
unreachable. The other order would leave an object with no row: invisible to
every future run and unreclaimable forever. Neither ordering is atomic; this one
self-heals.[[repository/src/blobstore.rs:BlobStore]]

Sizes are stat'd from the backing store per candidate (`metadata` locally,
`head_object` on S3) rather than stored in a column, so the report shows real
reclaimed bytes with no schema change and no back-fill hole. `--json` adds the
reachable-side total, so an operator can see what fraction of the store is
garbage. Both backends are supported; with an `s3://` datapath the metadata
database still lives on local disk. A per-blob failure is reported and the sweep
continues, but the command exits nonzero.

A registry that never runs `gc` behaves exactly as it always has — which is the
right default for an existing deployment. To a client, a collected blob is
visible only as a `GET /blob/<hash>` `404`, and only for a hash no live version
references.

## Release States — `POST /release-state`

A maintainer moves a published version between release states.
A state is registry metadata *about* a version — the blob and its signatures are
never touched. The maintainer states are `available`, `deprecated`, and
`yanked`; `blocked` and `legal-tombstoned` are registry-operator states and are
refused here.[[repository/src/server.rs:release_state]]

```json
{
  "owner": "alice",
  "ident": "alice#toolbox",
  "version": "1.2.3",
  "state": "deprecated",
  "sessionToken": "<JWT>",
  "identSignature": "<base64url ident signature>"
}
```

Authority is the **ident key**: an auth session alone can never change a
release state. The request carries both a live session (the machine is logged
in) and an ident signature over `release_state_message(ident, version, state)`
(signing domain `mfb-repo-release-state-v1`), verified under the owner's current
ident key. The change updates the version's state, records the transition with a
timestamp, and appends one `release-state` entry to the transparency log — all
in one transaction. The response echoes `{ident, version, state, logEntry}`.

`GET /index` serves the current state per version. Resolution eligibility
: `available`/`deprecated` are install-eligible, `yanked` is
selectable only by an exact pin, and `blocked`/`legal-tombstoned` are excluded
entirely.[[src/cli/resolve.rs:select_node]][[src/cli/pkg.rs:select_index_version]]

## Accounts — Orgs, Publish Tokens, Transfers

The accounts surface obeys two invariants: **publishing always
requires the ident key** (no feature creates a credential that can publish
without it), and **every account mutation is ident-authorized and logged** — an
auth session alone may read and request attestations, never change account
state. Each endpoint below carries a live session (liveness) *and* an ident
signature (authority).

**Orgs.** An org is an account with its own ident keypair; its ident is shared
among member machines via the machine-link flow, so an org package's proof is
org-ident-signed. `POST /orgs/members` grants or removes a member role
(`owner`/`admin`/`publisher`), authorized by the grantor's ident signature over
`org_role_message(org, member, role)`. The grantor must be the org itself (the
bootstrap grant) or an existing owner/admin member; the role is
logged.[[repository/src/server.rs:org_members]]

**Publish tokens.** A token is a **scoped auth key** — a CI credential that is a
linked machine whose auth key is scoped and short-lived. `POST /tokens` issues
one (owner-ident-signed), registering an auth key plus a `scope`
(`<owner>#<package>` or `<owner>#*`) and TTL. At `/signing` the token may request
attestations **only within its scope and only until it expires**; it can never
bypass the ident-proof requirement (the CI box still needs the org/owner ident
to sign the package proof). `POST /tokens/revoke` revokes it and closes its
sessions. Issue, use (via the log), and revoke are all
logged.[[repository/src/server.rs:issue_token]][[repository/src/store.rs:publish_token_for_key]]

**Ownership transfer.** Two-sided: `POST /packages/transfer/offer` (current
owner's ident signs `transfer_offer_message(ident, from, to)`) then `POST
/packages/transfer/accept` (recipient's ident signs
`transfer_accept_message(ident, to)`). The server re-binds the package to the
new owner and logs both halves; already-published versions keep verifying
against the old ident's proofs/attestations (issued-only facts), while new
versions publish under the new owner's ident.[[repository/src/store.rs:accept_transfer]]

## Signed Metadata — `/root.json`, `/snapshot.json`, `/timestamp.json`

A TUF-like signed-metadata layer sits on top of the
pinned-server-key anchor. An **offline root key** delegates three online keys —
the server (attestation) key, a snapshot key, and a timestamp key — so the
server key can be renewed under root authority, and a mirror or MITM cannot
serve a stale or partial index undetected. Each endpoint returns the exact
signed JSON string plus a signature the client verifies over those
bytes.[[repository/src/server.rs:root_metadata]]

`root.json` (root-signed, domain `mfb-repo-root-v1`): `{type, registryId,
version, expires, serverKey, snapshotKey, timestampKey}`. The root private key
is generated by an **operator ceremony** and never touches the serving host:
`mfb-repo init-root --dbpath <db> --datapath <data> --registry-id <id>
[--expires-days <n>]` generates the root + online keys, signs `root.json`, and
prints the root private key for the operator to store offline. The
`repoFingerprint` pin becomes "fingerprint of the server key **delegated by**
the pinned root".[[repository/src/store.rs:init_registry_root]][[repository/src/main.rs:parse_init_root_args]]

`snapshot.json` (snapshot-signed, `mfb-repo-snapshot-v1`): `{type, registryId,
version, expires, indexHash, checkpoint:{size, rootHash}}`, where `indexHash` is
a canonical hash of every served `(ident, version, hash, state)` tuple and the
version is the transparency-log size (monotonic).

`timestamp.json` (timestamp-signed, `mfb-repo-timestamp-v1`): `{type,
registryId, version, expires, snapshotVersion, indexHash}` — short-lived,
refreshed on demand, pinning the current snapshot version + index hash.

`mfb repo trust <registry-id> <root-fingerprint>` pins the registry id + root
fingerprint, fetches all three documents, and verifies the chain: the root key
matches the pinned fingerprint; every signature verifies under its delegated
key; the registry id matches; nothing is expired; the timestamp and snapshot
agree on version + index hash; the version has not rolled back below the pinned
one; and the pinned `server.pub` **is** the root-delegated server key. Once a
root is pinned, `mfb pkg add`/`install` re-verify the chain (and advance the
pinned snapshot version) before trusting the index — the layer is opt-in on top
of the pinned-server-key anchor.[[repository/src/client.rs:verify_registry_metadata]]

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

The checks run in the order, each with a distinct diagnostic:

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

The forgery property this enforces: producing a package that
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
  "logEntry": { "index": <n>, "leafHash": "<hex>" },
  "warnings": []
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
4. **Upload vendor blobs.** Read the `.mfp`'s section-10 table; for each
   `vendor` locator `HEAD /blob/<hash>` and, when absent, `PUT` the bytes of
   `<project root>/vendor/<source>`. The section-10 hash **is** the upload key,
   so the file is not re-hashed here — a second computation is a second chance
   to disagree. Distinct hashes are uploaded at most once, so a library shared
   across platforms or unchanged across versions transfers
   once.[[src/cli/pkg.rs:upload_vendor_blobs]]
5. Call **`/validate`** (`client::validate_package`); print the report; abort
   with `package validation failed` if `valid` is false.
6. Call **`/publish`** (`client::publish_package`); print `Published
   <ident>@<version> as <hash>`.

The client never publishes a package that did not validate; the server enforces
the same invariant independently by re-validating inside `/publish`.[[repository/src/client.rs:validate_package]][[repository/src/client.rs:publish_package]]

**Blobs go up before the `.mfp`, and come down after it.** Both directions
follow from the same fact: the `.mfp` is the only thing that names the blobs, so
it is the commit point. Publishing blobs first means a successful publish never
leaves a section-10 hash dangling (the registry enforces the converse); a
failure after the blobs but before the publish leaves unreferenced blobs —
garbage, not a broken package. On install the `.mfp` is fetched and **fully
verified** first, so section 10 is trusted before any hash in it is used;
fetching blobs first would mean acting on attacker-supplied hashes.

## See Also

* ./mfb spec package-manager signing — keypair generation, signing domains, and the package signature this protocol verifies
* ./mfb spec package-manager key-store — on-disk keypair and session-token storage (the rollback target and session source)
* ./mfb spec package-manager owner-names — owner-name validation and case folding applied before every request
* ./mfb spec package container-format — the `.mfp` bytes carried in `artifact` and parsed during validation
* ./mfb spec tooling cli-reference — `repo register`/`repo auth`/`pkg publish` command surface and exit codes
* ./mfb spec tooling project-manifest — the `project.json` that `pkg publish` reads before building
