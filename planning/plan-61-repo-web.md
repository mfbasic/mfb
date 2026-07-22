# Transparent Registry Web Interface Plan

Last updated: 2026-07-21
Effort: x-large (1d–3d)

This feature gives `mfb-repo` a public, anonymous, read-only web interface: a
landing page with a search box and the server fingerprint, a search results
list, and a package page showing the latest version, every version, the
package's description/author/url, the native blobs it ships and which targets
they support, and a transparency tab exposing the append-only log, release-state
history, and identity-key rotations.

The single behavioral outcome a correct implementation produces: **an anonymous
browser with no credentials can, over plain HTTP GETs, read the complete
published history of any package on the server — including yanked versions,
state transitions, key rotations, and transparency-log inclusion proofs — and
the server rejects every attempt to mutate state through that surface.**

Reaching it requires metadata plumbing that does not exist today. The registry
currently persists no human-facing metadata at all: `package_versions` carries
only `version`, `hash`, `state`, `abi_index`, `created_at`
(`repository/src/store.rs:263`). `author` and `url` are parsed at publish time
and discarded. Native target triples are parsed and explicitly skipped
(`repository/src/abi.rs:104-112`). `description` does not exist anywhere in the
toolchain. Sub-plans A and D add that data; B and C expose it.

References:

- **`planning/plan-61/` — the HTML/CSS mockups for the web surface**, plus
  `DESIGN-RATIONALE.md`. Consumed by plan-61-C, which defines their scope in
  §3.1: they are normative for *appearance only*. The plan documents own the
  routes, the data shapes, and the escaping/CSP contract; where a mockup
  disagrees, the plan wins.
- `src/docs/spec/package-manager/01_repository-protocol.md` — the wire protocol
  and endpoint table this plan extends
- `src/docs/spec/package/01_container-format.md` — the `.mfp` header, hard
  version 1.0
- `src/docs/spec/package/02_binary-representation.md` — the MFPC section table
- `src/docs/spec/package/03_metadata-encoding.md` — MANIFEST section 1
- `src/docs/spec/tooling/01_project-manifest.md` — the `project.json` schema
- `.ai/specifications.md` — spec-sync obligations (a format change updates the
  owning spec topic *in the same change*; this is part of the Hard Completion
  Gate)
- `bugs/completed-bugs/bug-347-repository-tests-never-run.md` — the prerequisite
  (fixed 2026-07-21)
- `bugs/skipped/bug-189-supply-chain-bootstrap-downgrade.md` — SUP-03, the
  downgrade weakness this plan partially mitigates

## Prerequisites

These are a precondition on the whole feature, not a dependency to negotiate.
Stated once here; every sub-plan points back to this section.

| Must be true | Command | Status |
|---|---|---|
| **bug-347 is fixed — `repository/` is in the Cargo workspace and its tests run** | `cargo test --workspace --no-run 2>&1 \| grep mfb_repository` → must list a `mfb_repository` unittests binary | **MET** (2026-07-21, re-checked after the fix landed): the command lists `Executable unittests src/lib.rs (…/mfb_repository-…)`. Landed in `43b97022f` (+ test work in `d0e3962b8`, `3ee3342ad`); the doc is now `bugs/completed-bugs/bug-347-repository-tests-never-run.md`, `Status: Fixed`. The earlier "wait for it to commit" instruction is discharged — it is safe to start. |
| The repository crate's existing tests pass | `cargo test -p mfb_repository` → 0 failures | **MET** (2026-07-21): 283 passed (264 lib + 19 bin), 0 failed. Note the count grew from 164 — bug-347's burn-down took `repository/src` to 97.76% line coverage. |
| Working tree is clean of unrelated repo-server edits | `git status --porcelain repository/` → empty | **MET** (2026-07-21, re-checked): empty. The `repository/Cargo.lock` deletion committed as part of bug-347 (a workspace member cannot carry its own lockfile). |
| **plan-60 is complete** — it edits the same spec topic and CLI surface | `ls planning/plan-60-*` → no matches (archived to `planning/old-plans/`) | **MET** (2026-07-21, re-checked): no matches; all six docs are in `planning/old-plans/`. Was NOT MET at authoring time. |

### Why plan-60 is a gate

> **RESOLVED 2026-07-21.** plan-60 completed and all six documents are archived
> to `planning/old-plans/`. The reasoning below is kept because the *shape* of
> the hazard recurs — and it has: bug-347 is now the in-flight change occupying
> the same tree. Re-read this section as a live warning about that, not as
> history.

This was discovered while writing plan-61, not predicted — another agent is
working plan-60 in this shared tree concurrently (`src/cli/repo.rs` was modified
at 09:31 on 2026-07-21, mid-authoring). plan-60 currently has uncommitted edits
to files plan-61 depends on:

```
$ git diff --stat src/docs/spec/package-manager/01_repository-protocol.md \
                  src/cli/repo.rs src/cli/pkg.rs src/binary_repr/reader.rs
 src/binary_repr/reader.rs                        |  2 +-
 src/cli/pkg.rs                                   | 55 +++++++++++++++---
 src/cli/repo.rs                                  | 31 +++++------
 .../package-manager/01_repository-protocol.md    | 26 +++++-----
```

Three of those four are plan-61 surfaces: `01_repository-protocol.md` is the spec
topic plan-61-B Phase 3 and plan-61-C Phase 4 both edit, `src/cli/pkg.rs` is the
publish path plan-61-A and plan-61-E hook, and `src/binary_repr/reader.rs` is
where plan-61-D adds the section-18 read. Starting plan-61 while plan-60 is
mid-flight means both plans edit the same files with no shared branch — and per
the working convention in this tree, other agents stash the whole tree, so a
half-finished plan-61 edit can vanish or collide.

Per the write-plan rule, this is a precondition, not scope to absorb and not a
soft preference: **if plan-60 is not complete, plan-61 cannot start, full stop.**

Note also that every measurement in §Measured populations was taken on
2026-07-21 against a tree plan-60 is actively changing. **Re-run each command
before relying on its number**, particularly the 81-manifest count that sets
plan-61-F's scope.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you.

### Why bug-347 is a hard gate and not a task inside this plan

This is the most important structural decision in the plan, so it is stated
plainly rather than buried.

`repository/` is outside the Cargo workspace — the root `Cargo.toml` declares no
`[workspace]` section, so `mfb_repository` is pulled in only as a `path`
dependency and Cargo never builds its test targets. The result, per bug-347:
**123 `#[test]` functions across 11 files have never executed**, and ~13,214
lines — the registry's HTTP surface, its SQLite store, its credential handling,
and its crypto — sit outside every automated gate. Measured today:

```
$ grep -rc '#\[test\]' repository/src/*.rs | grep -v ':0'
repository/src/store.rs:40   client.rs:25   abi.rs:14   package.rs:15
log.rs:9   local.rs:8   blobstore.rs:7   validation.rs:6   crypto.rs:5
main.rs:11   gc.rs:2   server.rs:2
```

This plan adds a public, unauthenticated, HTML-emitting attack surface to
precisely that crate, including HTML escaping and an href scheme allowlist whose
*only* proof of correctness is unit tests. Landing XSS-defense code into a crate
whose tests never execute is not acceptable.

There is a second, sharper reason. `.github/workflows/coverage.yml` enforces a
**global 95% line floor** and a per-file 95% gate, and both pass today
*precisely because* this ~13k lines is not in the denominator. Whoever fixes
bug-347 will drop 13k unmeasured lines plus everything this plan adds into that
denominator at once. Landing this plan first makes that cliff worse and blames
it on the wrong change.

Per the write-plan rule: a bug a plan depends on is a prerequisite with a command
that checks whether the fix landed — never scope this plan absorbs, never a soft
preference, never a dual-mode design. bug-347 is `Effort: medium (1h–2h)`. Fix
it, then start here.

Everything below is written against the world where these hold. There are no
hedges for the world where they don't.

## Dependency graph

```
A ← nothing
B ← A
C ← B
D ← nothing
E ← C + D
F ← E
```

A (server-side metadata capture) and D (the `description` field) block on
nothing but the prerequisites and can proceed in parallel. B and C build the
JSON and HTML surfaces over A. E rejoins them: it ingests D's description into
A's schema and renders it in C's pages. F lands last and alone: it migrates the
tree's 81 package manifests, regenerates the golden corpus, and flips
`description` from warning to required.

Execution is topological order, re-checking each letter's stated preconditions.

**The site is independently useful after C.** A reader who must stop early
should stop after C, which delivers a complete working web interface missing
only package descriptions. D, E, and F are separable and can land later without
rework beyond one column, one JSON field, and one template line.

| Letter | File | Effort | Delivers |
|---|---|---|---|
| A | `plan-61-A-server-metadata-capture.md` | medium | `author`/`url` persisted; native targets recorded; schema + backfill |
| B | `plan-61-B-read-endpoints.md` | medium | `GET /search`, `GET /packages/:ident`, `GET /packages/:ident/audit` |
| C | `plan-61-C-web-ui.md` | medium | The three HTML pages, escaping, CSP (mockups in `planning/plan-61/`) |
| D | `plan-61-D-description-field.md` | medium | `description` in `project.json` → new MFPC section 18, optional + warning |
| E | `plan-61-E-description-surfacing.md` | small | Description ingested, searchable, rendered |
| F | `plan-61-F-required-migration.md` | medium | 81 manifests migrated, goldens regenerated, field made required |

## 1. Goal

- An anonymous browser can search packages, open a package page, and read every
  published version (including yanked and non-current states), the native blob
  target matrix, and a transparency tab with log checkpoints, inclusion proofs,
  release-state transitions, and ident-key rotations — with no account, no
  cookie, and no credential of any kind.
- `mfb pkg add <name>` has a server-side search endpoint to call, closing the
  gap between `planning/old-moved-to-src-spec/repository.md:66` (which specifies
  cross-owner search) and the current protocol, which has no enumerate route at
  all.
- `project.json` gains a `description` field, required for `kind: "package"`,
  optional for `kind: "executable"`, carried inside the signed `.mfp` payload.

### Non-goals (explicit constraints)

- **No authentication on the web surface, ever.** No login page, no
  registration, no account management, no session. The web surface is anonymous
  and read-only in perpetuity.
- **No cookie authentication anywhere in `mfb-repo`.** Today every authenticated
  route takes `sessionToken` as a JSON body field; the sole exception,
  `PUT /blob/:hash`, uses an `Authorization: Bearer` header because a raw-body
  PUT has no JSON body (`repository/src/server.rs:1521-1524`). That means the
  server has **zero CSRF surface**, and adding HTML on the same origin does not
  create any. This property must be preserved deliberately. Introducing cookie
  auth would silently make every existing mutating POST CSRF-vulnerable.
- **No write path from the web.** Every route added by this plan is `GET`.
- **No `license`, `keywords`, or `readme` fields.** Only `description` was
  requested. The section-18 design (D §3) leaves room for them; adding them is
  out of scope.
- **The `.mfp` container version stays 1.0.** No change to the header layout, no
  change to MANIFEST section 1, no change to the ABI hash. See D §2 for why this
  is achievable and non-negotiable.
- **No federation or mirror support.**
- **No change to the existing JSON API's shapes.** `GET /index/:ident` keeps its
  current response exactly; new data goes on new routes.
- **The DOC section (id 17) is not repurposed.** Its prose comes from a
  `' PACKAGE` `DOC` block in MFBASIC source; `description` comes from the
  manifest. These are two different facts with two different authors and they
  stay separate.

## 2. Current State

### The registry stores almost no metadata

`package_versions` (`repository/src/store.rs:263`) is
`(id, package_id, version, hash, state, abi_index TEXT DEFAULT '{}', created_at)`.
`packages` (`store.rs:256`) is `(id, ident UNIQUE, owner_id, created_at)`. There
is no description, license, author, url, keyword, or target column anywhere.

`author` and `url` *are* already parsed — `MfpPackage`
(`repository/src/package.rs:22-27`) exposes both from the header, and
`read_mfp_header` (`repository/src/package.rs:118-129`) reads them with 512- and
2048-byte caps. They are then dropped. Persisting them requires **no format
change** (A).

They are also duplicated inside the signed payload: `encode_manifest`
(`src/binary_repr/writer.rs:992-1014`) writes interned stringIds for
`author` and `url` alongside the identity fields. The header copy is a plaintext
fast-scan convenience; the MANIFEST copy is inside `packageBinaryRepr`, which is
covered by `packageBinaryHash` and therefore by the package signature. **Render
from the MANIFEST copy** (A §4).

### Native target triples are parsed and thrown away

`repository/src/abi.rs:104-112` walks a section-10 locator and skips the
platform axis:

```rust
// os, arch: interned string ids (skipped — not needed here).
offset += 4; // os
offset += 4; // arch
let _libc = read_u8(bytes, offset)?;
```

`VendorBlobRef` (`abi.rs:44-48`) keeps only `{logical, source, hash}`.

### There is no search, and no enumerate route of any kind

`GET /index/:ident` requires an exact `<owner>#<package>` and 400s otherwise
(`repository/src/server.rs:919-923`). No route lists packages, owners, or
versions in bulk. The only fuzzy-matching primitive that exists is
`store.typosquat_candidates(ident)` (used warn-only at `server.rs:2104-2115`),
which finds idents within edit distance 1.

### There is no HTML anywhere

No `tower-http`, no `ServeDir`, no template engine in `repository/Cargo.toml`.
Every handler returns `Json<T>` except `GET /blob`, which returns
`application/octet-stream` or a 302 (`server.rs:1468-1483`). No static assets, no
fallback route. This is greenfield.

### Measured populations

| What | Count | Command |
|---|---|---|
| `project.json` files in tree (excl. `target/`, vendored `packages/`) | 1087 | `find . -name project.json -not -path './target/*' -not -path '*/packages/*' \| wc -l` |
| …declaring `kind: "executable"` | 1005 | same, `-exec grep -ohE '"kind"[[:space:]]*:[[:space:]]*"[a-zA-Z]+"' {} + \| sort \| uniq -c` → 950 spaced + 55 unspaced |
| **…declaring `kind: "package"`** (D's migration surface) | **81** | same command, `"package"` row |
| …with no `kind` key at all | 1 | `find … \| while read f; do grep -q '"kind"' "$f" \|\| echo "$f"; done \| wc -l` |
| …declaring an `ident` | 37 | `find … \| while read f; do grep -q '"ident"' "$f" && echo "$f"; done \| wc -l` |
| Package fixtures that also carry a `golden/` dir | 49 | `find … -exec grep -l '"kind".*"package"' {} + \| while read f; do d=$(dirname "$f"); [ -d "$d/golden" ] && echo "$d"; done \| wc -l` |
| `.mfp` goldens (churn if the payload changes) | 16 | `find tests -name '*.mfp' -path '*/golden/*' \| wc -l` |
| `.hex` (binary-repr) goldens | 2 | `find tests -name '*.hex' -path '*/golden/*' \| wc -l` |
| `.info` / `.audit` goldens | 23 | `find tests \( -name '*.info' -o -name '*.audit' \) -path '*/golden/*' \| wc -l` |
| `#[test]` fns in `repository/src` that never run today | 123 | bug-347; `grep -rc '#\[test\]' repository/src/*.rs \| grep -v ':0'` sums to 144 across 12 files — **see Corrections note below** |
| Highest MFPC section id in use | 17 (`SECTION_DOC_TABLE`) | `grep -nE 'SECTION_[A-Z_]+: u16 = [0-9]+' src/binary_repr/mod.rs` |

Where the 81 package manifests live
(`… -exec grep -l '"kind".*"package"' {} + | sed -E 's|^\./([^/]+/[^/]+)/.*|\1|' | sort | uniq -c`):

| Location | Count |
|---|---|
| `tests/syntax` | 49 |
| `tools/thread-package-sources` | 18 |
| `tools/security-package-sources` | 9 |
| `tools/link-package-sources` | 2 |
| `bindings/sqlite3` | 1 |
| `bindings/libsnd` | 1 |
| `benchmark/mfb` | 1 |

> **Discrepancy to resolve in D, not assumed away:** bug-347 states 123 tests
> across 11 files; the count above is 144 across 12 files. The bug doc is from
> 2026-07-18 and tests have been added since. Neither number changes any
> decision in this plan, but D must not cite either without re-measuring.

### Verified properties

These are claims a `file:line` citation cannot establish. Each was verified by
reading the code named.

- **VERIFIED — an unknown MFPC section id is silently ignored by every reader.**
  This is the fact the whole `description` design rests on. Read
  `src/binary_repr/reader.rs:320-352` and `repository/src/abi.rs:168-195`: in
  both, the section id is used *only* as a `HashMap`/`BTreeMap` key. There is no
  `match` on the id, no membership test against the section constants, no
  `unknown section` error path. Every subsequent access is a positive
  `sections.get(&SECTION_*)` lookup (`reader.rs:354-434`), and nothing iterates
  the map or asserts it was fully consumed. Optional sections are handled by
  absence-from-map — `reader.rs:407-410` for DOC. The intent is documented at
  `src/binary_repr/mod.rs:35-41` ("a consumer that does not understand it skips
  it entirely") and `src/docs/spec/package/02_binary-representation.md:49`
  ("Sections may appear in any order… loads the section table into a map keyed
  by `sectionId`"). **Therefore adding section 18 is forward-compatible: an old
  reader parses a new package successfully.**
- **VERIFIED — the `.mfp` header and MANIFEST section 1 are both hard-break
  surfaces.** The header is a fixed-order positional record with no length field
  and no skip mechanism; `repository/src/package.rs:118-129` walks it with a
  single moving offset and `:164-166` requires exact end alignment. The container
  rule is hard version 1.0 with no backward compatibility
  (`src/docs/spec/package/01_container-format.md:8-10`), and both readers reject
  anything but exactly 1.0 (`repository/src/package.rs:109-113`,
  `src/manifest/package.rs:143-150`) — so even bumping `containerMinor` is itself
  a break. `read_manifest` (`src/binary_repr/reader.rs:973-1014`) ends with
  `if offset != bytes.len() { return Err("invalid trailing bytes in manifest") }`.
  **Both candidate locations are rejected in D §2 on this evidence.**
- **VERIFIED — the ABI hash does not cover manifest or header fields.** Per
  `src/docs/spec/package/03_metadata-encoding.md:185-196`, `AbiSerializer` input
  is `MFBABI\0` + `abiFormatVersion` + per-symbol signature data only. A new
  section 18 therefore cannot change any `abiHash`, and ABI-compat checks stay
  green across the migration.
- **VERIFIED — the whole-file content hash covers everything, so D changes
  content hashes.** `package_content_hash` (`src/target/package_mfp/mod.rs:159-164`)
  is `sha256(bytes)` over the entire file. Adding section 18 changes the payload,
  hence `packageBinaryHash`, hence the signed prefix, hence the content hash.
  This is *not* newly breaking: the hash already covers the per-build signature,
  so pins are inherently rebuild-sensitive — a rebuilt package never matched its
  predecessor's hash. Only packages that are actually rebuilt are affected, and
  only from F onward, when manifests gain descriptions.
- **VERIFIED — the string pool needed to resolve os/arch is already parsed
  server-side.** `parse_vendor_blobs` loads it at `repository/src/abi.rs:69-73`
  before touching section 10, and `table_string` (`abi.rs:133-138`) already
  resolves `logical` and `source` on the same locator. A §3's change needs no new
  section parsing and no new I/O.
- **VERIFIED — the server has zero CSRF surface today.** Every authenticated
  handler reads `sessionToken` from the JSON body; the single exception is
  `PUT /blob/:hash` at `repository/src/server.rs:1521-1524`, which uses a bearer
  header. No handler reads a cookie. Confirmed by reading the route table at
  `server.rs:672-704` and the auth helper each handler calls.
- **VERIFIED — `2-200-0016` is the next free `PROJECT_JSON_*` code.** An earlier
  draft recorded this as UNVERIFIED on the strength of an empty
  `grep -oE 'PROJECT_JSON_[A-Z_]+' src/docs/spec/diagnostics/02_error-codes.md`.
  That grep hit the wrong registry. `02_error-codes.md` is the **runtime**
  `errorCode::` table (the one `build.rs` generates from, and the one
  `.ai/specifications.md:45-50` governs); it says in its own head that the
  compiler-facing rule set is separate. `PROJECT_JSON_*` lives in
  `src/docs/spec/diagnostics/01_rule-codes.md:260-274` as `2-200-NNNN`, spelled
  exactly as `validate_kind` uses them, with `0001`–`0015` allocated. The backing
  array is `src/rules/table.rs` and the drift guard is
  `every_rule_is_documented_in_the_spec` (`src/rules/mod.rs:231-249`) — `build.rs`
  is not involved. See D §4 for the allocation procedure.
- **UNVERIFIED — whether `mfb pkg publish` or the resolver rejects a package
  whose manifest lacks `description` once D lands.** D Phase 4 must check the
  resolver and lockfile paths, not just the builder.

## 3. Design Overview

Four layers, bottom-up:

1. **Capture** (A, D) — get the metadata into the server's database. A needs no
   format change: `author`/`url` are already parsed and discarded, and the
   os/arch/libc axis is already in the bytes. D adds `description` via a new
   MFPC section.
2. **Query** (B) — JSON endpoints over that data: search, package detail,
   transparency. Built first and independently, because
   `planning/old-moved-to-src-spec/repository.md:66` already specifies
   `mfb pkg add` doing cross-owner search and the CLI needs this endpoint whether
   or not a web UI ever exists.
3. **Render** (C) — server-side HTML over the layer-2 JSON. No client-side
   framework, no build step, no JavaScript required for any core function.
4. **Surface** (E) — join D's description into A's schema and C's pages.

### Where design uncertainty concentrated — and how it was resolved

The plan's premise was that `description` could be added without a flag-day
rebuild of every package. That was genuinely uncertain: the container is
explicitly hard-version-1.0 with no backward compatibility, and both obvious
homes (header, MANIFEST section 1) turned out to be hard breaks. It was resolved
*before* writing, by reading both section-table parsers, and the answer is the
new-section design in D §3. **No phase in this plan is a spike, because the
premise is already falsified-or-confirmed.** Had the section table validated
ids, D would have been a container-version-bump plan of very different shape.

### Where correctness risk concentrates — scheduled last

- **F's golden churn.** 16 `.mfp` + 2 `.hex` + 23 `.info`/`.audit` goldens change
  once packages actually carry a description, and 49 package fixtures carry
  goldens. Per the write-plan rule, output-churning work lands alone: F is its
  own letter, it is last, and its regeneration is its own phase. Note that D
  itself should churn **zero** goldens, because it omits section 18 when there is
  no description — a diff appearing in D is a bug, not a baseline.
- **F's required-field flip.** Making a field required breaks 81 manifests at
  once. F Phase 4 does the flip, after F Phase 2's migration, so no intermediate
  commit is red.
- **C's XSS surface.** The only new *security* risk in the plan. Publisher-
  controlled strings get rendered, and `url` is a 2048-byte field that becomes an
  `href`. Escaping and the scheme allowlist are decided up front (C §2) rather
  than left to the template.

### Rejected alternatives

- **`description` in the `.mfp` header.** Rejected: breaking in both directions
  (see Verified properties), and the container's own compat rule forecloses a
  graceful path.
- **`description` in MANIFEST section 1.** Rejected: `read_manifest` explicitly
  rejects trailing bytes.
- **Reusing the DOC section (17) for `description`.** Rejected: DOC's prose comes
  from source, `description` comes from the manifest. Two authors, two facts,
  two sources of truth — merging them guarantees a package page that disagrees
  with `mfb pkg doc`. Also, `read_doc_table` is positional and would need the
  same trailing-field treatment anyway.
- **Taking `description` from the publish request instead of the artifact.**
  Rejected: unsigned. A registry whose whole pitch is transparency must not
  render publisher metadata that the publisher did not sign.
- **A client-side SPA over the JSON API.** Rejected: adds a build step and a JS
  toolchain to a project whose stated principle is a single self-contained
  binary (`planning/old-moved-to-src-spec/repo-base.md:21-28`).
- **Making `description` optional-with-default**, matching the plan-58-C
  `maxBuffer` precedent. Rejected *as the end state* because the user explicitly
  required it for packages — but adopted as the **intermediate** state, which is
  how E Phase 3 keeps every commit green. See Open Decisions.

## 4. Transparency as a security property, not a presentation choice

This is worth stating because it changes what "done" means for B and C.

`bugs/skipped/bug-189-supply-chain-bootstrap-downgrade.md` is open on SUP-03: the
`/index` version array is **not integrity-protected** in the default path. Only
the owner→identKey name binding is signature-checked, not the `versions[]` array.
A malicious or post-bootstrap-MITM registry can therefore truncate or withhold
newer versions to force a **downgrade**, and a client talking only to that
registry cannot tell.

A publicly browsable transparency log is a partial mitigation, because it makes
the attack *detectable by third parties*: a registry that shows different
histories to different clients can be caught by anyone comparing the public view
against their own. This is why:

- the audit tab must expose **log inclusion proofs**, not just a prose history —
  a rendered `logEntry` index that cannot be verified proves nothing;
- **yanked and superseded versions must be listed**, not hidden. A UI that
  silently omits non-current versions reproduces the exact truncation the attack
  performs;
- the fingerprint on the landing page is presented as *"compare this against
  your out-of-band source"*, never as a claim the page authenticates itself. An
  MITM serving the page serves its own fingerprint too. C §4 fixes the wording.

This plan does **not** close SUP-03. It builds the surface that makes the
divergence observable.

## Compatibility / Format Impact

| Contract | Change |
|---|---|
| `.mfp` container version | **unchanged** — stays 1.0 |
| `.mfp` header layout | **unchanged** |
| MANIFEST section 1 | **unchanged** |
| MFPC section table | **one new optional section, id 18** (D). Old readers skip it — verified. |
| `abiHash` / ABI-compat checks | **unchanged** — the ABI serializer does not read manifest fields |
| Package content hash | **changes for rebuilt packages** (D). Already rebuild-sensitive because the hash covers the per-build signature. |
| `project.json` schema | **one new field, `description`** — required for `kind: "package"`, optional otherwise (D, E) |
| Existing HTTP routes | **unchanged**, including `GET /index/:ident` |
| New HTTP routes | `GET /search`, `GET /packages/:ident`, `GET /packages/:ident/audit`, and the HTML routes `GET /`, `GET /search.html`, `GET /p/:ident` (names finalized in B/C) |
| SQLite schema | new columns on `package_versions`; new `package_version_targets` table (A) |
| Auth model | **unchanged** — no cookie, no new credential, no new mutating route |

## Phases

This is a split plan; phases live in the lettered sub-plans. See the dependency
graph above for order.

## Validation Plan

Per-letter detail lives in each sub-plan; the shared obligations are:

- **Tests.** Rust unit tests are inline `#[cfg(test)] mod tests` at the bottom of
  the module, and nothing may follow that module
  (`[lints.clippy] items_after_test_module = "deny"`, `Cargo.toml:22-23`). New
  fixtures follow the `tests/{syntax,rt-error,rt-behavior}/<feature>/<name>/`
  convention: a directory is a test iff it contains a `project.json`
  (`scripts/test-accept.sh:229-235`).
- **Golden seeding.** `scripts/sync-goldens.sh` **never creates** golden files —
  pre-create an empty file per golden kind you want checked, then seed contents
  with `scripts/sync-goldens.sh target/debug/mfb <name-glob>`. Always pass a
  glob; with no filter it re-runs the entire ~15-minute suite.
- **Coverage.** The changed code must be *in the denominator*, which is the whole
  reason bug-347 gates this plan. After it lands:
  `sh scripts/coverage.sh && sh scripts/coverage-check.sh` (per-file 95% floor,
  `FLOOR` overridable) and the global `--fail-under-lines 95` gate.
- **Acceptance.** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  — the primary gate, ~15 min. `scripts/artifact-gate.sh target/debug/mfb` is the
  fast codegen gate (~5 min, execution-free).
- **Doc sync.** `.ai/specifications.md:12-18` makes this part of the Hard
  Completion Gate, not optional cleanup: a change to an observable contract
  updates the owning `src/docs/spec` topic **in the same change**. This plan
  obligates `src/docs/spec/tooling/01_project-manifest.md`,
  `src/docs/spec/package/02_binary-representation.md`, a new
  `src/docs/spec/package/NN_*.md` topic for section 18, and
  `src/docs/spec/package-manager/01_repository-protocol.md` for the new routes.
  Verify with `cargo build && cargo test --bin mfb spec`, then confirm
  `mfb spec <package> --all` renders with no leaked `[[` markers.

## Open Decisions

- **`description` required-ness rollout** — *recommended:* land optional +
  warning in D, migrate all 81 package manifests in F, then flip to hard error in
  F Phase 4. The alternative — flip immediately — makes one commit break 81
  fixtures and every intermediate state red. The user's requirement ("required
  for packages") is satisfied either way; only the path differs. Note this
  diverges from the project's standing pattern for new manifest fields, which is
  optional-with-a-documented-default: the worked precedent is plan-58-C's
  `maxBuffer`, added optional and set in exactly **one** fixture
  (`grep -rl maxBuffer tests --include=project.json | wc -l` → 1). F §2 argues
  why `description` warrants the exception; if that argument does not hold up on
  review, the fallback is to stop after E and leave the warning permanent, which
  costs nothing already built. (§F2, §F4)
- **Coordination with plan-60, which is in flight.** See §Prerequisites — the
  row is recorded there rather than resolved here, because it is a precondition,
  not a design fork.
- **Does `description` belong on `kind: "executable"` at all?** *Recommended:*
  allow it, ignore it, do not require it. Executables are never published to the
  registry, so the field is inert there — but forbidding it would make
  `kind` flips needlessly lossy. (§D4)
- **Search ranking** — *recommended:* exact ident match, then prefix match, then
  substring, then the existing edit-distance-1 `typosquat_candidates` primitive
  as the fuzzy tail. Alternative: SQLite FTS5, which is a real dependency
  decision against the lean-dependency posture. (§B3)
- **Whether the audit tab renders inclusion proofs inline or links to the JSON.**
  *Recommended:* render the proof path but link the raw JSON, so a third-party
  monitor can script against it. (§C3)

## Corrections

<!-- Filled in DURING execution. Every place the plan turned out to be wrong:
     the claim, what was actually true, and the evidence. A corrected number
     also needs a check of whether another letter's scope derived from it. -->

Found by a pre-execution review on 2026-07-21, before any code was written. Each
was verified against the tree, not reasoned about.

- **A Phase 2 named the wrong function.** It said to write
  `package_version_targets` rows at `repository/src/server.rs:2297-2306`. That
  range is inside `validate_package_request` (fn at `:2131`) — the `/validate`
  **dry-run**, which holds no transaction and no `package_version_id`, and whose
  contract is explicitly non-mutating (`:2270-2277`). The real discard site is
  `server.rs:2046-2051`; the write belongs in `store.publish_package_version`
  (`store.rs:1163-1230`), whose `vendor_hashes: &[String]` parameter must widen
  to `&[VendorBlobRef]`. A's Phase 2 is rewritten and the wrong site is called
  out inline so it is not re-derived.
- **A's Gotcha 2 rested on an impossible manifest.** "One vendored file listed
  under several platform locators" is rejected by
  `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` (`src/manifest/mod.rs:967-978`), so the
  stated acceptance criterion was unbuildable from a valid `project.json`. The
  underlying dedupe bug is real but reachable only via two *distinct* `source`
  filenames with byte-identical contents. Test shape corrected in A §3.
- **A's runtime proof would have passed vacuously.** It named `bindings/sqlite3`
  as "a real package with native libraries", but all its locators are
  `type: "system"` and `parse_vendor_blobs` emits only `vendor` entries
  (`repository/src/abi.rs:114`) — the table would be empty. Switched to
  `bindings/libsnd` (7 vendor locators, 3 arches, both libc values, and a macOS
  locator with no `arch` for the NULL case). A gained §3.1 stating that system
  locators are out of scope at all.
- **A promised two accessors no phase delivered.** `store.package_metadata` and
  `store.package_targets` were in `Produces:` and in no checkbox; B does not
  consume them. Removed.
- **B cited a per-owner rate limit for an anonymous route.**
  `BLOB_UPLOAD_PER_OWNER_MAX` is keyed `blob:{claims.sub}` (`server.rs:1529`),
  a key `/search` does not have. The applicable precedent is the per-IP block at
  `server.rs:625-627`. Also `server.rs:713` → `:719` for
  `into_make_service_with_connect_info`.
- **C's fingerprint section printed a command that cannot work.** It told users
  to paste the `/ident` server fingerprint into
  `mfb repo trust <registry-id> <fingerprint>`, but that argument is the **root**
  fingerprint — a different value from a different key (`src/cli/repo.rs:81-95`,
  `repository/src/client.rs:865-882`). In the one section whose entire purpose is
  not misleading the reader. C §4 now distinguishes the two; `plan-61/index.html`
  was corrected to show the root fingerprint.
- **D §4 sent Phase 1 to the wrong registry, and would have wasted it.** It
  grepped `02_error-codes.md` (the *runtime* `errorCode::` table) for
  `PROJECT_JSON_*`, got nothing, and concluded the scheme was unconfirmed. The
  compiler rule registry is `01_rule-codes.md`; codes are `2-200-NNNN`; next free
  is `2-200-0016`. The plan also never mentioned `src/rules/table.rs`, which must
  be edited for any new diagnostic. `build.rs` and `table_matches_registry` are
  not involved. See D §4.
- **F Phase 3's gate could not see the change it was gating.**
  `artifact-gate.sh` compares only `.ast`/`.ir`/`.hex`/native artifacts
  (`scripts/artifact-gate.sh:19`) — never `.mfp`, `.info`, `.audit`, or
  `build.log`. Its acceptance criterion "0 diffs" would have gone green with 39
  of the 41 churned goldens stale. Phase 3 now uses `test-accept.sh`.
- **F Phase 4 never said how a `warn` rule becomes an `error`.** D allocates
  `2-200-0016` as a warning; F flips it. Resolved as: mutate the severity of the
  same code in both `table.rs` and `01_rule-codes.md`, never allocate a second
  code, and expect a second round of `build.log` churn.
- **Pre-existing, found in passing:** `01_rule-codes.md:248-255` narrates the
  `2-200` block as `0001`-`0013` with "exactly six `warn` rules". The table runs
  to `0015` and there are **eight** warn rules — the prose omits `2-203-0115`
  and `2-203-0117`. The drift test only checks code and name presence, so it
  never caught this. D Phase 1 and F Phase 4 both now update it.
- **Measurements re-run and confirmed exact:** 81 `kind: "package"` manifests, 49
  golden-carrying fixture dirs, 16 `.mfp`, 2 `.hex`, 23 `.info`/`.audit`, highest
  MFPC section id 17. D's central premise — that a new section 18 is silently
  skipped — was re-verified against both section-table walkers in the tree
  (`src/binary_repr/reader.rs`, `repository/src/abi.rs`); there is no third
  reader, and neither validates ids nor asserts full consumption.
- **Prerequisite states moved** since authoring: plan-60 is now complete
  (MET); bug-347 is being fixed in the tree *right now* as an uncommitted edit,
  so `repository/` is no longer clean. See §Prerequisites.

## Summary

The real engineering risk is not the web interface — that is three server-side
HTML templates over JSON that a competent implementer can land in an afternoon.
It is in two places:

1. **The prerequisite.** This plan adds unauthenticated, HTML-emitting, XSS-
   sensitive code to a 13k-line crate whose 123+ tests have never run and which
   sits outside a 95% coverage gate. Everything else in this plan is routine;
   starting before bug-347 is fixed is not.
2. **The `description` rollout.** The format change itself (D) is verified
   non-breaking via the section-18 design and should churn zero goldens. The risk
   is all in F: 81 manifests, 41 goldens, and a tree-wide flip from warning to
   hard error. It is deliberately the last letter, its migration and its
   regeneration are separate phases, and it is the one place where the standing
   project pattern (new manifest fields are optional-with-a-default) is
   deliberately broken.

Left untouched: the container version, the header, MANIFEST section 1, the ABI
hash, every existing HTTP route, and the auth model. The plan adds one optional
MFPC section, some columns, six GET routes, and no credentials.

SUP-03 remains open. This plan makes registry equivocation *observable*; it does
not make it *impossible*.
