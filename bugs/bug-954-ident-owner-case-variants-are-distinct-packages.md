# bug-954: package idents differing only in owner-prefix case are distinct packages, so a former owner can re-publish a transferred package's name

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: (proposed) repository/src/server.rs `a_case_variant_ident_is_the_same_package`

## Cause

- Rule: identity is case-insensitive (owner decision 2026-10-05; `04_owner-names.md` "Case Folding"): two idents that fold to the same `fold_owner(owner)#fold(package)` are one package; display keeps the registered spelling.
- Where: `repository/src/store.rs` keys packages on the exact `packages.ident` string (`package_owner` and every lookup use `p.ident = ?1`); `server.rs:validate_package_request`/`require_package_owner` fall back to folding the prefix only on a miss.
- Fix: one canonical identity key (a folded-ident column with the UNIQUE constraint, filled by a migration) that every store lookup by ident uses, so a case variant resolves to the existing package; the exact-string lookups go. Migration: a registry already holding two case variants of one ident is reported and refuses to start (one of the owner's two allowed resolutions; nothing is silently dropped). This also fixes bug-947's ident half: `/log/publish` looks the release up by canonical ident and byte-exact version (no `LIKE`), so `1.0.0-RC` and `1.0.0-rc` stay distinct.

**Owner decision (2026-10-05):** identity is case-insensitive. Owner and package names compare case-insensitively
for uniqueness, registration, publish, transfer, lookup and the transparency log. `Alice` and
`alice` are the same owner, and `Alice#toolbox` resolves to the existing `alice#toolbox`. Display
keeps the registered spelling. This also covers bug-947 (case-insensitive `/log/publish` lookups
returning the wrong entry): once identity is case-insensitive, those lookups must return the one
canonical entry. Migration: an existing registry that already holds two case variants needs a
defined resolution (report and refuse to start, or keep the first-registered one). Bring it to
the owner if found.

Owner names are case-insensitive ("`Alice` and `alice` are the *same* owner"), but the
registry keys packages on the **exact** ident string (`packages.ident TEXT UNIQUE`,
`Store::package_owner` uses `p.ident = ?1`). A first publish is authorized by
**folding** the ident's owner prefix. So `alice#toolbox`, `Alice#toolbox` and
`ALICE#toolbox` are three different packages, and all three belong to whoever is
registered as `alice`. Each has its own index, versions, owner and transfer history.

This becomes a squatting vector after an ownership transfer. Once alice has transferred
`alice#toolbox` to carl, she can still publish `Alice#toolbox`, a brand-new package,
because its prefix folds to her and no row with that exact ident exists. A consumer
who types `mfb pkg add Alice#toolbox` (owner names "don't care about case") then gets
the former owner's code, which verifies cleanly under alice's key. The only signal is
the warn-only typosquat warning, and only when the variant is one edit away:
`ALICE#toolbox` gets no warning at all. The search page lists all three side by side.

Expected behavior (needs an owner decision — UNCLEAR): either (a) the owner component of
an ident is canonicalized/folded for package identity, so `Alice#toolbox` resolves to the
same package as `alice#toolbox` (and a case-variant first publish is refused as
"already exists, owned by carl"), or (b) the spec states that ident identity is
case-sensitive and the registry refuses a first publish whose ident case-folds to an
existing package's ident.

References:

- src/docs/spec/package-manager/04_owner-names.md, "Case Folding": "Owner names are compared **case-insensitively** … `Alice` and `alice` are the *same* owner"; "At a package's first publish the owner segment parsed out of the package ident is folded and compared against the folded session subject".
- 01_repository-protocol.md, "Ownership transfer" (the stored owner, not the prefix, governs after a transfer).
- Related: B16-log-publish-lookup-case-insensitive (the same case variants also corrupt `/log/publish`).

## Failing Reproduction

Local registry (127.0.0.1, /tmp data), /tmp/hardening/mfb-head:

```
mfb repo register alice; mfb repo auth alice
mfb init-pkg toolbox; (add "ident": "alice#toolbox"); mfb repo publish alice toolbox
mfb repo register carl;  mfb repo auth carl
mfb repo transfer 'alice#toolbox' carl
mfb repo transfer-accept 'alice#toolbox@carl'
cp -r toolbox tb3; set "ident": "Alice#toolbox", "version": "9.0.0" in tb3/project.json
mfb repo publish alice tb3
curl '127.0.0.1:47716/search?q=toolbox'
```

- Observed:
  ```
  Published Alice#toolbox@9.0.0 as 84347757…
  warning: published `Alice#toolbox` is one edit away from existing `alice#toolbox`
  ```
  The search lists `ALICE#toolbox` (owner alice), `Alice#toolbox` (owner alice, 9.0.0) and
  `alice#toolbox` (owner carl). `GET /index/Alice%23toolbox` returns a separate version list.
  An `ALICE#toolbox` publish printed no warning at all.
- Expected: the `Alice#toolbox` publish is refused, or resolves to the package carl now owns.
  Under option (a) it would be refused as "session owner is not the package's current owner".

## Root Cause

`repository/src/server.rs:validate_package_request` and `require_package_owner` look the
package up by exact ident (`Store::package_owner` → `WHERE p.ident = ?1`). On a miss they
fall back to `fold_owner(prefix) == fold_owner(session)`. Nothing folds the ident itself.
`store.rs:typosquat_candidates` compares raw strings with edit distance 1 and only warns.

## Goal

- One canonical package per case-folded owner prefix (+ package name per the chosen rule).
  A case-variant first publish must not create a second package.

### Non-goals (must NOT change)

- Already-published artifacts' signed ident strings (they keep verifying under the spelling they signed).

## Blast Radius

- `/signing` (`require_package_owner`), `/validate`+`/publish` (`validate_package_request`), `/index`, `/packages/*`, token scopes (`scope_permits` exact-scope compare) all key on the raw ident.
