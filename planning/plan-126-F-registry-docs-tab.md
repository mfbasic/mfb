# plan-126-F: The registry "Docs" tab

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-126-E

Adds a third tab to the registry package page rendering the latest active version's
documentation, as a native part of the site rather than an embedded artifact. This
is the feature the whole of plan-126 exists to make possible.

Behavioral outcome: `GET /p/<ident>/docs` renders the `DOC` blocks the package's
author wrote — groups, signatures, parameters, returns, errors, examples and
callouts — under the site's existing shell and CSP, with no script and no inline
style. A package whose latest active version carries no documentation gets the same
tab, saying so explicitly.

References:

- `repository/src/web/mod.rs:1-27` — the module doc: three independent XSS layers,
  and why `PreEscaped` is the greppable bypass to avoid. Read it before writing a
  line of this renderer.
- `src/doc/html.rs` — the compiler's standalone renderer, the reference for *what* a
  doc page contains. It is not reused; see § Non-goals.
- `repository/src/web/mod.rs:389-400` — `package_tabs`, the strip being widened.

## Prerequisites

See plan-126-A § Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-126-E complete (docs are stored and retrievable) | `grep -c latest_active_version_docs repository/src/store.rs` → ≥1 | NOT MET |
| plan-126-A complete (`latest_active_version` exists) | `grep -c 'fn latest_active_version' repository/src/store.rs` → 1 | NOT MET |

If either is not complete, this sub-plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report
> the status of *all* prerequisites if you stop.

## 1. Goal

- `GET /p/<ident>/docs` renders the latest active version's `DocPage` in maud, under
  the existing `page` shell (`repository/src/web/mod.rs:148`), with the site's CSP
  unchanged.
- The tab strip carries three tabs; the current one is marked `aria-current="page"`.
- A package with no documentation on its latest active version renders the tab with
  explicit copy stating the author did not include documentation.
- `GET /packages/<ident>/docs` serves the same content as JSON, keeping the site's
  every-HTML-page-mirrors-a-JSON-page property.

### Non-goals (explicit constraints)

- **The CSP does not change.** `repository/src/web/mod.rs:44` stays
  `default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self';
  base-uri 'none'; frame-ancestors 'none'`. No `<script>`, no `'unsafe-inline'`, no
  inline `<style>`.
- **`maud::PreEscaped` is not used anywhere in this sub-plan.** Doc prose,
  signatures, examples and parameter descriptions are 100% publisher-controlled
  text; auto-escaping is the whole defense. A `grep -c PreEscaped repository/src/web/`
  must not increase.
- **`src/doc/html.rs` is not reused or embedded.** It emits an inline `<style>`
  (`:172`, `STYLE` const at `:247`) which the CSP blocks outright, and injecting its
  HTML would require `PreEscaped`. The shared thing is the *model* (plan-126-D), not
  the markup.
- **No version selector.** Only the latest active version is served, per the
  decision that the registry is not a docstore for every version. The URL carries no
  version segment, so adding one later is additive.
- **The Overview and Audit tabs are unchanged**, including the Overview's complete,
  unfiltered version table.

## 2. Current State

### The page shell and tab strip

`repository/src/web/mod.rs:148 page(title, registry_id, body)` is the shared shell;
`html_response` (`:60`) attaches the CSP, referrer policy and nosniff headers so a
new page cannot forget them. `package_tabs(ident, audit: bool)` (`:389-400`) renders
two tabs from a bool, with the comment "The audit 'tab' is a separate URL, not a
script toggle — the site has no script."

Routes are registered at `repository/src/server.rs:875-876`:

```rust
.route("/p/:ident", get(package_page_html))
.route("/p/:ident/audit", get(package_audit_html))
```

`package_page_html`'s doc comment states the design property this sub-plan must
preserve: it "renders the *same* `package_detail` handler output the JSON route
serves, so the two surfaces cannot disagree".

### What a doc page contains

From the model moved in plan-126-D (`DocPage` / `DocGroup` / `DocDecl` / `Prose`):
a package name and subtitle, intro prose, optional deprecation notice, and public
and internal groups of declarations. Each `DocDecl` carries `anchor`, `kind_label`,
`badge_class`, `member_label`, `name`, `signature`, `desc`, `args`, `props`, `ret`,
`errors`, `example` and `deprecated`.

`Prose.kind` is one of four (`DocProseKind::{Desc, Warn, Info, Sec}`), which
`src/doc/html.rs:44-49` renders as an ordinary paragraph plus three callout classes
(`warning`, `info`, `danger`). That mapping is the reference for the maud version.

### Measured populations

| What | Count | Command |
|---|---|---|
| `repository/src/web/mod.rs` lines / tests | 1,082 / 8 | `wc -l`; `grep -c '#\[test\]'` |
| `repository/src/web/style.css` lines | 879 | `wc -l repository/src/web/style.css` |
| `src/doc/html.rs` lines (the reference, not reused) | 734 | `wc -l src/doc/html.rs` |
| `DocDecl` fields to render | 13 | `sed -n '36,51p' src/doc/mod.rs` |
| `Prose` kinds | 4 | `sed -n '132,137p' src/ast/types.rs` |
| Existing `PreEscaped` uses in `repository/src/web/` | UNMEASURED — measure in Phase 1 and record; the count must not increase | `grep -rc PreEscaped repository/src/web/` |
| Documented packages available as fixtures | 6 | the six `.mfp` with a section 17 |

### Verified properties

- **The CSP genuinely blocks the compiler's renderer.** `repository/src/web/mod.rs:44`
  sets `style-src 'self'` with no `'unsafe-inline'`, and the module doc at `:41-43`
  states that this is why the stylesheet is a served route rather than a `<style>`
  block. `src/doc/html.rs:172` writes `"  <style>{STYLE}</style>\n</head>"`. The two
  are incompatible by design, not by accident.
- **The site has no JavaScript and the fold pattern already works without it.** The
  Overview's collapsible target rows use a checkbox + label (`repository/src/web/mod.rs:527-534`,
  `.fold-cb` / `.tgt-toggle`). A docs page sidebar or group folding must use the same
  technique, not a script.
- **`mfb pkg doc` already has an empty-documentation path**
  (`src/cli/pkg.rs:1823-1827`: `docs.is_empty()` → `render_empty_html`, exit 0), so
  the registry's explicit "no documentation" state matches existing CLI behavior
  rather than inventing one.

## 3. Design Overview

Four pieces:

1. **`package_tabs` becomes three-way.** Replace the `audit: bool` parameter with a
   small `PackageTab` enum (`Overview`, `Docs`, `Audit`) so adding a fourth tab
   later cannot produce a two-current-tabs bug the bool made impossible only by luck.
2. **`docs_page(registry_id, view) -> Markup`** in `repository/src/web/mod.rs`,
   beside `package_page` and `audit_page`, taking a `DocsView { ident, version,
   page: Option<DocPage> }`.
3. **Stylesheet rules** appended to `repository/src/web/style.css`, reusing the
   site's existing tokens rather than importing the compiler renderer's palette.
4. **Routes** — `/p/:ident/docs` (HTML) and `/packages/:ident/docs` (JSON) at
   `repository/src/server.rs:875-884`.

**Where correctness risk concentrates:** escaping. Every string on this page comes
from a publisher — including `signature` and `example`, which look like code and are
the most tempting things to render raw. maud escapes by default, so the risk is
entirely in reaching for `PreEscaped`. The acceptance criterion is a test that
publishes a package whose `DOC` text contains `<script>alert(1)</script>` and asserts
the rendered page contains the escaped form and not the raw tag.

**Where design uncertainty concentrates:** whether the doc page's information
density fits the site's existing visual language without importing a second design
system. Cheap to resolve — render one real package early (Phase 2) and look at it
before writing the remaining markup.

**Byte-identity is not this sub-plan's gate.** No codegen is involved; a `diffs=0`
from `scripts/artifact-gate.sh` says nothing about it. The gate is the repository
crate's rendered-HTML tests plus manual review of a real package's page.

**Rejected alternative — hide the tab for undocumented packages.** The site's stated
posture is that absence should be visible: the Overview's own copy
(`repository/src/web/mod.rs:505-509`) says nothing is hidden or collapsed. A missing
tab makes "no documentation" and "the docs failed to load" indistinguishable.

**Rejected alternative — a version selector on the docs URL.** Out of scope by
decision; the URL is designed so `?v=` is additive if it is ever wanted.

## Compatibility / Format Impact

- Two new routes; no existing route's response changes.
- `package_tabs`'s signature changes — internal to `repository/src/web`, no external
  contract.
- New CSS rules appended to the served stylesheet; existing rules untouched.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` plus one line for partial; `- [x] ~~text~~ —
> moot: <evidence>` for moot. Fill `Commit:` the moment a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Three-way tab strip and the empty state

Lands the navigation and the undocumented-package case first, so the route exists
and is reachable before any doc markup is written.

- [ ] Measure and record the current `grep -rc PreEscaped repository/src/web/` count
      in Corrections; it is the baseline the rest of this sub-plan must not raise.
- [ ] Replace `package_tabs(ident, audit: bool)` (`repository/src/web/mod.rs:389-400`)
      with a `PackageTab` enum parameter and update its two existing callers
      (`package_page` at `:500`, `audit_page` at `:671`).
- [ ] Add `DocsView { ident, version: Option<String>, page: Option<DocPage> }` and a
      `docs_page` that, for `page: None`, renders the shell, the tab strip, and
      explicit copy: the package's author did not include documentation in this
      release, with a pointer to `mfb man` for the language itself.
- [ ] Add the `/p/:ident/docs` route and `package_docs_html` handler
      (`repository/src/server.rs:875-877`), reusing `package_detail`'s not-found
      path so an unknown ident returns the same `message_page` as the other tabs.
- [ ] Tests in `repository/src/web/mod.rs`: all three tabs render with exactly one
      `aria-current="page"`, once per tab; the empty-docs page contains the
      no-documentation copy and no declaration markup.

Acceptance: `GET /p/<ident>/docs` for a package with no stored docs returns HTTP 200
with the three-tab strip and the explicit no-documentation statement; a rendered-HTML
test asserts exactly one `aria-current` attribute on each of the three pages.
Commit: —

### Phase 2 — Render one real package

Resolves the density question against real content before the markup is finalized.

- [ ] Wire `Store::latest_active_version_docs` into the handler, decode with
      `mfb_wire::docs::read_package_doc_section` and build the `DocPage` with
      `mfb_wire::docpage::from_package`.
- [ ] Render the package header (name, version, subtitle), intro prose, and the four
      `Prose` kinds as paragraph plus `info` / `warning` / `danger` callouts,
      mirroring the mapping at `src/doc/html.rs:44-49`.
- [ ] Add the corresponding rules to `repository/src/web/style.css`, reusing the
      site's existing colour and spacing tokens.
- [ ] Publish `packages/jwt/jwt.mfp` to a local `mfb-repo` and look at
      `/p/<ident>/docs`. Adjust before writing the declaration markup.

Acceptance: the page renders `jwt`'s package-level documentation with correctly
styled callouts, and `curl -sI` confirms the response still carries the unchanged
`Content-Security-Policy` header. Loading it in a browser produces **no CSP violation
in the console** — the check that would have caught reusing the compiler's renderer.
Commit: —

### Phase 3 — Declarations, and the escaping proof (largest blast radius)

- [ ] Render each `DocGroup` and `DocDecl`: kind badge, name, signature, description
      prose, `args` / `props` tables with the group's `member_label`, `ret`,
      `errors`, `example`, and the deprecation notice.
- [ ] Render the internal groups separately from the public ones, or omit them —
      decide per the Open Decision below and state which in Corrections.
- [ ] Add in-page anchors from `DocDecl::anchor` and a no-script group index using
      the checkbox+label pattern already used at `repository/src/web/mod.rs:527-534`.
- [ ] Add `GET /packages/:ident/docs` returning the same content as JSON, so the
      HTML page mirrors a JSON route like every other page on the site.
- [ ] **Escaping test**: construct a `DocPage` whose package description, a decl
      `signature`, a decl `example` and a parameter description each contain
      `<script>alert(1)</script>` and `" onmouseover="`, render it, and assert the
      output contains `&lt;script&gt;` and contains neither `<script>` nor an
      unescaped attribute break.
- [ ] Confirm `grep -rc PreEscaped repository/src/web/` equals the Phase 1 baseline.

Acceptance: `/p/<ident>/docs` for `jwt` renders every declaration group with
signatures, parameters, returns, errors and examples; the escaping test is green;
the `PreEscaped` count is unchanged from the Phase 1 baseline; and
`rustup run 1.96.0 cargo test --no-fail-fast` passes.
Commit: —

## Validation Plan

- **Tests:** `repository/src/web/mod.rs` — three-tab `aria-current` correctness,
  empty-docs copy, the four `Prose` kinds each rendering their callout class, the
  escaping test above, and a JSON/HTML parity test asserting both routes report the
  same version.
- **Coverage check:** the repository crate's 351 lib tests are in the `cargo test`
  denominator. `scripts/artifact-gate.sh` covers **none** of this sub-plan; a 0-diff
  there is not evidence and must not be cited.
- **Runtime proof:** publish all six documented packages
  (`jwt`, `json_schema`, `libsnd`, `mustache`, `sqlite3`, `yaml`) plus one
  undocumented one (`examples/browser/dom/dom.mfp`) to a local `mfb-repo`, then load
  `/p/<ident>/docs` for each. All six render; the seventh shows the
  no-documentation copy. Check the browser console is free of CSP violations on at
  least one page — that is the check a unit test cannot make.
- **Cross-surface check:** for a package whose newest release is yanked, the Docs tab
  must show the older active version's documentation and name that version — the
  end-to-end proof that plan-126-A's selection is the one being used.
- **Doc sync:** `repository/DEPLOY.md` if it enumerates routes
  (`grep -n '/p/' repository/DEPLOY.md`). No `mfb man` or `src/docs/spec/**` change —
  this is the registry web UI, not the language.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast`;
  `docker build -f repository/Dockerfile .`.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Show internal declarations?** `DocPage` separates `public` from `internal`
  (`src/doc/mod.rs:21-22`), and `DeclDocEntry.internal` is a wire field. Recommended:
  **omit internal groups on the registry**, since a consumer browsing a published
  package cannot call them, and render only `public`. Alternative: render them
  collapsed behind the existing fold pattern for transparency. Decide before Phase 3
  and record it. (§Phase 3)
- **Does the tab appear when the latest active version has no docs but an older one
  did?** Recommended: show the no-documentation copy for the *current* release
  without mentioning older ones — the tab's contract is "the current version's
  docs", and surfacing an older version's would contradict it. (§1)
- **JSON shape for `/packages/:ident/docs`.** Recommended: serialize the `DocPage`
  model directly rather than the raw wire structures, so the two surfaces cannot
  disagree about grouping and anchors. (§Phase 3)

## Corrections

<!-- Fill in during execution. Record: the Phase 1 `PreEscaped` baseline count; the
     internal-declarations decision; and anything the real-package render in Phase 2
     shows about density that changes the markup plan. -->

## Summary

The engineering risk here is not layout, it is escaping: every string on this page
was written by a package publisher, and the site's defense is that maud escapes by
default and the one bypass is greppable. That is why the `PreEscaped` count is a
tracked baseline and why the escaping test targets `signature` and `example`
specifically — the two fields that look like code and most invite raw rendering. The
second risk is quieter: the CSP would let a page *look* fine in a test and be broken
in a browser, so Phase 2's acceptance includes an actual console check. Left
untouched: the Overview's complete version table, the Audit tab, the CSP, and the
site's no-JavaScript property.
