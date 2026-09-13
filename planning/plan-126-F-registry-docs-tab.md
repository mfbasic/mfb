# plan-126-F: The registry "Docs" tab

Last updated: 2026-09-12
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
| plan-126-E complete (docs are stored and retrievable) | `grep -c latest_active_version_docs repository/src/store.rs` → ≥1 | MET (measured 2026-09-12: **7**; E landed as acdbe0648, f9e26140a, 1940b4dd4, and was runtime-proven by publishing the real `jwt` package, whose stored 27,951-byte doc section is byte-identical to the published blob's section 17) |
| plan-126-A complete (`latest_active_version` exists) | `grep -c 'fn latest_active_version(' repository/src/store.rs` → 1 (corrected from `'fn latest_active_version'`) | MET (measured 2026-09-12: exactly **1**; A landed as 1285d1f96, 90150eff1, cce8f7b02). The original command returns **8**, not 1: without the `(` it also matches `fn latest_active_version_docs` and test names such as `fn latest_active_version_skips_a_yanked_newest_release`. The requirement holds; the command was miscalibrated. |

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

- [x] Measure and record the current `grep -rc PreEscaped repository/src/web/` count
      in Corrections; it is the baseline the rest of this sub-plan must not raise.
- [x] Replace `package_tabs(ident, audit: bool)` (`repository/src/web/mod.rs:389-400`)
      with a `PackageTab` enum parameter and update its two existing callers
      (`package_page` at `:500`, `audit_page` at `:671`).
- [x] Add `DocsView { ident, version: Option<String>, page: Option<DocPage> }` and a
      `docs_page` that, for `page: None`, renders the shell, the tab strip, and
      explicit copy: the package's author did not include documentation in this
      release, with a pointer to `mfb man` for the language itself.
- [x] Add the `/p/:ident/docs` route and `package_docs_html` handler
      (`repository/src/server.rs:875-877`), reusing `package_detail`'s not-found
      path so an unknown ident returns the same `message_page` as the other tabs.
- [x] Tests in `repository/src/web/mod.rs`: all three tabs render with exactly one
      `aria-current="page"`, once per tab; the empty-docs page contains the
      no-documentation copy and no declaration markup.

Acceptance: `GET /p/<ident>/docs` for a package with no stored docs returns HTTP 200
with the three-tab strip and the explicit no-documentation statement; a rendered-HTML
test asserts exactly one `aria-current` attribute on each of the three pages.
**Met (2026-09-12):** router test `the_docs_tab_states_when_a_release_has_no_documentation`
(`grep -n 'fn the_docs_tab_states_when_a_release_has_no_documentation' repository/src/server.rs`)
publishes an undocumented release and asserts HTTP 200, the three-tab strip and the
no-documentation copy; `every_package_tab_page_marks_exactly_one_current_tab` fetches
all three routes and asserts exactly one `aria-current` on each, on the right tab;
`package_tabs_mark_exactly_the_current_tab` pins the same at the renderer.
`rustup run 1.96.0 cargo test --no-fail-fast --manifest-path repository/Cargo.toml --lib`
→ 392 passed, 0 failed.
Commit: aa1d90ca1 (Phases 1 and 2 together; see Corrections)

### Phase 2 — Render one real package

Resolves the density question against real content before the markup is finalized.

- [x] Wire `Store::latest_active_version_docs` into the handler, decode with
      `mfb_wire::docs::read_package_doc_section` and build the `DocPage` with
      `mfb_wire::docpage::from_package`.
- [x] Render the package header (name, version, subtitle), intro prose, and the four
      `Prose` kinds as paragraph plus `info` / `warning` / `danger` callouts,
      mirroring the mapping at `src/doc/html.rs:44-49`.
- [x] Add the corresponding rules to `repository/src/web/style.css`, reusing the
      site's existing colour and spacing tokens.
- [x] Publish `packages/jwt/jwt.mfp` to a local `mfb-repo` and look at
      `/p/<ident>/docs`. Adjust before writing the declaration markup.

Acceptance: the page renders `jwt`'s package-level documentation with correctly
styled callouts, and `curl -sI` confirms the response still carries the unchanged
`Content-Security-Policy` header. Loading it in a browser produces **no CSP violation
in the console** — the check that would have caught reusing the compiler's renderer.
**Met (2026-09-12):** `jwt.mfp` published to a local `mfb-repo` on 127.0.0.1:7792.
`curl -sI 'http://127.0.0.1:7792/p/alice%23jwt/docs'` → `HTTP/1.1 200 OK` with
`content-security-policy: default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'`
(identical to `CSP` in `repository/src/web/mod.rs`). The body (7,020 bytes) names `v0.1.0`,
carries the subtitle, and renders 6 `callout--danger` blocks. In the browser:
`document.styleSheets.length` = 1 (the served `/style.css`), computed `.callout`
`border-left-width` = `4px` (the stylesheet applied — a blocked sheet would leave it `0px`),
0 `[style]` elements, 0 `<style>`/`<script>`, and the only `aria-current` is `Docs`.
A static audit of the served HTML (`python3 /tmp/p126-csp-audit.py`) found 0 `style=`
attributes, 0 `on*` handlers, 0 `javascript:` URLs, and one loaded resource,
same-origin `/style.css`; nothing on the page can trip `default-src 'none'`.
Commit: aa1d90ca1

### Phase 3 — Declarations, and the escaping proof (largest blast radius)

- [x] Render each `DocGroup` and `DocDecl`: kind badge, name, signature, description
      prose, `args` / `props` tables with the group's `member_label`, `ret`,
      `errors`, `example`, and the deprecation notice.
- [x] Render the internal groups separately from the public ones, or omit them —
      decide per the Open Decision below and state which in Corrections.
- [x] Add in-page anchors from `DocDecl::anchor` and a no-script group index using
      the checkbox+label pattern already used at `repository/src/web/mod.rs:527-534`.
- [x] Add `GET /packages/:ident/docs` returning the same content as JSON, so the
      HTML page mirrors a JSON route like every other page on the site.
- [x] **Escaping test**: construct a `DocPage` whose package description, a decl
      `signature`, a decl `example` and a parameter description each contain
      `<script>alert(1)</script>` and `" onmouseover="`, render it, and assert the
      output contains `&lt;script&gt;` and contains neither `<script>` nor an
      unescaped attribute break.
- [x] Confirm `grep -rc PreEscaped repository/src/web/` equals the Phase 1 baseline.
      Measured after Phase 3: `mod.rs:1`, `style.css:0` — the baseline — and
      `grep -rn 'PreEscaped(' repository/src/` finds no call.
- [x] *(Added.)* Prefix every declaration element id with `doc-`
      (`crate::web::doc_element_id`). The model's anchors avoid only the compiler
      page's own ids; this site's shell puts `id="q"` on the search box in every page
      header, so a declaration named `q` would have duplicated it and its index link
      would have jumped to the search box. Pinned by
      `a_declaration_named_like_a_shell_id_does_not_duplicate_it`; the JSON route
      reports the same prefixed id.
- [x] *(Added.)* Render backtick spans in doc text as `<code>` (`doc_inline`), the
      compiler renderer's only inline markup (`src/doc/html.rs`, `inline`), with every
      piece still interpolated so it is escaped. Pinned by
      `doc_inline_renders_backtick_spans_as_escaped_code`.

Acceptance: `/p/<ident>/docs` for `jwt` renders every declaration group with
signatures, parameters, returns, errors and examples; the escaping test is green;
the `PreEscaped` count is unchanged from the Phase 1 baseline; and
`rustup run 1.96.0 cargo test --no-fail-fast` passes.
**Met (2026-09-12):** see § Validation results. `jwt`'s tab renders 21 declarations in
5 groups with 21 signatures, 28 parameters, 21 returns, 38 errors and 17 examples;
`docs_page_escapes_every_publisher_controlled_field` is green; the `PreEscaped` count
is the baseline 1; repository lib tests 402 passed.
`rustup run 1.96.0 cargo test --no-fail-fast` (full workspace, pre-merge tree) → exit 0: **163 test binaries, 5,395 passed, 0 failed, 6 ignored** (log `/tmp/p126-f3-fulltest.log`; 5,317 at plan-126-E's prerequisite run). The main crate's unit tests alone took 4,002 s against 2,073 s earlier the same day, on a host at load average 70–110 on 12 cores (other sessions' test runs, a QEMU VM, two stray probes); the slowest test, `the_whole_corpus_survives_the_top_of_the_dial`, predates this plan (8e19307a1) and is untouched by it.
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

### Validation results (measured 2026-09-12)

- **Tests** (`rustup run 1.96.0 cargo test --no-fail-fast --manifest-path repository/Cargo.toml --lib`
  → **402 passed, 0 failed**, 0 warnings; 392 before Phase 3). Phase 3 added
  `the_docs_page_renders_every_part_of_a_declaration`,
  `internal_declarations_are_not_rendered`, `a_page_with_no_public_declarations_says_so`,
  `doc_inline_renders_backtick_spans_as_escaped_code`,
  `a_declaration_named_like_a_shell_id_does_not_duplicate_it`,
  `docs_page_escapes_every_publisher_controlled_field` (web), and
  `the_docs_json_route_mirrors_the_docs_tab` (the JSON/HTML parity test),
  `the_docs_json_route_reports_null_for_an_undocumented_release`,
  `the_docs_json_route_404s_like_the_package_route`,
  `the_docs_tab_documents_the_older_active_release_when_the_newest_is_yanked`
  (server, through the real router). The four `Prose` kinds each rendering their
  callout class is `the_docs_page_renders_package_prose_and_every_callout_kind`.
- **Runtime proof.** All six documented packages plus `dom` were published
  (`mfb repo publish alice <copy>`; logs `/tmp/p126-pub-*.log`, all `valid: true`) to a
  local `mfb-repo` restarted on the Phase 3 build, then checked by
  `python3 /tmp/p126-f3-validate.py`, which fetches both `/p/<ident>/docs` and
  `/packages/<ident>/docs` for each and requires: HTTP 200 on both; the exact site CSP;
  no `<script`, `<style` or `style=`; the JSON `version` named on the page; every JSON
  anchor present as an element id; section count = JSON declaration count = the
  compiler's own `mfb doc` section count for the same package; and signature/example/
  returns counts equal between the two surfaces. Result, `failures: 0`:

  | package | version | groups | decls (registry / `mfb doc`) | params | errors | examples |
  |---|---|---|---|---|---|---|
  | jwt | 0.1.0 | 5 | 21 / 21 | 28 | 38 | 17 |
  | json_schema | 0.1.0 | 4 | 12 / 12 | 12 | 24 | 12 |
  | libsnd | 1.5.0 | 4 | 12 / 12 | 12 | 24 | 6 |
  | mustache | 0.1.0 | 1 | 4 / 4 | 6 | 6 | 4 |
  | sqlite3 | 0.1.0 | 6 | 20 / 20 | 23 | 9 | 3 |
  | yaml | 0.1.0 | 1 | 3 / 3 | 2 | 16 | 3 |
  | dom | 0.1.0 | — | no documentation; the explicit copy renders | — | — | — |

- **Browser check** (jwt, fresh-origin load): one stylesheet, 0 `[style]`, 0 `<script>`,
  0 `<style>`, 21 `section.decl`, 0 duplicate element ids, 21 index links with 0
  broken; the new rules apply (index groups `display: grid`, signature background
  `oklch(0.24 0.006 90)` = dark `--surface-2`, `overflow-x: auto`, badge radius
  `999px`). The fold works with no script: unchecking the index checkbox computes
  `display: none`, re-checking restores `grid`.
- **Cross-surface check.** Proven through the real router by
  `the_docs_tab_documents_the_older_active_release_when_the_newest_is_yanked`: before
  the yank the tab names `v2.0.0` and shows its docs; after yanking 2.0.0 both the tab
  and `/packages/:ident/docs` name `1.0.0` and serve 1.0.0's documentation, with no
  trace of 2.0.0's.
- **Doc sync.** Moot: `grep -n -E '/p/|/packages/' repository/DEPLOY.md` returns
  nothing — DEPLOY.md enumerates no routes.
- **Acceptance.** `docker build -f repository/Dockerfile .` → exit 0 on the Phase 3 tree (log `/tmp/p126-docker-f.log`: `Compiling mfb_wire`, `Compiling mfb_repository`, `Finished release profile … in 2m 11s`, all five runtime stages DONE). The full `rustup run 1.96.0 cargo test --no-fail-fast` result is recorded under Phase 3's acceptance.

## Open Decisions

- **Show internal declarations?** `DocPage` separates `public` from `internal`
  (`src/doc/mod.rs:21-22`), and `DeclDocEntry.internal` is a wire field. Recommended:
  **omit internal groups on the registry**, since a consumer browsing a published
  package cannot call them, and render only `public`. Alternative: render them
  collapsed behind the existing fold pattern for transparency. Decide before Phase 3
  and record it. (§Phase 3)
  **Resolved: omitted** — see Corrections.
- **Does the tab appear when the latest active version has no docs but an older one
  did?** Recommended: show the no-documentation copy for the *current* release
  without mentioning older ones — the tab's contract is "the current version's
  docs", and surfacing an older version's would contradict it. (§1)
  **Resolved as recommended:** `lookup_package_docs` reads only
  `latest_active_version_docs`; the empty state names the current release only.
- **JSON shape for `/packages/:ident/docs`.** Recommended: serialize the `DocPage`
  model directly rather than the raw wire structures, so the two surfaces cannot
  disagree about grouping and anchors. (§Phase 3)
  **Resolved as recommended:** `PackageDocsResponse` is built from the `DocPage` by
  `PackageDocsResponse::from_view`, and both routes share one `lookup_package_docs`.

## Corrections

- **Phase 1 `PreEscaped` baseline = 1** (`grep -rc PreEscaped repository/src/web/` →
  `mod.rs:1`, `style.css:0`; also 1 at the fork commit,
  `git show e66e594a4:repository/src/web/mod.rs | grep -c PreEscaped`). The one hit is
  the module doc *naming* the bypass, not a use — `grep -n 'PreEscaped(' repository/src/web/`
  returns nothing. The count is of mentions, so a doc comment can raise it: this
  sub-plan's own `doc_prose` comment first said "PreEscaped" and took the count to 2; it
  was reworded ("never reaches for maud's escaping bypass") to hold the baseline.
- **Phase 2 names the wrong decoder.** It says decode with
  `mfb_wire::docs::read_package_doc_section`, but that function takes a whole `.mfp`
  *payload* and locates section 17 itself; `package_version_docs` stores the raw
  section-17 bytes (plan-126-E), so the handler decodes them with
  `mfb_wire::docs::read_doc_table` (`grep -n 'pub fn read_doc_table\|pub fn read_package_doc_section' wire/src/docs.rs`).
  Passing the stored bytes to `read_package_doc_section` would fail to find a section table.
- **Phases 1 and 2 landed as one commit.** Phase 1's route could not be exercised end to
  end without Phase 2's store read (the handler has one body), so both phases' boxes
  are ticked in the same commit as the code.
- **Browser check method.** The preview tool's `snapshot` call failed on every tab and
  `evaluate` timed out twice on the first; the in-browser measurements above come from a
  fresh tab's `evaluate`, backed by the static HTML audit, which does not depend on the
  tool at all.
- **Store and decode failures are a 500, not the empty state.** An error reading or
  decoding the stored section renders "Documentation unavailable" (HTTP 500) instead of
  the no-documentation copy, so a corrupt row can never be displayed as "the author
  included none" — the indistinguishability § Design Overview rejects hiding the tab for.
- **Phase 2 density review.** The package-level render (header, intro, callouts) fits the
  site's existing `pkg-head` / `pkg-latest` / `pkg-desc` classes and its existing
  `--note*` / `--warn*` / `--danger*` tokens (each already with light and dark values)
  without a second design system: `git diff -- repository/src/web/style.css` is 32 added
  lines, 0 removed, and no new custom property. No change to the Phase 3 markup plan.
- **Internal declarations are omitted** (Open Decision). A decl is internal only when its
  author wrote the `INTERNAL` attribute on its `DOC` block
  (`grep -n 'eq_ignore_ascii_case("INTERNAL")' src/ir/docs.rs`), i.e. it is
  explicitly not API. Neither surface renders them; both the HTML renderer and
  `PackageDocsResponse::from_view` read only `page.public`
  (`internal_declarations_are_not_rendered`, and the parity test asserts `secretHelper`
  is absent from both bodies). None of the seven real packages ships one: the
  compiler's `mfb doc` HTML for all of them contains 0 `Internal — not part of the
  public API` headings (`grep -c` over `/tmp/p126-bytecheck`).
- **Declaration ids are `doc-<anchor>`, not the bare model anchor** — the id collision
  with the shell's `id="q"` recorded as an added Phase 3 task. The JSON `anchor` field
  carries the prefixed id so it links straight into the tab.
- **JSON `members` carries a declaration's `props` whatever its kind**, the model
  serialized directly as the Open Decision recommended; the HTML tab shows them only
  under a `memberLabel` (`Fields`/`Variants`/`Members`), matching the compiler's
  `render_decl`. Measured over the six published packages' JSON, no declaration has
  members without a label (0 in each); the 22 members that exist (libsnd 12, sqlite3
  10) all sit under a label, so the two surfaces show the same members today.
- **The HTML tab links its JSON mirror** (`… — raw JSON`), as the Audit tab does, which
  the parity test asserts.
- **Escaping test assertion corrected before landing.** A draft of
  `docs_page_escapes_every_publisher_controlled_field` also asserted the page contains
  no ` onmouseover=` substring; it failed because the correctly escaped *text*
  `&quot; onmouseover=&quot;x` contains that substring (the same over-match plan-126
  corrected once before). It now asserts no attribute break (`" onmouseover="`) and that
  `onmouseover` occurs exactly 8 times, each as the escaped text of one of the eight
  hostile fields — which also proves each field was rendered, not dropped.
- **Stylesheet caching hid the new rules in the browser.** The first Phase 3 browser
  measurement showed the index as `display: block` and a transparent signature:
  `/style.css` is served with `cache-control: public, max-age=3600`
  (`curl -sI http://127.0.0.1:7792/style.css`), so the tab reused the sheet cached
  during the Phase 2 look, while `curl` showed the served sheet already had the rules.
  A load from a fresh origin (`localhost` instead of `127.0.0.1`) measured them
  applied. Not a defect in this sub-plan, but a deploy consequence worth knowing: a
  stylesheet change reaches a returning visitor up to an hour after deploy.
- **Three pre-existing test warnings fixed.** `cargo test --lib` warned
  ``unused `axum::Json` that must be used`` at three `log_checkpoint(...).expect(...)`
  calls in `anonymous_log_routes_are_rate_limited_per_ip` and
  `the_log_budget_clears_a_full_install_burst_from_one_ip`, from 460b983dd (bug-579),
  an ancestor of this plan's fork
  (`git merge-base --is-ancestor 460b983dd e66e594a4` → 0). Bound to `let _ =`; the lib
  tests now build with 0 warnings.

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
