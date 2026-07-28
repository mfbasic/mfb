# plan-10: Package Registry Completion — overview

Last updated: 2026-07-04
Overall Effort: x-large

**Baseline: [plan-23](plan-23-key-trust-model.md) is assumed COMPLETE before any
plan-10 sub-plan starts.** plan-23 delivers the key/trust model this plan was
originally missing: per-account ident keypair (client-held, copied to linked
machines), per-machine auth keys, one-off per-package signing keys, the
ident-signed proof + server-signed attestation (`POST /signing`), the v2 `.mfp`
header, the publish/install verification chains, ident rotation/re-anchor, and
the transparency log (plan-23-B). Everything below is scoped as the delta ON TOP
of that baseline; superseded items are marked `→ plan-23`.

This document scopes the work required to take the repository service from its
plan-23 base to the full design in `mfb spec package-manager`
(`src/docs/spec/package-manager/`). It is the successor planning document to the
base pass (now folded into `mfb spec package-manager`), which deliberately
limited the first pass to account registration and authentication.

**This is a split plan** (Overall Effort x-large). The work is split *by effort* into four
small/medium sub-plans under a shared `plan-10` number; this file is the **overview/index** holding
the shared material (summary, gap analysis, sequencing, cross-cutting decisions, out-of-scope). The
sub-plan documents are listed in §3, each bundling several phases. As each sub-plan lands, remove
its lettered document; the whole `plan-10` set is gone once the last one is done.

It complements:

- `mfb spec package-manager` (`src/docs/spec/package-manager/`) — the full
  registry protocol, key store, signing, and owner-name design (authoritative;
  also holds the completed base pass)
- `mfb spec package container-format` (`src/docs/spec/package/`) — `.mfp`
  container + content hash
- `mfb spec tooling project-manifest` (`src/docs/spec/tooling/01_project-manifest.md`)
  — `project.json` + `mfb pkg` CLI surface
- `mfb spec tooling lockfile` (`src/docs/spec/tooling/03_lockfile.md`) —
  `mfb.lock` schema

This is a planning document. It records what exists today, the gaps against the
full design, and a phased implementation sequence with validation criteria for
each phase.

---

## 1. Summary

The base pass shipped a working but narrow slice:

- `mfb-repo --path <repo>` HTTP service with `meta.db` (SQLite) + `packages/`.
- `POST /accounts/register`, `POST /auth/challenge`, `POST /auth/login`,
  `GET /health`, plus `POST /keys/signing`, `POST /validate`, `POST /publish`.
- Ed25519 proof-of-possession registration, challenge/response login, HS256 JWT
  sessions (1h, tracked by `jti`).
- Signed-package publish that parses the `.mfp` header, recomputes the content
  hash, verifies the signature, enforces ident/version/owner binding, writes the
  content-addressed blob, and records `packages` / `package_versions` /
  `package_blobs`.
- Client CLI: `mfb repo register|auth`, `mfb pkg publish`, `mfb build --sign`,
  `mfb pkg add` (today only `file://…​.mfp`), `mfb pkg info|verify`.

On top of that base, **plan-23** (assumed complete) replaces the single-key
model wholesale: two-key register, `POST /signing` (replacing `/keys/signing`),
the container v1.0 header with proof + attestation, the publish/install
verification chains, machine linking, ident lifecycle, and the transparency
log — plan-10 no longer contains any key-model work.

The largest remaining gap is that **nothing can be installed**. There is no
`GET /blob/<hash>`, no `GET /index/<owner>#<package>`, no resolver, and no
`mfb.lock`. A package can be published but never resolved or downloaded through
the registry (plan-10-A, plan-10-B).

The second is **resolution**: no ABI index, resolver, or lockfile (plan-10-B).
The third is the remaining **trust metadata**: release states and the signed
root/snapshot/timestamp chain (plan-10-C) — both now building on plan-23's
server key and transparency log rather than replacing stubs.

The phases below are ordered so each builds on the last and each ends at a
testable milestone.

---

## 2. Gap Analysis

Organized against `repository.md`. Status: ✅ done · ◑ partial · ❌ missing.

### 2.1 Identity & naming (§1)

| Item | Status | Notes |
|---|---|---|
| Owner-handle validation, `std` reserved | ✅ | `validation.rs` |
| `<owner>#<package>` ident on publish | ◑ | Parsed/enforced at publish; no package-slug vs header-`name` separation |
| `std#*` resolution short-circuit | ❌ | Resolver does not exist |
| Identity permanence / visible tombstones | ❌ | No tombstone records; deletion path undefined |
| Typosquat warn-only check at publish | ❌ | |
| `mfb pkg add <owner>#pkg[@ver][--pin]`, name search | ❌ | `add` only accepts `file://` |
| `mfb pkg install` (lock-driven) | ❌ | |

### 2.2 Accounts (§2)

| Item | Status | Notes |
|---|---|---|
| Open key-based registration | → plan-23 | ident + auth keypairs, role-separated proofs |
| Key roles (auth/ident/signing) | → plan-23 | per-machine auth, per-account ident, one-off signing |
| Machine linking / auth-key add + revoke | → plan-23 | plan-23-B |
| Key-rotation recovery | → plan-23 | ident chain rotation + re-anchor ceremony (plan-23-B) |
| Orgs (members, owner/admin/publisher roles) | ❌ | plan-10-D; org model must fit plan-23 keys (see D1) |
| Publish tokens (delegated CI creds, TTL, revoke) | ❌ | plan-10-D; reshaped: tokens = scoped auth keys, CI is a linked machine |
| Ownership transfer (two-sided, signed, logged) | ❌ | plan-10-D; both parties sign with their ident keys |

### 2.3 Trust & signing (§3)

| Item | Status | Notes |
|---|---|---|
| Proof-of-possession at registration | → plan-23 | one proof per role, domain-separated |
| Distinct ident vs signing authority | → plan-23 | ident-signed proof + one-off signing key |
| Signing-key rotation lifecycle | obsolete | one-off key per package subsumes rotation; no key windows |
| Past-key install verification (`publishedAt < rotatedAt`) | obsolete | proofs/attestations are `issued`-only facts; packages verify forever |
| Ident rotation + revocation | → plan-23 | chain rotation, auth revoke, re-anchor (plan-23-B) |
| Transparency log (Merkle, checkpoints) | → plan-23 | plan-23-B (subsumes old C1) |
| Inclusion + consistency proofs | → plan-23 | plan-23-B |
| Offline root + `/root.json` | ❌ | plan-10-C; binds plan-23's server (attestation) key |
| Online snapshot/timestamp + `/snapshot.json` `/timestamp.json` | ❌ | plan-10-C |
| `signatureType=1` enforced; unsigned rejected | → plan-23 | v2 header chain (proof + attestation + prefix signature) |

### 2.4 Resolution & lockfile (§4, §8)

| Item | Status | Notes |
|---|---|---|
| `ABI_INDEX` computed by compiler, embedded in `.mfp` | ❌ | publish returns `abiIndex: {}` |
| `ABI_INDEX` stored + served in index | ❌ | |
| Per-symbol superset substitution resolver | ❌ | |
| Diamond conflict diagnostics | ❌ | |
| `mfb.lock` write + byte-identical re-resolve | ❌ | |
| `mfb pkg check-abi` | ❌ | |

### 2.5 Registry API & storage (§5)

| Endpoint | Status |
|---|---|
| `/accounts/register` `/auth/challenge` `/auth/login` `/health` | → plan-23 (two-key register; per-machine auth) |
| `POST /signing` (attestation issuance) | → plan-23 |
| `/validate` `/publish` | → plan-23 (full §3.4 check chain; ABI still missing → plan-10-B) |
| key lifecycle endpoints (link, auth revoke, ident rotate/re-anchor) | → plan-23 (plan-23-B) |
| `/log/checkpoint` `/log/proof/<entry>` | → plan-23 (plan-23-B) |
| `/index/<owner>#<package>` | ❌ plan-10-A |
| `/blob/<hash>` | ❌ plan-10-A |
| `/release-state` | ❌ plan-10-C |
| `/root.json` `/snapshot.json` `/timestamp.json` | ❌ plan-10-C |
| Release states `deprecated`/`yanked`/`blocked`/`legal-tombstoned` | ❌ plan-10-C (only `available`) |

### 2.6 Operational gaps (not in spec, needed for correctness)

- Single `Mutex<Connection>` serializes all DB access; no WAL, no pool.
- No expired-challenge / expired-session reaping.
- No rate limiting on registration, challenge, or login.
- Blob written to disk *before* the DB row commits in `publish_package`; a
  failed transaction leaves an orphan blob.
- Session and challenge verification run twice per publish (minor).
- No request-size cap on inline base64 artifacts; no upload-reference path.

---

## 3. Sub-plan documents

Split by effort into four small/medium sub-plans; each bundles the phases shown and is an
independently shippable document. Open the lettered file for its phases, tasks, and acceptance.

| Doc | Effort | Bundles phases | Depends on |
|---|---|---|---|
| [plan-10-A](plan-10-A-keys-install.md) — Install path | small | `/blob` + `/index` (key phase superseded by plan-23) | plan-23 |
| [plan-10-B](plan-10-B-resolution.md) — Resolution | medium | ABI index · resolver + lockfile | plan-23, A |
| [plan-10-C](plan-10-C-transparency-trust.md) — Release states & signed metadata | medium | release states · signed-metadata root (log superseded by plan-23-B) | plan-23, A |
| [plan-10-D](plan-10-D-accounts-hardening.md) — Accounts + hardening | medium | orgs/tokens/transfers · operational hardening | plan-23, C |

## 4. Suggested Sequencing

```
plan-23 (keys/trust/log — COMPLETE before plan-10 starts)
   └─> plan-10-A (install) ─┬─> plan-10-B (ABI + resolver/lock)
                            │
                            └─> plan-10-C (release states → signed metadata)
plan-10-D (accounts + hardening) interleaves (accounts need plan-23's keys+log, and C).
```

plan-10-A delivers a registry you can install from (plan-23 already made it one
you can securely publish to). plan-10-B makes resolution real. plan-10-C adds
release states and the signed-metadata chain on top of plan-23's server key and
transparency log. The hardening half of plan-10-D depends on nothing and can
slot in anytime.

## 5. Cross-Cutting Decisions To Confirm

1. ~~Local key file layout~~ — **superseded by plan-23** (decided there:
   `<owner>.auth.*` / `<owner>.ident.*`; no standing signing key files).
2. **ABI section placement** in the `.mfp` container — needs a
   container-format amendment (plan-10-B) before the compiler emits it; must
   land inside plan-23's signed prefix / `packageBinaryHash` coverage.
3. ~~Merkle hashing scheme~~ — decided in plan-23-B (RFC 6962, matching
   CT/Rekor tooling).
4. **Where root/snapshot/timestamp private keys live** — offline root must not
   sit on the serving host (plan-10-C); document the signing workflow. The
   plan-23 server (attestation) key becomes a *delegated* key under this root.

## 6. Out Of Scope (future passes, per `repository.md`)

- Threshold / N-of-M multi-sig for critical-tier packages (spec defers to v2).
- Federation / mirror discovery protocol.
- Exact legal-tombstone audit procedure and public takedown fields.
- Critical-package threshold values (download/dependent counts).
