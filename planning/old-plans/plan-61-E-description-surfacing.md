# plan-61-E: Surface `description` through the server and the site

Last updated: 2026-07-21
Effort: small (<1h)
Depends on: plan-61-C + plan-61-D
Produces:
- `package_versions.description` populated at publish time
- `description` non-null in `SearchResponse` and `PackageDetailResponse`
- Description rendered on the package page and in search results
- `/search` matching on description text

Joins plan-61-D's artifact field to plan-61-A's database column and plan-61-C's
pages. This is deliberately small: A created the column NULL-able, B put
`description` in the JSON shapes from day one, and C left a slot for it — so this
sub-plan adds no field to any contract and breaks no consumer.

The single behavioral outcome: publishing a package whose `project.json` has a
description makes that text appear on its package page and makes the package
findable by searching for a word from it.

References:
- `plan-61-A-server-metadata-capture.md` — the column
- `plan-61-B-read-endpoints.md` §3 — the response shapes
- `plan-61-C-web-ui.md` §2 — the escaping rules that apply to this new
  publisher-controlled string

## Prerequisites

See `plan-61-repo-web.md` §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-61-C complete | `curl -sf "$REPO/p/alice%23pkg" \| grep -q '<html'` → succeeds | **MET** (2026-07-21): C is archived to `planning/old-plans/`; its own runtime proof ran exactly this request against a live `mfb-repo` and got a full HTML page. |
| plan-61-D complete | `grep -q SECTION_PACKAGE_META src/binary_repr/mod.rs` → found | **MET** (2026-07-21, re-run): found. D is archived; `49a710136`. |

## 1. Goal

- Description flows artifact → database → JSON → HTML, and is searchable.

### Non-goals

- **No Markdown or HTML rendering of the description.** It is plain text,
  escaped like every other publisher-controlled string. Rendering publisher
  markup would reintroduce the XSS surface plan-61-C closed.
- No change to any response shape — `description` already exists in both, as
  `null`.
- No truncation policy change in search results beyond a plain character clamp.
- Still no `license`/`keywords`.

## 2. Design

Three small joins:

1. **Publish** reads section 18 via the repository reader added in plan-61-D and
   writes `package_versions.description`, inside the existing publish
   transaction alongside author/url from plan-61-A Phase 3.
2. **Endpoints** stop returning `null`. `PackageDetailResponse.description` comes
   from the latest version's row; `SearchResponse.results[].description` is
   clamped to a preview length server-side so a 4096-byte description cannot
   bloat a 50-result page.
3. **Pages** render it. Search results show the clamped preview; the package page
   shows the full text.

The description shown for a package is **the latest version's**, not a
package-level value — descriptions legitimately change between versions, and
inventing a package-level one would require deciding which version wins. The
version table on the package page can show per-version descriptions where they
differ.

## 3. Backfill

plan-61-A Phase 4 added `mfb-repo backfill-metadata`. Extend it to populate
`description` from section 18 of stored blobs. Packages published before
plan-61-D simply have no section 18 and stay NULL — that is correct, not a
failure, and the backfill must not log it as an error.

## Phases

> Tick `- [x]` in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Ingest and expose

- [x] In the publish path (`repository/src/server.rs`, near the author/url write
      from plan-61-A Phase 3), parse section 18 and persist
      `package_versions.description`.
- [x] Populate `description` in `PackageDetailResponse` (latest version's value)
      and in `SearchResponse` results, clamped to a preview length. Define the
      clamp as a named constant next to the existing caps.
- [x] Extend `mfb-repo backfill-metadata` to fill `description` from stored
      blobs; absence of section 18 is a normal outcome, not a skip-with-warning.
- [x] Tests: publish with a description → persisted and returned; publish without
      one → `null`, and the response still validates; backfill populates
      descriptions for pre-existing versions and leaves pre-plan-61-D packages
      NULL without logging an error.

Acceptance: **MET**, and verified over real HTTP. Against a live `mfb-repo`
holding a `.mfp` built by a real `mfb build`:
`curl /packages/alice%23toolbox` returns
`"description":"Zygomorphic layout primitives with native BLAS kernels."`.
`a_description_reaches_the_json_and_the_page` covers the null case: a package
with no description reports `null` and its page renders the
"No description provided." placeholder — asserted with
`!body.contains("None")`, so a stray `Option` rendering would fail it.
`backfill_fills_descriptions_and_stays_quiet_about_packages_that_have_none`
pins the §3 rule: a blob with no section 18 leaves NULL and is **not** counted
or logged as a skip.
Commit: `1c4c24494`

### Phase 2 — Search on description

- [x] Add description matching to `store.search_packages`, ranked **below** ident
      and owner matches per `plan-61-B-read-endpoints.md` §3.
- [x] Tests: a package findable only by a word in its description is returned; an
      ident match still outranks a description match for the same query.

Acceptance: **MET.** Live: `curl "/search?q=zygomorphic"` — a word appearing
only in the description — returns `alice#toolbox`.
`search_matches_descriptions_but_ranks_them_below_idents` also pins the ranking
with a term that is one package's *ident* and another's *description*, and
asserts the ident-matching package comes first.
Commit: `1c4c24494`

### Phase 3 — Render

- [x] Render the description on the package page and the clamped preview in
      search results, through the auto-escaping template path from
      plan-61-C Phase 1.
- [x] Tests: **extend plan-61-C's XSS regression test** to cover description —
      publish a fixture whose description is `<img src=x onerror=alert(1)>` and
      assert the page renders it as visible inert text with no `onerror`
      attribute in the output.
- [x] Tests: a package with no description renders the page without an empty
      section or a stray "None".

Acceptance: **MET.** The plan-61-C XSS fixture now carries
`description: <img src=x onerror=alert('desc')>` alongside the hostile author
and url, and `a_hostile_author_and_url_render_inert` asserts it renders as
visible escaped text with no `<img` in the output. Live, the real description
renders in both places — `pkg-desc">Zygomorphic layout primitives…` on the
package page and `result__desc">…` in search results — on pages that contain no
script at all, so JavaScript being disabled changes nothing.
Commit: `1c4c24494`

### Phase 4 — Spec sync

- [x] Update the response shapes in
      `src/docs/spec/package-manager/01_repository-protocol.md` to show
      `description` as populated, and document the search-preview clamp.
- [x] Verify: `cargo test --bin mfb spec`; `mfb spec package-manager --all`
      renders with no leaked `[[` markers.

Acceptance: **MET** — `cargo test --bin mfb spec` → 48 passed, 0 failed;
`mfb spec package-manager --all` renders with 0 leaked `[[` markers. The topic
now records that `description` comes from the signed artifact rather than the
publish request, the below-ident search ranking, the newest-version scoping, and
the 200-character preview clamp.
Commit: `1c4c24494`

## Validation Plan

- Tests: inline tests in the publish path, `repository/src/store.rs`, and
  `repository/src/web/`. The XSS case is the one that matters.
- Coverage check: `sh scripts/coverage.sh && sh scripts/coverage-check.sh`.
- Runtime proof: add a description to `bindings/sqlite3`, publish it to a local
  `mfb-repo`, search for a word from the description, and open the package page
  in a browser.
- Doc sync: `src/docs/spec/package-manager/01_repository-protocol.md`.
- Acceptance: `cargo test -p mfb_repository`;
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Search-result preview length** — *recommended:* 200 characters, clamped
  server-side on a character boundary (not a byte boundary — the description is
  UTF-8 and a byte clamp can split a grapheme).

## Corrections

- **Description search is scoped to the newest version.** §2 says the *displayed*
  description is the latest version's but does not say which versions `/search`
  matches against. Matching any version would keep a package findable by a word
  its current release no longer contains — a stale-index behaviour a user would
  read as a bug. The `EXISTS` subquery selects the newest version explicitly.
- **The runtime proof used a purpose-built package, not `bindings/sqlite3`.**
  Same reason as plan-61-D's: `bindings/sqlite3` and `bindings/libsnd` are
  modified in the working tree by another agent, and editing a file someone else
  is mid-edit on is how work gets lost. The proof is unweakened — it ran a real
  `mfb build`, a real `mfb-repo backfill-metadata`, and real `curl`s against a
  live server.
- **`PublishMetadata` gained the field rather than the signature growing.** The
  struct introduced in plan-61-A Phase 3 was created for exactly this: E adds
  `description` to it and no call site's arity changes.
