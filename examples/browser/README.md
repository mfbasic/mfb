# browser — a terminal web viewer

A tiny full-screen terminal "web browser" built on the `term::` TUI surface and
the `http::` client. It fetches a page, parses the HTML into a small DOM, and
shows the readable text — with an address bar, the page title, a padlock
secure-indicator, an animated loading spinner, and a **Raw Mode** that displays
the parsed DOM as an indented tree.

It is split into three packages plus the app so each concern is its own unit and
the loading spinner keeps spinning while a page loads *and* parses:

| Project     | Kind       | What it is                                                                    |
| ----------- | ---------- | ----------------------------------------------------------------------------- |
| `dom/`      | package    | The DOM: `Node` (`ElementNode`/`TextNode`/`HeaderNode`) + `StyleNode` (a CSS rule) + an HTML **and** CSS parser, plus `Style`/`Layout` resolution (`resolveStyles`, `updateLayout`). |
| `fetch/`    | package    | The network worker: blocking `http::` fetch, redirects, `dom::parse`, **and loading external stylesheets**. |
| `display/`  | package    | Renders a `dom::Node`: `paint(node, widthCols, cellPx, linePx)` → a positioned 2D canvas (maps the DOM's CSS-px `Layout` to cells), `render(node, width)` → reflowed text, `tree(node, width)` → an indented DOM tree. |
| `app/`      | executable | The TUI: layout, input, scrolling, the three modes.                          |

`fetch` and `display` both import `dom` (they name its `Node` type in their APIs);
the app imports all three. All are referenced by **relative local paths**, not a
package repository:

```json
"packages": [
  { "name": "dom",     "version": "=0.1.0", "source": "file:packages/dom.mfp" },
  { "name": "fetch",   "version": "=0.1.0", "source": "file:packages/fetch.mfp" },
  { "name": "display", "version": "=0.1.0", "source": "file:packages/display.mfp" }
]
```

## Why a worker thread — and why the DOM crosses it

`http::read` is **blocking**: it freezes the calling thread until the whole
exchange finishes. To keep the UI free to animate the spinner, the app runs the
fetch on a separate OS thread with `thread::start(fetch::fetch, url)` and polls
`thread::isRunning` while redrawing. `thread::start` requires an **exported
`ISOLATED` function from an imported package**, which is why the fetch lives in
`fetch`.

The worker parses the page into a DOM on that thread and returns the **document
`Node`** — a recursive tree — which `thread::waitFor` **deep-copies** out of the
worker's arena into the app's. (Transferring a recursive value across threads was
`bug-391`; it is fixed.) So both the network *and* the parse stay off the UI
thread, and the app just renders the tree.

## Building

Each package must be built to a `.mfp` and installed where the next one's
manifest expects it (`packages/<name>.mfp`). From the repository root:

```sh
# 1. dom (no deps)
mfb build examples/browser/dom

# 2. fetch and display both depend on dom
for p in fetch display; do
  mkdir -p examples/browser/$p/packages
  cp examples/browser/dom/dom.mfp examples/browser/$p/packages/dom.mfp
  mfb build examples/browser/$p
done

# 3. the app depends on all three
mkdir -p examples/browser/app/packages
cp examples/browser/dom/dom.mfp examples/browser/fetch/fetch.mfp \
   examples/browser/display/display.mfp examples/browser/app/packages/
mfb build examples/browser/app
./examples/browser/app/build/browser.out
```

The installed `.mfp` files (and each package's build artifact) are git-ignored,
so re-run the relevant steps whenever you change a package's source.

## Using it

Run it from an interactive terminal. The top bar shows the page **title**
(right-aligned) in Display Mode, or `HREF: <url>` while editing an address. The
far-left indicator is a spinner while loading, 🔒 for an https page, 🔓 otherwise.

- **G** — enter Address Mode; type a URL, **Backspace** deletes, **Enter** loads
  it, **Esc** clears the address (press it again on an empty field to cancel).
  `https://` is assumed when you omit the scheme.
- **R** — (Display Mode) switch to **Raw Mode**: the parsed DOM as an indented tree,
  followed by each form's data object as JSON (see **Form data**, below).
- **D** — (Raw Mode) switch back to Display Mode.
- **Up / Down** — scroll vertically. **Left / Right** — pan horizontally across a
  page laid out wider than the terminal (a mouse/trackpad's scroll drives these too,
  via the terminal's alternate-screen scroll translation).
- **Tab** — (Display Mode) focus the next **on-screen link or form field**, wrapping
  within those currently visible; the focused one is highlighted. **Enter** follows a
  focused link, **Esc** un-focuses. Scroll to bring the targets you want on screen,
  then Tab through them; a target that scrolls out of view (or a reflow from
  resize/mode) drops the selection — except a focused field, which is re-found by its
  ordinal so typing survives a resize.
- **Typing into a field** — with a form field focused, every printable key types into
  it and **Backspace** deletes. **Tab** moves on to the next target and **Esc** leaves
  the field; the page shortcuts (Q/G/R/L/M) are text while a field has focus, and
  **Enter** is ignored — a single-line `<input>` takes no newline. The box shows the
  value it holds, keeping its `size` width (an over-long value shows its tail, and a
  `password` is masked).
- **L** — (Display Mode) open the **Links list**: a labelled multi-column list of *every*
  link on the page — `A) Link Text`, `B) …` (two letters past 26 links). Type a link's
  letter(s) to follow it, **Up / Down** scroll the list, **Esc** returns to the page. The
  column count follows the width mode, so **(w)**/**(e)** show more columns.
- **M** — cycle the display width mode: **(s)**tandard (the terminal width),
  **(w)**ide (300 columns), **(e)**xtra-wide (600). Toggling re-lays-out whatever is
  on screen (a rendered page, the raw tree, or the fallback). The current mode shows
  as `(s)`/`(w)`/`(e)` just before `Files` in the footer; loading a new address
  resets it to standard. Wider-than-terminal modes are navigated with Left / Right.
- **Q** — quit (also aborts an in-flight load).

Redirects (301/302/303/307/308) are followed automatically, up to 10 hops.

## The DOM parser

`dom::parse` is a forgiving, iterative tag stripper (not a strict XML parser): it
drops `<script>`/`<style>`/comments (recording their src/href/inline markers in a
`HeaderNode`), captures `<title>`, builds an `ElementNode`/`TextNode` tree, decodes
character references (`encoding::htmlUnescape`), and handles unclosed tags. The
document is an `ElementNode` tagged `#document` whose first child is the header.

Everything is **iterative** (an explicit stack, never a recursive function over
`Node`) — a recursive function over an *imported* union does not lower to native
code across a package boundary, so `dom`/`display` walk trees with a work-stack.
`dom::parse` also caps its node count: extracting nodes churns short-lived arena
allocations, whose free list is quadratic (a known open arena issue), so a
multi-megabyte page is rendered as a truncated preview rather than hanging.

## CSS

The `fetch` worker also loads stylesheets: it reads each `<link rel="stylesheet">`
over http (resolving relative hrefs against the page URL) and captures each inline
`<style>`. Every stylesheet body is kept on the header (`HeaderNode.css`) and
parsed by `dom::parseCss` into **`StyleNode`s** — one per rule, each with the
selector and a `props: Map OF String TO String` (a comma selector list becomes one
rule per selector). The rules hang off `HeaderNode.rules` and show in Raw Mode; the
footer's `Files: n/m` counts the HTML document plus each `<link>` stylesheet, where a
sheet counts as **loaded** when its fetch completes with HTTP 200 and the whole body
arrives (received bytes == `Content-Length`, or a chunked body finishes). That counts
a valid empty sheet — e.g. google.com serves one `<link>` as a 200 with
`Content-Length: 0` — and excludes a redirect or 404 error body.

`StyleNode` is a plain **standalone record** (not a `Node` variant): `display`
reads its fields directly to format each rule, even though a `StyleNode` is reached
only *transitively* through `HeaderNode.rules` (a field of a `Node` variant). That
cross-package transitive read used to be opaque to a consumer's codegen — so this
was formerly a `Node` variant with all iteration living in `dom` (`dom::styleLines`
returning pre-formatted strings) — until the `bug-435` fix made a re-exported
union carry the full type closure of its variants' fields.

## Style & layout

Every `ElementNode` carries a resolved **`Style`** and a computed **`Layout`**,
filled by two passes over the tree that live *inside* `dom` (so they may recurse
over `dom`'s own `Node` union — the "must be iterative" rule only binds a consumer
recursing over an *imported* union):

- **`Style`** (`style.mfb`) is the CSS a single element resolves to — a closed
  record with one typed field per supported property (a `Display` / `FlexDirection`
  / `FlexWrap` / `Justify` / `Align` enum, or an Integer length in **device-
  independent CSS px**, with `-1` meaning `auto`), not an open string bag. Lengths are
  parsed by unit: `px`/unitless keep their number and `em`/`rem` use a 16px default
  font size. A **`width`** additionally carries its unit (`Style.widthUnit`) so a
  relative width resolves against the right reference at layout time: `60vw` is
  60% of the viewport (`60vw` of an 848px viewport → 508px → 64 cells), `100%`/`50%`
  is a fraction of the containing block, and `vh`/`vmin`/`vmax` (no height in a
  scrolling text viewport) fall back to auto. Percent/viewport units on the other box
  properties (they are rare there) still fall back to auto. `dom::resolveStyles(doc)`
  (`resolve.mfb`) walks the tree and for each element applies the user-agent default
  for its tag, then every matching `StyleNode` rule in document order, then the
  inline `style="…"` attribute. Selectors support **compound** simple-selectors
  (`a.gb_4a`, `.a.b`, `div#id`, `*`) and the **descendant** combinator (`.box a` — a
  child combinator `>` is treated as descendant); the rightmost compound must match
  the element and each earlier compound must match an ancestor, so the resolver
  threads each element's ancestor chain down the tree. Unsupported pieces (a
  `:pseudo`, `[attr]`) simply fail to match rather than matching wrongly. Layout
  properties do not inherit (except `text-align`), so each element resolves from its
  own tag/attrs, its ancestors, and the global rule set. The `fetch` worker runs this
  once, on its thread, so the fully-styled document is what crosses back.

- **`Layout`** (`layout.mfb`) is the computed geometry: an absolute border box
  (`x, y, width, height` in **device-independent CSS px**, from the viewport's
  top-left), stored on every `ElementNode` *and* every `TextNode` (a text run's box is
  where it wraps). `dom::updateLayout(doc, viewportPx, cellPx, linePx, fitWidth)`
  derives it from `Style`. A `display:flex`
  box uses flex layout along its `flex-direction` (honoring `flex-grow`/`shrink`/
  `basis`, `width`/`height`, `gap`, and `justify-content`); a shrinking flex item
  never drops below its **min-content** width (its longest word), matching real
  flexbox's automatic minimum — so an over-full row overflows and clips at the edge
  instead of collapsing each item into a column of single characters. Every other box uses
  **block formatting** — block-level children (`display:block`/`flex`) stack
  vertically, while consecutive **inline-level** children (text and inline elements
  like `<a>`/`<b>`/`<em>`) flow *together* into one wrapped run, so a paragraph's
  inline markup does not break onto separate lines. A `<br>` forces a break, and an
  inline element that actually wraps block-level content (a `<span>`/`<center>` around
  `<div>`s, as real pages often do) breaks out as a block rather than flattening its
  blocks into one crammed run. A `<table>` gets **real column layout**: each column's
  width is the max of its cells' natural widths (shrinking toward each column's
  min-content — its longest word — to fit), and every row places its cells at the
  shared column positions so columns line up across rows; `colspan` is honored
  (`rowspan` is treated as one row). An auto-width table **shrinks to fit** its
  columns rather than filling the width, so it can be centred. `center` and the sectioning wrappers are block by
  default. Vertical spacing between blocks comes from margins;
  the UA sheet gives paragraphs, lists, and headings a default margin. **Horizontal
  alignment** is unified into the flex vocabulary: `text-align`, a `<center>`
  element, and an `align="center"`/`"right"` attribute all resolve into the shared
  `Justify` enum (`FlexStart`/`Center`/`FlexEnd`) on the block, and each wrapped line
  of its inline content is placed accordingly — the same `Justify` that drives a flex
  row's `justify-content`. Alone among the layout properties this one **inherits**, so
  a `<center>` (or `text-align` on an ancestor) centers all descendant text. It also
  centers (or right-aligns) a **block** child that is narrower than the content box —
  a shrink-to-fit table or a fixed-width element — by shifting its laid box, so
  `<center>` centres a whole table, not just text. Layout is recomputed on each
  render/resize (in the app), not stored by the worker. (Current subset: single-line
  flex — `flex-wrap` falls back to nowrap — cross-axis `align-items` is not applied,
  and inline markup collapses to plain text with no per-glyph styling.)

  **The DOM is pixel-native and device-independent** — it lays out entirely in CSS px
  and knows nothing about terminals. `Style` lengths are already CSS px (`em`/`rem`
  reduced to a 16px font size), and a relative `width` (`60vw`, `100%`) is resolved
  to px against the pixel viewport / containing block during layout, so `updateLayout`
  does no cell conversion; it just needs the renderer's **glyph size** — `cellPx` wide,
  `linePx` tall — to measure and wrap text (a run's px width is its column count times
  `cellPx`, its px height its line count times `linePx`). `fitWidth` treats an explicit
  width wider than the viewport as auto so a desktop-width page reflows to a narrow
  viewport instead of overflowing. A future graphics renderer would call the same
  `updateLayout` with its own viewport and font metrics and get pixel geometry back.

`display::paint(doc, widthCols, cellPx, linePx)` consumes all of this: it lays the
page out in a `widthCols * cellPx`-px viewport, then **maps each text run's px box
down to terminal cells** (dividing by the glyph size) as it draws onto a
`widthCols`-wide canvas — one **`AttributedString`** per page row — that the app scrolls
a window over. This is where *all* px → cell conversion lives — the DOM never does it.
Element boxes are invisible containers (a text terminal has no backgrounds or borders),
so the positioned text is the whole picture: flex columns land side by side, padding
indents, widths clip. The app supplies the terminal's ~8×16px cell
(`termCellPx`/`termLinePx`) — the one piece of terminal knowledge, and it lives in the
app, not the DOM.

**Styling.** Each run carries its inline styling as `dom::TextSpan`s (which stretches
are a link, `<b>`/`<strong>`, `<u>`/`<ins>`); `paint` maps those spans through the same
word-wrap it draws and records them on the output rows, so each row is an
`AttributedString` with per-column **bold**, **underline**, and a **foreground colour**
(links render in blue and underlined). The app draws every content row with
`term::drawText(row, 0 - hscroll, attributedRow)` — its negative-column skip and
right-edge clip window the row horizontally without slicing the opaque AttributedString,
and it honours the run's bold/underline/colour. (Mapping assumes one grapheme ≈ one
column, true for the Latin text styling appears on; a wide glyph may shift an attribute a
cell, never corrupt the frame.)

**Form controls and links** would otherwise be invisible (an `<input>` has no text,
a link looks like plain words), so the inline layout decorates them into the flowed
text: a link's text is wrapped in `[brackets]` (and coloured), a `<button>`/`<input
type=submit>` label in `<angle brackets>`, and an `<input>` becomes a field glyph —
`|S:----|` for text, `|N:----|` for a number, `|P:----|` for a password (the box spans
its `size`, showing the field's value then `-` filler), `[ ]`/`( )` for a
checkbox/radio. A `<textarea>` is a replaced block drawn as `rows` lines of `|----|`
(each `cols` wide). A space inside a field renders as U+00A0 so the inline whitespace
collapse and the word wrap cannot shrink or split the box.

### Form data

The document describes a page's form **structure**; it never holds what you type.
`dom::indexFields` stamps each editable `<input>` with its ordinal in document order —
the identity the renderer exposes as a Tab target — and `dom::fieldSpecs` describes
each field: which `<form>` owns it, the key it takes in that form's data, how it draws,
and the value the page shipped it with. Fields outside every `<form>` share one
implicit form, numbered last.

The app holds the live values as **one `json::JsonObj` per form**, keyed by each
field's data key (its `name`, made unique within the form, synthesized when the element
has none) — the shape a form submission has. A keystroke rewrites that one small
object. This is why the values do not live in the tree: a document is immutable, so
recording a keystroke in it means rebuilding the whole thing — measured at 70–330 ms
per keystroke on a page at the parser's node cap, and degrading the longer you type.
Against the JSON object it is a flat 1–3 ms on any page.

Two paths draw a field, and both render through `dom::fieldBox` so they cannot
disagree. A **full re-layout** (load, resize, width mode) folds the values into a
throw-away copy of the document with `dom::applyFieldValues` and paints that, leaving
the page's own document untouched. **Typing** repaints nothing else: a field's box is a
fixed `size` columns wide whatever it holds, so editing one moves nothing on the page,
and the app stamps just the edited box over the cached rows.
