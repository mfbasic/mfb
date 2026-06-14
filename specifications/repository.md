# MFBASIC Package Registry — Design Document v0.1

## Goals

Avoid the failure modes of PyPI and npm specifically:

- **No arbitrary code execution at install time.** `.mfp` is portable typed bytecode with no native pointers; there is no install-script equivalent. Native bindings only enter through the declared `NATIVE_LINK_TABLE`, which is auditable.
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

**Content addressing.** The actual unit of download/cache/reproducibility is a **cryptographic hash of the package content** (signature-excluded — see §3). The registry index maps `ident@version → hash`. A locked install skips the index entirely and fetches by hash from any source — registry, mirror, local cache — that has that blob.

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

- **Account** = identity record: verified email (required), optional phone (recovery/2FA), optional linked OAuth (Google/Apple/Microsoft) for sign-in convenience.
- Personal **owner handle** is reserved at signup, only after email verification (prevents handle-squatting via throwaway addresses).
- **Orgs** are additional owner handles with their own member list and roles (owner / admin / publisher). Layer 2 identity doesn't distinguish personal vs. org owners — same namespace, same rules.
- **OAuth is a signal, not the foundation.** Linking Google/Apple/Microsoft can satisfy email verification if emails match, but account recovery always falls back to the verified email/phone on file — never solely dependent on a third party's account state.
- **2FA — tiered.** Optional by default. Automatically *required* once a package crosses a download/transitive-dependent threshold ("critical package" tier); publishing is blocked until enabled, with advance warning.
- **Publish tokens**, not account credentials, for CI. Scoped to a specific package or org, short default TTL, individually revocable, scope must be a subset of the issuing account's current access.
- **Ownership transfers** are explicit two-sided actions (initiator + acceptor), written to the transparency log — never a support-ticket-mediated process (which is sociallyengineerable).

---

## 3. Trust & Signing

- Each account/org registers one or more **Ed25519 keys**, optionally scoped to specific package identities (a CI key can be limited to signing only `acme/geometry`). Key registration itself requires the account's normal auth (2FA if enabled) and is logged.
- At publish time, the registry checks the `.mfp`'s signed `identKey` and signature against the account's *currently registered, non-revoked, in-scope* Ed25519 keys. **Unrecognized keys are rejected outright** — never silently accepted as "first seen, trust it."
- **Content hash excludes the signature region.** Re-signing a version after key rotation doesn't change its content hash, so downstream lockfiles stay valid through a rotation/revocation response.
- **Revocation:** immediate stop on new publishes from that key, logged with timestamp. Already-published versions aren't auto-invalidated (avoids a self-inflicted outage); reviewed individually and yanked if malicious.
- **Transparency log** (Merkle-tree, CT/Rekor-style), built from v1: every key registration, revocation, publish (identity, version, content hash, signer fingerprint, timestamp), and ownership transfer. Clients pin the last-seen checkpoint (no rollback). Lets any maintainer audit "did I really publish this?" and makes registry-level compromises detectable rather than invisible.
- **Index signing root-of-trust:** v1 index is signed by an online key; the index format reserves a field for an offline-root-key / threshold scheme (TUF-style root role) so this can be added in v2 without a breaking format change.
- **Threshold/multi-sig** for critical-tier packages (N-of-M maintainer signatures required to publish): deferred to v2, gated behind the same critical-package threshold as mandatory 2FA.
- **First-install trust gap, stated plainly:** a client resolving a never-before-seen identity trusts the registry's *current* index mapping — same baseline as PyPI/npm/crates.io today. The difference is that mapping is itself in the transparency log, so any later tampering is permanently, publicly checkable.

---

## 4. Resolution & Lockfile

- Dependency graph nodes are idents (`<owner>#<package>`). For each ident, collect all requested ABI anchors and pins from every package that depends on it; resolve to one selected version per ident.
- A resolution failure produces a diagnostic naming the conflicting requirers directly, including the symbol ABI hashes that disagree or the pinned versions that cannot both be selected.
- `mfb.lock` records exact selected versions, requested versions, pins, source aliases, content hashes, signer metadata, package/container versions, bytecode versions, ABI metadata, native metadata hashes, and transitive dependencies. Its schema is specified in `lockfile.md`.
- Re-resolving without `update` must reproduce byte-identical locks. A locked `install` with a current lockfile fetches by `hash` alone and does not consult the registry index.

---

## 5. Registry API & Storage (sketch)

- **Blob storage**: content-addressed, immutable, keyed by the signature-excluded content hash. **Write-once, permanent** — once a `.mfp` is released under a hash, `GET /blob/<hash>` works forever; the blob store has no delete path for normal operation. Infinitely cacheable — any CDN or third-party mirror can serve blobs without being trusted, since the client verifies hash + signature regardless of source.
- **Index/metadata service**: maps `<owner>#<package>@<version> → hash`, plus signature, signer fingerprint, and transparency-log reference for each version. This layer is **mutable** — a release's metadata can be edited, or the release can be removed from the index entirely (delisted/yanked), without touching the underlying blob. A locked install that already has the hash never consults the index, so delisting a version doesn't break existing lockfiles — it only prevents *new* resolutions from finding it. Small, mutable, short-cache.
- **Core endpoints**:
  - `GET /index/<owner>#<package>` — version list + metadata
  - `GET /blob/<hash>` — the `.mfp` itself
  - `POST /publish` — authenticated, enforces key/scope checks from §3
  - `GET /log/checkpoint`, `GET /log/proof/<entry>` — transparency log access
- **Mirroring/federation**: because blobs are self-verifying (hash + signature), any mirror can serve them regardless of which registry's index pointed there. Self-hosted/internal registries are just additional `source` aliases configured locally — they don't need to be embedded in the ident itself.

---

## 6. Policy Summary

| Question | Decision |
|---|---|
| Major version = new identity? | No — ident grammar is `<owner>#package`; publishers use a separate ident (e.g. `<owner>#shape2`) by convention if v1/v2 must coexist |
| Transparency log? | Yes, from v1 |
| Offline root keys for index? | Format reserves field; mechanism deferred to v2 |
| Threshold/multi-sig for critical packages | Deferred to v2 |
| Typosquat detection | Warn-only at v1 |
| Owner handle registration | Registry-issued accounts, email/phone verified, OAuth as additional sign-in |
| Yanking | Hide from new resolution; bytes retained; never full delete |
| Blob vs. metadata lifecycle | `/blob/<hash>` is permanent and undeletable in normal operation; index/metadata (listing, description, yank status) is mutable independent of the blob |
| Name/identity recycling | Never, once published |

---

## 7. Lockfile Format

The lockfile format lives in `lockfile.md`. The registry design depends on these lockfile properties:

- Locked packages are fetched and verified by signature-excluded content hash.
- Registry and mirror endpoint URLs are local configuration; the lockfile stores source aliases, idents, and hashes.
- The lockfile records the transparency-log checkpoint, signer metadata, selected versions, requested versions, pin state, ABI metadata, native metadata hashes, package/container versions, bytecode versions, and transitive dependency cache.
- `install` with a current lockfile does not resolve or update versions. `update` is the explicit operation that re-runs resolution.

---

## 8. Dependency Version Resolution (ABI-Generation Model)

### 8.1 The core idea

`project.json`'s `packages[].version` (and the `@<version>` in `mfb pkg add`) is **not** a semver range and **not** a pin. It's a **resolution anchor**: "find me whatever is ABI-compatible with this version, at its best available point."

This replaces the `^`/`~`/`>=`/wildcard range syntax in `project.md` §7 entirely. There is exactly one form: a concrete version string (`"2.1.0"`).

### 8.2 The `ABI_INDEX` section

Each published `.mfp` carries an `ABI_INDEX`: a map from each exported declaration's identity — function/sub signature, exported type and its fields, constant, `NATIVE_LINK_TABLE` entry — to a hash of that declaration's full signature. This is computed by the compiler at build time. It's not a single per-package number; it's one hash per exported symbol.

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
2. Among published versions `V` with `ABI_INDEX(V) ⊇ ABI_INDEX(anchor-version)`, select the highest.
3. Write that version + its content hash to `project.json`/lockfile.

This means: explicitly adding/updating a dependency always gets you the most up-to-date *non-breaking* release — bugfixes and additions — but never silently substitutes something that changed or dropped a symbol the anchor version had.

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

- Legal/takedown exception process for the blob store's "permanent, undeletable" guarantee (e.g. CSAM, court order) — a true zero-exception policy is rarely tenable; needs a narrow, separately-audited path that doesn't undermine the reproducibility guarantee for everything else
- Exact `.mfp` header layout change needed to exclude signature from the hashed region
- Critical-package threshold values (download count / dependent count)
- Index format schema for the v2 TUF root-key extension
- Federation/mirror discovery protocol details
