# plan-61-B: Public read endpoints

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: plan-61-A
Produces:
- `GET /search?q=&limit=&offset=` → `SearchResponse`
- `GET /packages/:ident` → `PackageDetailResponse`
- `GET /packages/:ident/audit` → `PackageAuditResponse`
- `store.search_packages(query, limit, offset)`
- `store.package_detail(ident)`, `store.package_audit(ident)`

Adds the three anonymous JSON endpoints the web UI renders. **These are built as
a JSON API first and rendered second, on purpose**: the design doc at
`planning/old-moved-to-src-spec/repository.md:66` already specifies `mfb pkg add
<name>` performing cross-owner search, and no such endpoint exists. The CLI needs
this whether or not a web UI is ever built. The HTML in plan-61-C is the second
consumer, not the reason.

The single behavioral outcome: `curl -s "$REPO/search?q=sql"` returns matching
packages with no credential, and `curl -s "$REPO/packages/alice%23sqlite"`
returns every version including yanked ones.

References:
- `plan-61-repo-web.md` §Prerequisites, §4 (transparency as a security property)
- `repository/src/server.rs:672-704` — the route table
- `repository/src/server.rs:504-536` — `IndexResponse`, the shape to mirror

## Prerequisites

See `plan-61-repo-web.md` §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-61-A complete | `grep -q package_version_targets repository/src/store.rs` → found | NOT MET |

## 1. Goal

- Three anonymous `GET` routes returning JSON, requiring no `sessionToken`, no
  bearer header, and no cookie.
- Every version is listed regardless of release state, per `plan-61-repo-web.md` §4.

### Non-goals

- **No authentication on any route in this sub-plan.** Not optional auth, not
  "auth if present". These routes never read a credential of any kind.
- **No cookie is read or set anywhere.** See `plan-61-repo-web.md` §1 non-goals
  for why this is load-bearing rather than stylistic.
- No HTML — that is plan-61-C. B returns `Json<T>` only.
- No mutation. All three routes are `GET`.
- No change to `GET /index/:ident`. It keeps its exact current shape; clients
  depend on it.

## 2. Current state

There is **no enumerate route of any kind**. `GET /index/:ident`
(`repository/src/server.rs:915`) requires an exact `<owner>#<package>` and 400s
otherwise (`:919-923`). The route table is `repository/src/server.rs:672-704`.

The only fuzzy-match primitive is `store.typosquat_candidates(ident)`, used
warn-only during publish (`repository/src/server.rs:2104-2115`), which finds
existing idents within edit distance 1.

Rate limiting already exists as a sliding window, keyed per caller — the server
is served with `into_make_service_with_connect_info::<SocketAddr>()`
(`repository/src/server.rs:719`) precisely so handlers can key on the peer IP.

**Use the per-IP precedent, not the per-owner one.** `/search` is anonymous, so
there is no `claims.sub` to key on. The applicable precedents are
`REGISTER_PER_IP_MAX = 20`, `LOGIN_PER_IP_MAX = 30`, and
`AUTH_GLOBAL_CEILING = 2000` (`server.rs:625-627`), which throttle the other
unauthenticated routes. An earlier draft cited `BLOB_UPLOAD_PER_OWNER_MAX = 120`
as "the existing precedent"; that limiter is keyed **per owner** — literally
`&format!("blob:{}", claims.sub)` at `server.rs:1529` — and its key is
unavailable here. The window shape is shared either way:
`allow(key, max, window_secs)` (`server.rs:40`) with a 60-second window.

Existing transparency data available with no new capture: the
`release_state_changes` table, the `ident_chain` table (also surfaced by
`GET /idents/:owner`, `server.rs:1626`), `log_entries`, and the existing
`/log/checkpoint`, `/log/proof/:index`, `/log/publish` routes.

### Verified properties

- **VERIFIED — no existing handler reads a cookie.** Read the route table at
  `repository/src/server.rs:672-704` and the auth helper each authenticated
  handler calls: every one takes `sessionToken` from the JSON body, except
  `PUT /blob/:hash` (`:1521-1524`) which uses `Authorization: Bearer`. Adding
  anonymous GETs therefore cannot create a CSRF vector.
- **UNVERIFIED — whether `typosquat_candidates` is efficient enough to serve on
  a request path.** It was written for a single call during publish, not for a
  per-keystroke search. Phase 2 must read it and measure before wiring it into
  the fuzzy tail; if it is a full table scan with per-row edit distance, gate it
  behind "only when exact+prefix+substring return nothing".

## 3. Design — search

Ranking, in order (recommended; see `plan-61-repo-web.md` §Open Decisions):

1. exact `ident` match
2. `ident` prefix match
3. `ident` substring match
4. owner display-name match
5. edit-distance-1 fuzzy tail via `typosquat_candidates`, **only if** 1–4 are
   empty, and only if Phase 2's measurement says it is affordable

Explicitly rejected: SQLite FTS5. It is a real dependency and a schema
commitment, against the lean-dependency posture
(`repository/Cargo.toml:13-17` already feature-gates the AWS SDK specifically to
keep it out of the core build). Revisit only if `description` search (plan-61-E)
proves substring matching inadequate.

`limit` is capped server-side. An uncapped `limit` on an anonymous enumerate
route is a trivial resource-exhaustion lever, and `offset`-based paging over a
growing table is fine at this scale.

### Response shapes

Mirror `IndexResponse` (`repository/src/server.rs:504-536`) in naming style
(camelCase fields).

```
SearchResponse   { query, total, results: [{ ident, owner, latestVersion,
                                             description, publishedAt }] }
PackageDetailResponse { ident, owner, identKey, identFingerprint,
                        serverFingerprint, author, url, description,
                        latestVersion,
                        versions: [{ version, hash, publishedAt, state,
                                     abiIndex, logEntry,
                                     targets: [{ os, arch, libc, libType,
                                                 logical, source, blobHash }] }] }
PackageAuditResponse  { ident, logCheckpoint: { size, rootHash, signature },
                        publishes: [{ version, index, leafHash }],
                        stateChanges: [{ version, fromState, toState, at }],
                        identChain: [{ oldKey, newKey, signature, issued }] }
```

`description` is present in the shape from day one and is `null` until
plan-61-E. That is deliberate: it means E adds no field and breaks no consumer.

## 4. Design — every version, always

`PackageDetailResponse.versions` lists **every** version regardless of `state`,
including yanked and superseded. Per `plan-61-repo-web.md` §4: a UI that silently
omits non-current versions reproduces exactly the truncation that the open SUP-03
downgrade attack performs. `state` is a field for the client to render, never a
filter the server applies.

The audit route exposes log inclusion proofs, not just a prose history — a
rendered `logEntry` index that cannot be independently verified proves nothing.

## Phases

> Tick `- [x]` in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Package detail and audit

The read-only routes over data that already exists. Lands before search because
it has no ranking design risk.

- [ ] Add `store.package_detail(ident)` in `repository/src/store.rs`: joins
      `packages`, `owners`, `package_versions`, `package_version_targets`.
      Returns every version, ordered newest-first by `created_at`.
- [ ] Add `store.package_audit(ident)`: joins `log_entries`,
      `release_state_changes`, `ident_chain`.
- [ ] Add `GET /packages/:ident` and `GET /packages/:ident/audit` handlers in
      `repository/src/server.rs`, registered in the route table at `:672-704`.
      Both anonymous. Percent-decoded `:ident` must accept the `#` in
      `<owner>#<package>`. `GET /index/:ident` (`server.rs:917-923`) already
      proves axum's `Path<String>` handles `%23`, so mirror it rather than
      re-deriving it; still assert it in a test.
- [ ] **Check the route conflict before anything else in this phase.** The table
      already holds `POST /packages/transfer/offer` and
      `/packages/transfer/accept` (`server.rs:698-699`). Adding
      `GET /packages/:ident` puts a parameter segment as a sibling of the static
      `transfer` segment. matchit (axum 0.7, `repository/Cargo.toml:23`) is
      expected to accept this — static wins over param — but a conflict there
      **panics inside `Router::route` at startup**, not in a handler test, so it
      would surface as a dead server rather than a red test. Start the server
      once, immediately, and confirm both transfer routes still resolve. If it
      does conflict, rename the read routes (`/pkg/:ident`) rather than moving
      the existing POST routes, which are a published wire contract.
- [ ] 404 with the standard `{error: "..."}` shape (`server.rs:585`) for an
      unknown ident. Do not leak whether an *owner* exists separately from a
      package.
- [ ] Tests: a yanked version appears in the response; a package with two
      platform targets renders both; `%23` in the path resolves; unknown ident
      404s; **no route accepts or requires a `sessionToken`**.

Acceptance: `curl -s "$REPO/packages/alice%23pkg"` with no credentials returns
every version including a yanked one, with its target rows; and the same request
carrying a valid `sessionToken` behaves identically (proving the route ignores
credentials rather than accepting them).
Commit: —

### Phase 2 — Search

- [ ] Read `store.typosquat_candidates` and determine its cost. Record the
      finding in this file — if it is a full scan, the fuzzy tail is gated behind
      empty exact/prefix/substring results, or dropped.
- [ ] Add `store.search_packages(query, limit, offset)` implementing the ranking
      in §3.
- [ ] Add `GET /search?q=&limit=&offset=`. Cap `limit` server-side; document the
      cap in the spec topic. Empty or whitespace-only `q` returns an empty result
      set, not the whole table.
- [ ] Apply the existing sliding-window rate limit to `/search`, keyed on the
      **peer IP** per the `REGISTER_PER_IP_MAX` / `LOGIN_PER_IP_MAX` precedent
      (`server.rs:625-627`) — not the per-owner `BLOB_UPLOAD_PER_OWNER_MAX`,
      whose `claims.sub` key does not exist on an anonymous route (§2). Search is
      the only route here that does real query work per request.
- [ ] Tests: exact beats prefix beats substring; `limit` above the cap is
      clamped, not honored; empty query returns empty; a query with SQL
      metacharacters (`%`, `_`, `'`) is parameterized and returns no rows rather
      than erroring or matching everything.

Acceptance: `curl -s "$REPO/search?q=sql"` returns ranked matches anonymously;
`?limit=100000` returns at most the cap; `?q=%25` returns an empty set rather
than every package.
Commit: —

### Phase 3 — Spec sync

- [ ] Add the three routes to the endpoint table in
      `src/docs/spec/package-manager/01_repository-protocol.md` (table at
      `:64-98`), including the `limit` cap and the "anonymous, no credential"
      property.
- [ ] Note in that topic that these routes deliberately list all release states,
      with a pointer to why (`plan-61-repo-web.md` §4 / bug-189 SUP-03).
- [ ] Verify: `cargo build && cargo test --bin mfb spec`, then
      `mfb spec package-manager --all` renders with no leaked `[[` markers.

Acceptance: `cargo test --bin mfb spec` passes and the rendered topic lists all
three routes.
Commit: —

## Validation Plan

- Tests: inline `#[cfg(test)] mod tests` in `repository/src/store.rs` and the new
  handler module. Negative cases: unknown ident, empty query, over-cap limit, SQL
  metacharacters, credential-bearing request to an anonymous route.
- Coverage check: `sh scripts/coverage.sh && sh scripts/coverage-check.sh`;
  confirm the new handler code is in the denominator.
- Runtime proof: start `mfb-repo`, publish two versions of a package, yank one,
  then `curl` all three routes with no credentials and confirm the yanked version
  is present and the audit route returns a checkpoint plus an inclusion proof.
- Doc sync: `src/docs/spec/package-manager/01_repository-protocol.md`.
- Acceptance: `cargo test -p mfb_repository` and
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Search ranking** — see `plan-61-repo-web.md` §Open Decisions. Resolve in
  Phase 2 with the measurement, not by argument.
- **Should `/search` match on `description` once plan-61-E lands?**
  *Recommended:* yes, ranked below ident and owner. It is a one-line `LIKE`
  addition and is the main reason a user searches at all. Deferred to E.

## Corrections

- *(none yet)*
