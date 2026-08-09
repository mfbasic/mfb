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
| `display/`  | package    | Renders a `dom::Node`: `paint(node, width)` → a positioned 2D canvas (via `Layout`), `render(node, width)` → reflowed text, `tree(node, width)` → an indented DOM tree. |
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
  it, **Esc** cancels. `https://` is assumed when you omit the scheme.
- **R** — (Display Mode) switch to **Raw Mode**: the parsed DOM as an indented tree.
- **D** — (Raw Mode) switch back to Display Mode.
- **Up / Down** — scroll vertically. **Left / Right** — pan horizontally across a
  page laid out wider than the terminal (a mouse/trackpad's scroll drives these too,
  via the terminal's alternate-screen scroll translation).
- **M** — cycle the display width mode: **(s)**tandard (the terminal width),
  **(w)**ide (≥300 columns), **(e)**xtra-wide (≥600). The current mode shows as
  `(s)`/`(w)`/`(e)` just before `Files` in the footer; loading a new address resets
  it to standard. Wider-than-terminal modes are navigated with Left / Right.
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
footer's `Files: n/m` counts the HTML document plus each stylesheet fetched.

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
  independent CSS px**, with `-1` meaning `auto`), not an open string bag.
  `dom::resolveStyles(doc)`
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
  (`x, y, width, height` in output cells, from the viewport's top-left), stored on
  every `ElementNode` *and* every `TextNode` (a text run's box is where it wraps).
  `dom::updateLayout(doc, width, metrics)` derives it from `Style`. A `display:flex`
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
  `<center>` centres a whole table, not just text. Text is
  measured by `strings::displayWidth`. Layout is recomputed on each
  render/resize (in the app), not stored by the worker. (Current subset: single-line
  flex — `flex-wrap` falls back to nowrap — cross-axis `align-items` is not applied,
  and inline markup collapses to plain text with no per-glyph styling.)

  Because `Style` lengths are CSS px but the output is in cells, `updateLayout`
  first maps px → cells with a **`Metrics`** — `scaleX`/`scaleY` (output cells per
  CSS px) plus a `fitWidth` policy. `dom::pixelMetrics()` is 1:1 (a pixel canvas);
  `dom::terminalMetrics()` scales down for a ~8×16px monospace cell (so `width:600px`
  → 75 cells, `padding:16px` → 2 cols / 1 row) and, with `fitWidth`, treats a width
  wider than the viewport as auto so a desktop-width page reflows instead of clipping.
  Text stays cell-measured; only the authored box lengths scale.

`display::paint(doc, width, metrics)` consumes all of this: it runs `updateLayout`
and then draws every text run at its box onto a `width`-wide canvas (one String per
page row) that the app scrolls a window over. Element boxes are invisible containers
— a text terminal has no backgrounds or borders — so the positioned text is the
whole picture: flex columns land side by side, padding indents, widths clip. The app
paints with `dom::terminalMetrics()`.

**Form controls and links** would otherwise be invisible (an `<input>` has no text,
a link looks like plain words), so the inline layout decorates them into the flowed
text: a link's text is wrapped in `[brackets]`, a `<button>`/`<input type=submit>`
label in `<angle brackets>`, and an `<input>` becomes a field glyph — `|S:----|` for
text, `|N:----|` for a number (the dashes span its `size`), `[ ]`/`( )` for a
checkbox/radio. A `<textarea>` is a replaced block drawn as `rows` lines of
`|----|` (each `cols` wide). These are plain glyphs today (no color).
