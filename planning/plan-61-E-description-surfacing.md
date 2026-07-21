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
| plan-61-C complete | `curl -sf "$REPO/p/alice%23pkg" \| grep -q '<html'` → succeeds | NOT MET |
| plan-61-D complete | `grep -q SECTION_PACKAGE_META src/binary_repr/mod.rs` → found | NOT MET |

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

- [ ] In the publish path (`repository/src/server.rs`, near the author/url write
      from plan-61-A Phase 3), parse section 18 and persist
      `package_versions.description`.
- [ ] Populate `description` in `PackageDetailResponse` (latest version's value)
      and in `SearchResponse` results, clamped to a preview length. Define the
      clamp as a named constant next to the existing caps.
- [ ] Extend `mfb-repo backfill-metadata` to fill `description` from stored
      blobs; absence of section 18 is a normal outcome, not a skip-with-warning.
- [ ] Tests: publish with a description → persisted and returned; publish without
      one → `null`, and the response still validates; backfill populates
      descriptions for pre-existing versions and leaves pre-plan-61-D packages
      NULL without logging an error.

Acceptance: `curl -s "$REPO/packages/alice%23pkg" | grep description` shows the
published text, and a package published before plan-61-D returns `null`.
Commit: —

### Phase 2 — Search on description

- [ ] Add description matching to `store.search_packages`, ranked **below** ident
      and owner matches per `plan-61-B-read-endpoints.md` §3.
- [ ] Tests: a package findable only by a word in its description is returned; an
      ident match still outranks a description match for the same query.

Acceptance: searching a distinctive word that appears only in a package's
description returns that package, ranked below any ident match for the same
query.
Commit: —

### Phase 3 — Render

- [ ] Render the description on the package page and the clamped preview in
      search results, through the auto-escaping template path from
      plan-61-C Phase 1.
- [ ] Tests: **extend plan-61-C's XSS regression test** to cover description —
      publish a fixture whose description is `<img src=x onerror=alert(1)>` and
      assert the page renders it as visible inert text with no `onerror`
      attribute in the output.
- [ ] Tests: a package with no description renders the page without an empty
      section or a stray "None".

Acceptance: the extended XSS regression test passes, and a real package's
description is visible on its page in a browser with JavaScript disabled.
Commit: —

### Phase 4 — Spec sync

- [ ] Update the response shapes in
      `src/docs/spec/package-manager/01_repository-protocol.md` to show
      `description` as populated, and document the search-preview clamp.
- [ ] Verify: `cargo test --bin mfb spec`; `mfb spec package-manager --all`
      renders with no leaked `[[` markers.

Acceptance: `cargo test --bin mfb spec` passes.
Commit: —

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

- *(none yet)*
