# plan-61-C: The web interface

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: plan-61-B
Produces:
- `GET /` (landing), `GET /search.html`, `GET /p/:ident` (package page, with an
  audit tab)
- `repository/src/web/mod.rs` — the render layer
- `escape_html()` and `safe_href()` helpers
- The `Content-Security-Policy` header applied to every HTML response

Renders plan-61-B's JSON as three server-side HTML pages. After this sub-plan the
site is complete and independently useful — everything except package
descriptions, which arrive in plan-61-F.

The single behavioral outcome: a browser with JavaScript disabled can reach the
landing page, search for a package, open its page, read its version history and
native target matrix, and switch to the audit tab — and a package whose `author`
field contains `<script>alert(1)</script>` renders that text visibly and inertly.

References:
- `plan-61-repo-web.md` §1 non-goals (no auth, no cookie, ever), §4
  (transparency as a security property)
- `planning/old-moved-to-src-spec/repo-base.md:21-28` — the single
  self-contained binary principle
- **`planning/plan-61/` — the HTML/CSS mockups.** Static reference renderings of
  every page, plus `DESIGN-RATIONALE.md` explaining the visual decisions. Read
  these before writing a template; do not redesign the pages from scratch. Scope
  and authority are defined in §3.1.

## Prerequisites

See `plan-61-repo-web.md` §Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-61-B complete | `curl -sf "$REPO/packages/alice%23pkg" \| head -c1` → succeeds against a local server | NOT MET |

## 1. Goal

- Three HTML pages, server-rendered, anonymous, functional with JavaScript
  entirely disabled.
- Publisher-controlled strings cannot execute script in a visitor's browser.

### Non-goals

- **No login, registration, or account page. Ever.** Not behind a flag, not
  "phase 2".
- **No cookie is set or read.** The server has zero CSRF surface today
  (`plan-61-repo-web.md` §1); this sub-plan is the one most likely to erode that
  by reflex, so it is called out here again.
- **No JavaScript is required for any core function.** Search is a `<form
  method="get">`. The audit "tab" is a separate URL, not a script toggle.
- No client-side framework, no bundler, no npm, no build step.
- No user-supplied HTML or Markdown rendering. `description` (plan-61-F) is
  rendered as plain text.
- No mutation. Every route is `GET`.

## 2. Design — XSS is the entire security story

Everything rendered on the package page is publisher-controlled: `author`, `url`,
`logical`, `source`, `ident`, and later `description`. `url` is capped at 2048
bytes (`repository/src/package.rs:118-129`) and is otherwise unvalidated — it
goes into an `href`, which makes `javascript:` the obvious vector.

Three independent layers, so that no single mistake is sufficient:

**1. Auto-escaping templates.** Use a template engine whose default is escaped
and whose bypass is explicit (`{{ x }}` escapes; `{{ x|safe }}` does not). This
inverts the failure mode: with hand-rolled `format!()` every interpolation must
*remember* to escape, and one forgotten call is a hole. See Open Decisions for
the engine choice and the dependency tradeoff.

**2. An href scheme allowlist.** `safe_href(url) -> Option<String>` returns
`Some` only for `http://` and `https://`. Everything else — `javascript:`,
`data:`, `vbscript:`, `file:`, protocol-relative `//evil`, and anything with
leading control characters or whitespace before the scheme — returns `None`, and
the template renders the URL as inert text instead of a link. Match on the
parsed scheme, never on a substring: `jAvAsCrIpT:`, `java\tscript:`, and
`%6aavascript:` must all be rejected.

**3. A strict Content-Security-Policy on every HTML response.** Because no page
needs JavaScript, the policy can be maximal:

```
default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self';
base-uri 'none'; frame-ancestors 'none'
```

Note there is **no `script-src` value permitting anything** — `default-src
'none'` denies script outright. This is the defense-in-depth layer: even a total
escaping failure cannot execute script. Do not weaken it by adding an inline
`<script>` or `'unsafe-inline'` later; if a feature seems to need one, it does not
belong on this site.

External links additionally carry `rel="noopener noreferrer nofollow"`.

## 3. Design — pages

**`GET /`** — title, a `<form method="get" action="/search.html">` with a single
`q` field, and the server fingerprint from `/ident`
(`repository/src/server.rs:731`).

**`GET /search.html?q=`** — renders `SearchResponse`. Each row links to
`/p/:ident`. Empty query renders the form and no results, never the full table.

**`GET /p/:ident`** — renders `PackageDetailResponse`: latest version prominently,
then author/url/description, then the full version table (**every** version,
including yanked, with its state rendered as a visible label — see
`plan-61-repo-web.md` §4), then the native blob table with its os/arch/libc
matrix. `arch = NULL` renders as "any", not as blank.

**`GET /p/:ident/audit`** — the audit tab, rendered from `PackageAuditResponse`:
log checkpoint, per-version inclusion proofs, release-state transitions, and
ident-key rotations. Links to the raw JSON endpoint so a third-party monitor can
script against it.

### 3.1 The mockups are UI-only — this document owns everything else

`planning/plan-61/` holds a finished static rendering of every page. It exists so
the implementer does not re-derive the layout, the state cues, the responsive
strategy, or the fingerprint copy. Map each file to the route above:

| Mockup | Route |
|---|---|
| `index.html` | `GET /` |
| `search.html`, `search-noresults.html`, `search-empty.html` | `GET /search.html?q=` — three renderings of **one** route, split into three files only so each state is visible |
| `package.html` | `GET /p/:ident` |
| `audit.html` | `GET /p/:ident/audit` |
| `style.css` | served at `/style.css` |

**Authority.** The mockups are normative for *appearance only* — markup
structure, class names, copy, and the CSS. They are **not** normative for
anything else, and where they disagree with this plan, this plan wins:

- **Routes.** §3 above is the route table. Any path, form `action`, or `href` in
  the mockups is illustrative; re-point them at the real routes.
- **Data shapes.** Every value in the mockups is placeholder content. The real
  fields come from plan-61-B's `SearchResponse`, `PackageDetailResponse`, and
  `PackageAuditResponse`.
- **Escaping, `safe_href`, and the CSP.** §2 is the security contract. The
  mockups happen to satisfy it — no `<script>`, no inline `style=`, no external
  fonts or images — but that is a property to *preserve*, not evidence that a
  generated page is safe. Every value the templates interpolate still goes
  through the §2 layers.

Two notes for the implementer: `DESIGN-RATIONALE.md` describes a `preview/`
subdirectory that is not present and is not part of the handoff — ignore it. And
the mockups' hostile-content fixture (an `author` of `<script>alert(1)</script>`)
is the same case Phase 3's XSS regression test asserts; keep them in sync.

## 4. The fingerprint must not overclaim

The landing page shows the server fingerprint, but the wording matters. The
fingerprint is TOFU-pinned (`src/docs/spec/package-manager/01_repository-protocol.md:103-118`),
and **an attacker serving you a forged page serves you their fingerprint too**.
The page therefore authenticates nothing about itself.

Render it as a value to compare against an out-of-band source — never as "this
registry is verified". Getting this wrong turns a useful convenience into
security theater that actively misleads.

**Two different fingerprints — do not conflate them.** `mfb repo trust` takes the
**root** fingerprint, not the server fingerprint this page displays:

```
mfb repo trust <registry-id> <root-fingerprint>
```

`src/cli/repo.rs:81-95` binds its second argument as `root_fingerprint`, and
`client::trust_registry` (`repository/src/client.rs:865-882`) verifies `root.json`
against it — then *separately* checks the pinned `/ident` server key against the
root-delegated one. Two values, two keys
(`src/docs/spec/package-manager/01_repository-protocol.md:900-907`). Pasting the
`/ident` fingerprint into that command simply fails.

So the landing page must do one of:

- **(recommended)** surface the **root** fingerprint from `/root.json` as the
  value to compare and paste, since that is the one the pinning command consumes;
  or
- keep showing the `/ident` server fingerprint, but present it purely as a value
  to *compare* and do not print a `mfb repo trust` command next to it.

Showing the server fingerprint above a command that takes the root fingerprint is
the worst option: it reads as a verified copy-paste and then errors out. Phase 2
must resolve this explicitly. **`planning/plan-61/index.html:67` currently has the
wrong form** — fix the mockup in the same change.

## Phases

> Tick `- [x]` in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Escaping primitives and CSP

Lands first and alone because every later phase depends on it being right, and
because it is the only part with real security consequences.

- [ ] Create `repository/src/web/mod.rs`. Add `safe_href(&str) -> Option<String>`
      implementing the §2 allowlist.
- [ ] Add the CSP header from §2 to every HTML response via a shared response
      builder — not per-handler, so a future page cannot forget it.
- [ ] Tests, and these are the point of the phase — a table-driven test asserting
      **rejection** of: `javascript:alert(1)`, `JaVaScRiPt:alert(1)`,
      `java\tscript:alert(1)`, `java\nscript:`, ` javascript:` (leading space),
      `\x01javascript:`, `data:text/html,<script>`, `vbscript:msgbox`,
      `//evil.example`, `file:///etc/passwd`; and **acceptance** of
      `http://x.example`, `https://x.example/a?b=c#d`.
- [ ] Tests: every HTML response carries the CSP header, asserted through the
      shared builder.

Acceptance: the rejection table passes in full, and a response built through the
shared builder without explicitly requesting CSP still carries the header.
Commit: —

### Phase 2 — Landing and search pages

- [ ] Add `GET /style.css`, serving the stylesheet from `planning/plan-61/style.css`
      embedded in the binary (`include_str!`) with `Content-Type: text/css`. The
      §2 CSP is `style-src 'self'` with no `'unsafe-inline'`, so the stylesheet
      **must** be a real served route — there is no inline fallback. Keeping it
      compiled in preserves the single self-contained binary principle.
- [ ] Add `GET /` rendering the title, search form, and fingerprint with the §4
      wording. Port the markup from `planning/plan-61/index.html` (§3.1).
- [ ] Add `GET /search.html?q=` rendering `SearchResponse`. Port the markup from
      `planning/plan-61/search.html`, and its two states from
      `search-noresults.html` and `search-empty.html`.
- [ ] Register both in the route table (`repository/src/server.rs:672-704`).
      Confirm the HTML routes do not shadow any existing JSON route — in
      particular that adding a `/` handler does not disturb `/health` or
      `/ident`.
- [ ] Tests: the landing page renders the fingerprint from `/ident`; a search for
      a nonexistent term renders a "no results" page with HTTP 200, not 404; an
      empty `q` renders the form with no results.
- [ ] Tests: a package whose ident contains HTML metacharacters renders escaped
      in the results list.

Acceptance: with JavaScript disabled, submitting the landing form navigates to a
results page listing matching packages.
Commit: —

### Phase 3 — Package page and audit tab (largest surface)

Last, because it renders the most publisher-controlled fields.

- [ ] Add `GET /p/:ident` per §3, porting the markup from
      `planning/plan-61/package.html` (§3.1). Handle `%23` in the path as in
      plan-61-B Phase 1.
- [ ] Render the version table with **every** state visible and labeled. A yanked
      version must be visually distinguished but present.
- [ ] Render the native blob table: one row per target, `arch = NULL` shown as
      "any", `libc = NULL` shown as "—".
- [ ] Add `GET /p/:ident/audit` per §3, porting the markup from
      `planning/plan-61/audit.html`, and linking to `GET /packages/:ident/audit`
      for the raw JSON.
- [ ] Tests: **the XSS regression test** — publish a fixture whose `author` is
      `<script>alert(1)</script>` and whose `url` is `javascript:alert(1)`, then
      assert the rendered page contains the escaped text, contains no `<script`
      substring, and renders the url as text rather than an anchor.
- [ ] Tests: a yanked version appears in the rendered table; a package with two
      platform targets renders two rows; unknown ident renders a 404 page.

Acceptance: the XSS regression test passes, and a package page for a real
published package with native libraries shows its full version history and target
matrix with JavaScript disabled.
Commit: —

### Phase 4 — Spec sync

- [ ] Document the HTML routes in
      `src/docs/spec/package-manager/01_repository-protocol.md`, stating
      explicitly that they are anonymous, read-only, and carry no credential.
- [ ] Record the CSP and the no-cookie invariant in that topic, so a future
      change that would break it has to argue with the spec first.
- [ ] Verify: `cargo build && cargo test --bin mfb spec`; `mfb spec
      package-manager --all` renders with no leaked `[[` markers.

Acceptance: `cargo test --bin mfb spec` passes; the topic documents all HTML
routes and the no-cookie invariant.
Commit: —

## Validation Plan

- Tests: inline `#[cfg(test)] mod tests` in `repository/src/web/mod.rs`. The
  href-rejection table and the XSS regression test are the two that matter.
- Coverage check: `sh scripts/coverage.sh && sh scripts/coverage-check.sh`; the
  new `web/` module must appear in the report.
- Runtime proof: start `mfb-repo`, publish a package with a hostile `author` and
  `url`, open all four pages in a real browser with JavaScript disabled, and
  confirm: pages render, no script executes, the hostile url is not a link, and
  the browser console shows no CSP violations for legitimate content.
- Doc sync: `src/docs/spec/package-manager/01_repository-protocol.md`.
- Acceptance: `cargo test -p mfb_repository`;
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Template engine vs. hand-rolled `format!`** — *recommended:* a compile-time
  auto-escaping engine (askama or maud). The argument is not ergonomics, it is
  that auto-escaping makes the safe path the default and the unsafe path
  explicit (`{{ x|safe }}`), whereas hand-rolled string building makes every
  interpolation a place to forget. The cost is one compile-time proc-macro
  dependency and no runtime dependency, which is a defensible reading of the
  lean-dependency posture — note `repository/Cargo.toml:13-17` already
  feature-gates the AWS SDK specifically to keep weight out of the core build, so
  the precedent is "gate heavy optional things", not "never add a dependency".
  Alternative: hand-rolled with a single `escape_html()` and a lint/test that
  greps templates for unescaped interpolation. Decide in Phase 1, before any
  template is written.
- **Serve CSS inline in a `<style>` tag or as a static `/style.css`?**
  *Recommended:* a single `/style.css` route returning a `&'static str`, so
  `style-src 'self'` needs no `'unsafe-inline'`. Keeps the single-binary property
  (no `ServeDir`, no asset directory).

## Corrections

- *(none yet)*
