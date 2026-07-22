# mfb-repo — design rationale

Static HTML + CSS reference for the four public, anonymous, read-only pages.

## Files

**Deliverables (hand these to the implementer):**

- `index.html` — `GET /`
- `search.html` — `GET /search.html?q=` (results state)
- `search-noresults.html` — the 0-results state (HTTP 200, not an error)
- `search-empty.html` — the empty-query state (form only)
- `package.html` — `GET /p/:ident`
- `audit.html` — `GET /p/:ident/audit`
- `style.css` — the single stylesheet, served from `/style.css`

The three search states are three renderings of the same route; they are split
into three files here only so the implementer can see each one.

**`preview/`** — copies with the CSS inlined into a `<style>` block. These exist
*only* so the design renders inside this tool's preview (which does not serve a
sibling stylesheet for a plain `.html` file). They are not part of the handoff —
ship the files above, which keep the CSS external as the CSP requires.

## Compliance with the hard constraints

- **Zero JavaScript.** No `<script>` tag exists in any file. Search is
  `<form method="get" action="search.html">`. The two package tabs are two
  `<a>` links to two URLs. Every hex value that truncates uses a native
  `<details>/<summary>` disclosure — an HTML element, not a script.
- **CSP.** No `style=""` attributes and no inline `<style>` in the deliverables;
  all CSS is in `/style.css`. No external fonts (system stacks only), no external
  images (no `<img>` at all — every glyph is a Unicode character or CSS
  `::before`), no `<iframe>`.
- **No accounts.** There is no sign-in, account menu, or logged-in variant
  anywhere — header, footer, or empty states.
- **No mutation.** Every affordance is a link or a GET form. There is no button
  that implies an action beyond running a search.
- **Hostile content.** All values are treated as attacker-controlled visible
  text. The `matrix` fixture's `author` is the literal string
  `<script>alert(1)</script>`, shown escaped as text. Long values
  (URLs, 64-char hashes, the `owner#package` ident) wrap with
  `overflow-wrap: anywhere` / `word-break: break-all` and never break the grid;
  the 200-row version table and an 8-row target matrix are both shown at size.

## 1 — The fingerprint wording

The single hardest requirement was to present the server fingerprint without it
reading as a trust assertion. The design does three things:

1. **The words never claim verification.** The eyebrow is
   *"Compare this — do not trust it on sight."* The body says plainly that the
   page *cannot* prove it is the real registry, and that an impostor would serve
   its own fingerprint and this same reassuring paragraph. It then instructs the
   reader to obtain the expected value **out of band** ("a source that does not
   pass through this server") and compare character by character, and only then
   run `mfb repo trust <registry-id> <fingerprint>`. It closes with the failure
   branch: *"If the two values differ, stop."* The copy frames the fingerprint
   as **input to an action the reader performs**, never as a verdict the server
   delivers.

2. **The visual treatment signals "instruction," not "verified."** There is no
   check, shield, lock, or badge, and deliberately **no green** — those are the
   vocabulary of "this is fine." The callout uses a neutral indigo accent
   (chosen precisely because it is *not* a success color), a dashed rule under
   the heading, and a command styled like a shell prompt (`$ …`). It looks like
   a step in a runbook, which is what it is.

3. **The one truly alarming line is the only colored one.** "You are not talking
   to the registry you think you are" is the sole use of the danger color on the
   page, so the reader's eye lands on the consequence of a mismatch.

## 2 — Version states stay unmissable without hiding anything

Every version is rendered, in one continuous table, oldest included. Nothing is
collapsed, filtered, folded, or paginated — a version *missing* from this list is
itself the tamper signal the registry exists to expose, so the caption says so.

`deprecated` and `yanked` are made unmistakable through **redundant, non-color
cues** (so they survive color-blindness and grayscale printing):

- a bordered uppercase **state label** with a leading shape glyph
  (`▲` deprecated, `■` yanked) — not relying on hue alone;
- a **tinted background across the whole row**, not just the pill; and
- a **colored bar down the row's leading edge** (amber / red).

`available` is deliberately quiet — no green "go" signal — so attention is spent
only where a version's status should give a consumer pause. On a phone each
version becomes a card; the yanked/deprecated card keeps a full colored border
and tinted header so the signal survives the layout change.

## 3 — The target matrix and audit hex on a phone

**Responsive strategy — no horizontal scroll.** Every data table carries a
`data-label` on each `<td>`. Below 760px the table's header is visually hidden
and each row becomes a self-contained card: each cell displays as a
`label → value` line, the label pulled from `data-label` via a CSS `::before`.
Tables become vertical lists of labeled fields, which is the natural phone
reading order, instead of a pannable grid. This applies uniformly to the version
table, the nested target matrix, and all three audit tables.

**Target matrix.** Targets hang directly beneath their version as a nested
sub-table (a version with zero targets — the common case — instead shows a single
muted "source-only" line, so absence is explicit, not blank). Null values render
as words, never emptiness: `arch = null` → **`any`**, `libc = null` → **`—`**.
`libType` is a small `system`/`vendor` tag. On a phone each target collapses to
its own labeled card, so an 8-platform version reads as eight short blocks rather
than a wide grid.

**Audit hex (64+ chars).** Each long value is a native `<details>` disclosure:
the summary shows a truncated head with a `show`/`hide` affordance; expanding
reveals the complete value in a wrapping monospace block that is fully
selectable. This gives truncation-by-default with the full value **reachable
without JavaScript and without a clipboard button** (which would need JS). The
raw JSON endpoint is linked prominently near the top for monitors that script
against the log. The framing throughout is evidentiary, not reassuring: the page
closes by stating that a transparency log lets a third party *detect*
inconsistency — it does not promise there is none.
