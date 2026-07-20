# plan-13-B: `app::` GUI package — TextArea + AttributeString

Last updated: 2026-07-09
Effort: large

Part **B** of plan-13 (the `app::` GUI feature). Extends
[plan-13-A](plan-13-A-app-builtin.md) (the `app::` native-widget package)
with a **multi-line attributed text editor** and the text type it needs:

- a **TextArea** — a multi-line editable text field whose value is a new **`AttributeString`**
  type (rich/attributed text), and
- **`AttributeString`** — a shared `text::` text type carrying overlapping character-attribute
  spans (bold / italic / underline / strike / font / fg / bg color / size).

The single behavioral outcome: a `mfb build -app` program can present a multi-line
attributed-text editor whose styled content round-trips through user edits — slotting into
plan-13-A's shadow-tree + `sync` model over the same, additively-extended §8 host seam.

The **Table** widget (a widget-cell grid) was split out to
[plan-13-C](plan-13-C-app-table.md). Its cells hold ordinary widgets, so a Table shows
attributed text by hosting a `TextArea` cell — plan-13-B is how attributed content reaches a
Table, but there is no hard ordering dependency between the two.

It complements:

- [`planning/plan-13-A-app-builtin.md`](plan-13-A-app-builtin.md) — the base `app::` design
  (§2 lifetime model, §5 surface, §7 shadow/`sync`, §8 host seam, §10 language checkpoints).
  This plan reuses every one of those and adds to them; it changes none.
- [`planning/plan-13-C-app-table.md`](plan-13-C-app-table.md) — the Table widget; its cells hold
  ordinary widgets, so a `TextArea` cell is how a Table displays this plan's `AttributeString`.
- `./mfb spec memory` — the `String` memory/ABI representation that `AttributeString` composes
  over (the core trick of §3 below).
- `./mfb spec package` — where the `app::`/`text::` surface and new types are documented.
- `./mfb spec language` — types, overload resolution, resource unions, operator overloading.
- `./mfb spec threading` — `AttributeString` inherits `String`'s thread-transfer rules.

## 1. Goal

- **AttributeString**: a new first-class text type carrying overlapping character-attribute
  **spans** (bold / italic / underline / strike / font / fg / bg color / size), applied by range
  via typed `text::` descriptors, with `toString()` / `toAttributeString()` conversions. Its
  representation is a compound `{ text: String, lut }` built from an ordinary `String` + a
  per-value attribute table — **no new memory primitive**, so it inherits String's
  copy/freeze/transfer ABI (§3).
- **TextArea**: `app::TextArea` — a multi-line editable widget (`NSTextView` in an `NSScrollView`
  / `GtkTextView` in a `GtkScrolledWindow`) whose value is an `AttributeString`. Program can
  read/write both the plain text and the attributes; user edits round-trip attributes back
  through `sync` (§4).

### Non-goals (explicit constraints)

- **No change to plan-13-A's lifetime law, surface, or seam semantics.** `TextArea` is one new
  `RES` widget + one new `app::Widget` union variant; nothing in plan-13-A changes.
- **No change to `String` memory layout / ABI.** `AttributeString` is a *compound* of an existing
  `String` + a table (§3); copy/freeze/thread-transfer/golden output for `String` are unaffected
  (Layout/ABI Impact).
- **No new external dependency.** System toolkits only (AppKit, GTK4), per plan-13-A §1.
- **No change to the layout solver contract.** `TextArea` is a **leaf** `Widget` from the shared
  flex solver's view (measured + framed as one box); it scrolls *internally*. It is the first
  widget with internal scroll and a flexible (fill-preferring) intrinsic size rather than a
  content size — a solver *input*, not a solver change (§4).
- **`app::sync`, mutators, getters stay non-blocking; `app::poll` stays the only wait** (plan-13-A
  §9). `TextArea` event reads are frame-latched shadow reads.

## 2. Current State

plan-13-A is the base. Relevant precedents this plan mirrors or extends:

- **Widget model** (plan-13-A §7): each widget is a worker-side *shadow* node with a dirty flag;
  `sync` reconciles shadow→native (an owned command batch posted to the main thread) and
  native→shadow (event records drained from the event pipe). `TextArea` is a new shadow node
  kind; its events are new frame-latched records.
- **`Input` bidirectional value** (plan-13-A §5/§7): the value is written by *both* user and
  `setValue`, resolved by *program-set-wins-this-frame*, drained via
  `host_input_drain -> (text, userChanged, submitted)`. `TextArea` reuses this shape exactly, with
  `AttributeString` in place of `String` and a per-backend attribute serializer at the seam.
- **`app::Widget` resource union + `compatible()`-routed validation** (plan-13-A §3/§10): the
  widget-wide ops (`getVisible`/`setVisible`/`getSize`/`setSize`/`frame`) accept the union.
  **This is a language change plan-13-A must land first** — plan-13-A Phase 0 amends
  `mfb spec language resource-management` (which today forbids a union in a `RES` parameter)
  and teaches the three checkers. Once landed, adding the `TextArea` variant gives it the
  widget-wide ops **for free**, with no per-op edits — this plan is the first real test of
  that. It does add one line to the `WIDGET_VARIANTS` table in `src/builtins/app.rs`
  (plan-13-A §10, site 2), which a `#[test]` pins against the registered union.
- **Per-widget registered close ops** (plan-13-A §2): `app::TextArea` needs its own
  `app::destroy(ta AS RES app::TextArea)` close op, like every other widget type. A close op
  must name a concrete type, so it can never be the `Widget` union. As with the other widgets
  it is an **internal** close op (registry-only, not user-callable) — scope-drop of the
  `RES app::TextArea` binding is the sole release path; `app::destroy(...)` in user code is an
  unknown function.
- **Trailing-omission argument rule** (plan-13-A §5.0): builtins have no AST-inserted default
  expressions; optional parameters are trailing and omission selects a shorter arity overload.
  A middle parameter cannot be skipped even by name. Every signature and call below obeys this.
- **`term::` type-ID + shared-type precedent** (plan-13-A §10; the `TermColor`/`TermSize`
  reservation): new record/enum type IDs (`AttributeString`, `Attribute`, `Color`, `AttrName`)
  use the high reserved range to dodge the `FIRST_TABLE_TYPE_ID` collision.
- **Overridable `toString`** (memory: `overridable-builtins-returntype-overloads`, `plan-03`):
  `toString(AttributeString)` and `toAttributeString(String)` register through the same
  return-type/overload machinery.
- **Operator overloading** exists (plan-13-A uses algebraic SIMD operator overloads; the vector
  package): `&` on `AttributeString` concatenates two values, remapping span ids (§3.3).
- **String ABI** (`mfb spec memory`): arena-backed, length-prefixed UTF-8 byte buffer.
  `AttributeString`'s `text` field reuses it verbatim — the enabling fact for the whole design.

## 3. AttributeString — overlapping `[ON]`/`[OFF]` spans + a per-value LUT

**The representation.** An `AttributeString` is a compact compound of two parts, both built from
existing ABI primitives (a `String` plus a small table), carried together as one value:

1. **`text`** — an ordinary `String` of the visible characters, with inline **span markers** in a
   reserved PUA range. A span is one attribute *application*, delimited by a matched pair:
   `[ON]<id>` opens the span, `[OFF]<id>` closes it; the `id` links the pair and points at a LUT
   row. Stored form of `"Hello World.\nBack to normal"` with " World." styled bold:
   `"Hello[ON]<1> World.[OFF]<1>\nBack to normal"`. **Spans may overlap freely** — `bold 10–15`,
   `underline 5–12`, `font 0–10` are three independent pairs interleaved in the stream; no nesting
   or RESET is required, because each attribute ends at its own `[OFF]<id>`.
2. **`lut`** — a per-value **attribute table**: one row `(id, name, value)` per span. `id`
   identifies the **span instance** (unique per `setAttribute`, so `[OFF]<id>` closes exactly the
   right span even when several overlap); `name` is the attribute kind (Bold / Italic / Underline
   / Strike / Font / Foreground / Background / Size); `value` is the packed parameter (bool, a font
   name / string-pool ref, an RGBA int, or a size).

The stream carries only *ids*; the LUT holds the definitions — closer to RTF's control table than
to inline SGR. Chosen over a pure-inline toggle stream for two reasons:

- **Overlaps are trivial + mutation is local.** `setAttribute` allocates one id, inserts one
  `[ON]<id>…[OFF]<id>` pair, and adds one LUT row — no re-segmentation of neighbours;
  `removeAttribute` deletes a pair. Independent overlapping spans just interleave.
- **Dedup / reuse.** The id indirection keeps parameter payloads out of the stream; identical
  payloads can share a string/color pool entry (a later optimization) so a value with many
  same-styled spans (syntax highlighting; a table column) doesn't repeat the payload inline.

**The one cost moves to render, not edit.** Because spans overlap, the *effective* attribute set
at a position is "for each attribute name, the highest-id span of that name whose `[ON]…[OFF]`
covers the position" (§3.3). Rendering therefore **flattens** the overlapping spans into
contiguous native runs — a standard interval sweep carrying a per-name highest-id winner — done
by the serializer at `sync` (§4.2), which already has a serialize step. `getAttributes` runs the
same resolution at a single point. Edits stay O(1)-ish in the span count; only render pays the
flatten.

**The LUT is per-value, not global.** It lives *inside* each `AttributeString`, so values stay
self-contained, purely functional, and **thread-transfer-safe** — no shared global registry to
synchronize. `getValue` → `setValue` on another widget carries the styles with it. (A global
registry is the tempting-but-wrong alternative: it would break transfer and purity.)

**`&` is not free byte-concat.** Concatenating `a & b` must **remap `b`'s ids** into `a`'s
namespace: offset (or reallocate) `b`'s ids, rewrite `b`'s inline `[ON]`/`[OFF]` references, and
append its LUT rows. O(len + lut) per concat — cheap, but real; the price of the id indirection.

**ABI note.** `AttributeString` is **not** "literally a `String`" — it is a compound
`{ text: String, lut: attrTable }`. Still **no new memory primitive**: it composes from an existing
`String` and a table, both already copy/freeze/thread-transfer-able, so it rides plan-13-A's
shadow-`value` machinery without new ABI work (Layout/ABI Impact). It is a *distinct type*:
`len`/indexing count visible Unicode scalars only, and the markers/LUT are never observable as text.

### 3.1 The inline marker encoding (reserved PUA range)

- **Reserved block: `U+F0000`–`U+F01FF`** (Supplementary Private-Use-Area-A), "MFB rich-text
  control codes." Chosen over BMP PUA (`U+E000…`, squatted on by icon fonts / vendor glyphs and
  *does* occur in real text) because SPUA-A essentially never appears in real text. `toString`
  strips this **whole** block; a literal codepoint from it inside an `AttributeString` is
  documented as reserved.
  > The block must span `U+F0000`–`U+F01FF`, not `U+F0000`–`U+F00FF`. An earlier draft reserved
  > only the low 256 while placing the id byte-carriers at `U+F0100`–`U+F01FF` — *outside* the
  > block it promised to strip — so `toString` would have left every carrier codepoint visible
  > in the plain text.
- Only **two markers** live in the stream: `ON` (`U+F0000`) and `OFF` (`U+F0001`), each followed
  by *id* byte-carrier codepoints (sub-range `U+F0100`–`U+F01FF`, each carrying one byte 0–255; two
  carriers ⇒ up to 65 535 spans per value). All *parameter* data (colors, sizes, font names) lives
  in the LUT, **not** the stream. No `RESET` (each span ends at its own `[OFF]<id>`).
- **Spans, not a stateful cursor**: `[ON]<id>` opens a span that stays active until its matching
  `[OFF]<id>`, independent of any other open span. A position's effective attributes = all spans
  currently open over it, resolved per attribute *name* by **highest id wins** (§3.3). Ids are
  allocated in increasing order, so "highest id" is "most recently applied" — last-writer-wins
  falls out of the encoding with no re-segmentation. Well-formedness invariant: every `id`
  appears exactly once as `ON` and once as `OFF`, with the `ON` before the `OFF` (validated on
  construction/concat).

### 3.2 Conversions & length semantics

```
toString(a AS AttributeString) AS String            ' visible text: markers stripped, LUT ignored
toAttributeString(s AS String) AS AttributeString   ' wrap plain text: text := s, empty LUT
```

`len`, indexing, and slicing count **visible Unicode scalar values only** (marker codepoints
skipped) — the reason `AttributeString` is a distinct type and not routed through plain
`strings::` ops (which would count/split the markers and misread the LUT).

**Scalars, not graphemes** — deliberately, and matching the rest of the language. `strings::`
indexes by Unicode *scalar* (`mfb man strings`: *"Index- and count-based functions (find, mid,
left, right) measure positions in zero-based Unicode scalar values, not bytes or graphemes"*);
`graphemes`/`graphemeAt`/`graphemesCount` are the explicit exception. An `AttributeString`
whose positions were graphemes, but whose `toString` output feeds scalar-indexed `strings::mid`
and `strings::find`, would be a silent-corruption footgun. The builtin is `len`, not `length`
(`src/builtins/general.rs:4`).

The one thing scalars cost: a caller may address a position inside a grapheme cluster, exactly
as `strings::mid` lets them. Marker insertion still never splits a base+combining sequence — the
span endpoints are clamped outward to the enclosing cluster boundary — so the rendered result
degrades gracefully rather than producing mojibake. That clamp is §3.4 invariant (4).

### 3.3 The `text::` surface — attribute descriptors + position-based mutation

`AttributeString`, `Attribute`, `Color`, and the builders live in a new shared **`text::`**
package (**locked**, §Open Decisions): the type is not GUI-specific — `term::` can render the same
value to ANSI later — and `app::TextArea` consumes `text::AttributeString` as its value type.
Styling is **typed descriptor + range** (locked, §Open Decisions), not a `{name, value}` literal:
each descriptor function returns an opaque `Attribute`; `setAttribute` applies one over a range.
This mirrors `NSMutableAttributedString.addAttribute(range:)` with type-safe values.

```
' Attribute descriptors — the vocabulary (each returns one opaque Attribute):
text::bold() AS Attribute
text::italic() AS Attribute
text::underline() AS Attribute
text::strike() AS Attribute
text::font(name AS String) AS Attribute
text::foreground(c AS Color) AS Attribute
text::background(c AS Color) AS Attribute
text::size(points AS Float) AS Attribute            ' clamped

' Construction / mutation (return a NEW value — AttributeString is immutable) / introspection:
text::plain(s AS String) AS AttributeString
text::setAttribute(value AS AttributeString, attr AS Attribute, start AS Integer, end AS Integer) AS AttributeString
text::removeAttribute(value AS AttributeString, attr AS Attribute, pos AS Integer) AS AttributeString
text::clearAttributes(value AS AttributeString, start AS Integer, end AS Integer) AS AttributeString
text::getAttributes(value AS AttributeString, pos AS Integer) AS List OF Attribute
' `&` concatenates two AttributeStrings, remapping the right operand's ids into the left's LUT (§3).
```

`start`/`end`/`pos` are **visible scalar indices** (0-based, `end` exclusive), *never* raw
byte/marker offsets — the user never counts control codes (§3.4). Semantics:

- **`setAttribute(value, attr, start, end)`** inserts a new span and **never re-segments its
  neighbours**: allocate the next id, add the LUT row `(id, attr.name, attr.value)`, and place
  `[ON]<id>` at `start` and `[OFF]<id>` at `end` (mapped to raw offsets via §3.4). Independent
  attributes coexist by overlap.
  **Same-name overlap rule — highest id wins, resolved at read/flatten, not at insert.** When two
  spans of the same *name* cover a position, the one with the **larger id** supplies the value
  there. Since ids increase with each `setAttribute`, that is exactly last-writer-wins:
  `Font=serif` applied over `Font=mono` reads as serif inside the new range and mono outside it,
  with the mono span left physically intact. Different *names* (Bold vs Font) always coexist.
  > An earlier draft asserted both "one pair, one row, no re-segmentation of neighbours" *and*
  > "the new span wins on its range" — which, if resolved at insert time, requires trimming or
  > splitting the old span and contradicts the first claim. Resolving at read time makes both
  > true, keeps insertion O(1) in the number of existing spans, and gives `getAttributes` its
  > precedence rule for free.
- **`removeAttribute(value, attr, pos)`** deletes the **effective** span of `attr`'s *name* at
  visible `pos` — the highest-id same-name span covering `pos` (the value is ignored: you remove
  "the bold here", not "bold=TRUE here"). A lower-id same-name span underneath is thereby
  revealed, which is the intended inverse of last-writer-wins.
- **`clearAttributes(value, start, end)`** removes *all* spans' coverage over the range. This is
  the one op that **does** re-segment: a span straddling a boundary is split into the surviving
  outside piece(s), each keeping the original span's LUT row under a fresh id (so relative
  precedence order is preserved).
- **`getAttributes(value, pos)`** returns the **effective** attribute set at visible `pos`: for
  each name covering `pos`, the value of that name's highest-id covering span. At most one
  `Attribute` per name is returned.

Types:

```
TYPE Color                       ' shared; high reserved type-ID range (term:: precedent)
    r AS Integer                 ' 0..255 (clamped)
    g AS Integer
    b AS Integer
    a AS Integer                 ' 0..255, 255 = opaque
END TYPE

text::AttrName  Bold, Italic, Underline, Strike, Font, Foreground, Background, Size   ' LUT-row `name`
```

`Attribute` is an **opaque** record — `{ name AS text::AttrName, flag AS Boolean, num AS Integer,
str AS String }`, with only the field its `name` needs populated (Bold ⇒ `flag`; Size ⇒ `num`
tenths-of-point; Foreground/Background ⇒ `num` packed RGBA; Font ⇒ `str`). It is constructed
**only** via the descriptor functions and read only via `getAttributes`, so the packing stays
private (Open Decision notes a union alternative). Worked example — style `" World."` (visible
scalars 5..12) bold-red:

```
' "\n" is the newline escape (plan-27/28); there is no CHR builtin.
LET v  = text::plain("Hello World.\nBack to normal")
LET v2 = text::setAttribute(v,  text::bold(), 5, 12)
LET v3 = text::setAttribute(v2, text::foreground(Color[r := 200, g := 0, b := 0, a := 255]), 5, 12)
' Both spans open at visible 5 and close at visible 12, so BOTH [ON] markers precede the
' space and BOTH [OFF] markers follow the period:
' v3.text == "Hello[ON]<1>[ON]<2> World.[OFF]<1>[OFF]<2>\nBack to normal"
' LUT: id 1 -> (Bold, TRUE);  id 2 -> (Foreground, 0xC80000FF)
' toString(v3) == "Hello World.\nBack to normal"
' len(v3) == 27   ' "Hello World." = 12, "\n" = 1, "Back to normal" = 14
```

> Two bugs fixed from the earlier draft of this example: `[ON]<2>` was written *after*
> `" World."`, which opens span 2 at visible 12 and closes it at 12 — an **empty** span, not
> 5..12. And `length(v3) == 26` was off by one (12 + 1 + 14 = 27); the builtin is also spelled
> `len`, not `length`.

Overlapping-spans example (the canonical stress case):

```
LET b = text::bold()
LET u = text::underline()
LET f = text::font("monospace")
LET v2 = text::setAttribute(v,  b, 10, 15)     ' bold      scalars 10..15   (id 1)
LET v3 = text::setAttribute(v2, u,  5, 12)     ' underline scalars  5..12   (id 2, overlaps bold)
LET v4 = text::setAttribute(v3, f,  0, 10)     ' font      scalars  0..10   (id 3)
LET v5 = text::removeAttribute(v4, b, 10)      ' drop the effective Bold span at scalar 10 (id 1)
```

Same-name precedence (why "highest id wins" is the whole rule):

```
LET m  = text::setAttribute(v,  text::font("monospace"), 0, 20)   ' id 1
LET s  = text::setAttribute(m,  text::font("serif"),     5, 10)   ' id 2, overlaps id 1
' scalars 0..5  -> Font=monospace  (only id 1 covers)
' scalars 5..10 -> Font=serif      (ids 1 and 2 cover; 2 > 1 wins)
' scalars 10..20-> Font=monospace  (only id 1 covers)
' The id-1 span is never trimmed. removeAttribute(s, text::font(""), 7) drops id 2 and
' restores monospace across 5..10.
```

### 3.4 Visible⇄raw position mapping (core internal machinery)

Public positions are **visible scalar indices**; the stored `text` interleaves marker codepoints
(`ON`+id carriers, `OFF`+id carriers). Every public op taking a `pos` first translates it through a
private mapping, so the user never accounts for control codes:

- `raw_offset_of_visible(value, vpos) -> raw_index`: scan `text` left-to-right, skipping whole
  marker sequences (`ON`/`OFF` + their id carriers) and advancing a visible-scalar counter over
  every other scalar, until the counter reaches `vpos`; return that raw boundary (guaranteed
  *outside* any marker sequence).
- `visible_of_raw(value, raw_index) -> vpos`: the inverse, used by the serializer / drain path.

Both are O(len) scans in v1 (a precomputed prefix index is a later optimization if it shows up
hot). Invariants the mapping must hold, pinned by Phase-1 tests: (1)
`raw_offset_of_visible(v, len(v))` == end-of-text; (2) round-trip
`visible_of_raw(v, raw_offset_of_visible(v, p)) == p`; (3) inserting/removing markers never changes
`toString(v)` or `len(v)`; (4) a marker is never emitted *inside* a grapheme cluster: when a span
endpoint lands mid-cluster (legal, since positions are scalars — §3.2), it is clamped outward to
the enclosing cluster boundary before the marker is written.

## 4. `app::TextArea` — multi-line attributed input

`app::TextArea` is a new **`RES` widget** and a new **`app::Widget` union variant**, so it inherits
the widget-wide ops (`getVisible`/`setVisible`/`getSize`/`setSize`) via `compatible()` automatically
(plan-13-A §10). Native peer: `NSTextView` inside an `NSScrollView` / `GtkTextView` inside a
`GtkScrolledWindow`. It scrolls **internally** and is a **leaf** to the flex solver, preferring to
*fill* (default `Size` `< 0`) rather than reporting a content size.

### 4.1 Surface

Per plan-13-A §5.0 every optional parameter is **trailing**, the `= …` values are supplied by the
implementation (not inserted into the AST), and each arity is its own overload. `text` precedes
`value` because the plain-text constructor is the common case and a named `value :=` would
otherwise have to skip nothing — the ordering is what makes `addTextArea(box, text := "hi")` legal.

```
' Plain-text constructor (the common case).
app::addTextArea(parent AS RES app::Container,
                 text AS String = "",
                 placeholder AS String = "",
                 editable AS Boolean = TRUE,
                 wrap AS Boolean = TRUE,
                 margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::TextArea

' Attributed constructor — a separate name, NOT an overload of the above. An `AttributeString`
' second parameter would collide with the `String` one only by type, and the two would then
' have to agree on parameter names arity-for-arity (plan-13-A §10). A distinct name is free.
app::addAttributedTextArea(parent AS RES app::Container,
                           value AS AttributeString,
                           placeholder AS String = "",
                           editable AS Boolean = TRUE,
                           wrap AS Boolean = TRUE,
                           margin AS app::Spacing = Spacing[top := 0, bottom := 0, left := 0, right := 0]) AS RES app::TextArea

' TextArea's registered close op (plan-13-A §2 — every widget type has one). Internal only,
' NOT exported: scope-drop calls it; `app::destroy(...)` in user code is an unknown function.
app::destroy(ta AS RES app::TextArea) AS Nothing

app::getValue(ta AS RES app::TextArea) AS AttributeString      ' current attributed text (shadow)
app::setValue(ta AS RES app::TextArea, value AS AttributeString) AS Nothing
app::getText(ta AS RES app::TextArea) AS String                ' convenience: toString(getValue(..))
app::setText(ta AS RES app::TextArea, text AS String) AS Nothing   ' convenience: setValue(toAttributeString(text))
app::valueChanged(ta AS RES app::TextArea) AS Boolean          ' user edited since last sync (frame-latched)
app::getEditable(ta AS RES app::TextArea) AS Boolean
app::setEditable(ta AS RES app::TextArea, editable AS Boolean) AS Nothing
app::getWrap(ta AS RES app::TextArea) AS Boolean
app::setWrap(ta AS RES app::TextArea, wrap AS Boolean) AS Nothing
```

(No `submitted`: Enter inserts a newline in a multi-line field. `getSize`/`setSize`,
`getVisible`/`setVisible`, and `frame` come from the `Widget` union.)

Note `getValue`/`setValue` are genuine type-based overloads against plan-13-A's `Input` forms
(`getValue(RES app::Input) AS String` vs `getValue(RES app::TextArea) AS AttributeString`):
same arity, different first-parameter type, same parameter name. That is exactly the shape
`resolve_call` separates by argument type, and it does not need the return-type-overload
machinery.

### 4.2 The attributed round-trip (the risk concentration)

This is exactly plan-13-A §7's bidirectional-`value` rule, with `AttributeString` in place of
`String`. The one new thing is a per-backend **attribute serializer** at the seam:

- **Push (shadow→native), at `sync` when `value` is dirty:** **flatten** the shadow's overlapping
  `[ON]/[OFF]` spans into contiguous native runs (§3, interval sweep) and set them on the view as
  an `NSAttributedString` / `GtkTextTag` spans. *Program-set-wins-this-frame* (plan-13-A §7) is
  unchanged.
- **Pull (native→shadow), at `sync` when `value` is not dirty:** serialize the view's live native
  attributed model back into an `AttributeString` — walk the native runs and, per attribute *name*,
  open an `[ON]<id>` where its value begins and close `[OFF]<id>` where it ends (coalescing a name's
  identical adjacent runs into one span) — and latch `valueChanged` iff the user edited
  (`textDidChange` / `GtkTextBuffer::changed`), never on a programmatic push (no echo loop — plan-13-A
  §7).

**Why this is bounded, not a text engine.** Between syncs the **native view owns the attributed
model** — typing inherits adjacent attributes, cut/paste preserves runs, etc., all handled by
AppKit/GTK. We never track cursors or edits; we only flatten-on-push and re-span-on-drain. The
correctness lives entirely in one faithful **flatten/serialize pair per backend** (Phases 3–4
below), tested by a headless value⇄native⇄value fixed-point check (identity on the supported
attribute set, modulo span-id renumbering and same-name coalescing).

### 4.3 Seam additions (additive to plan-13-A §8)

```
host_create_textarea(value, placeholder, editable, wrap) -> handle
host_textarea_set_value(handle, attr_string)             ' flatten spans -> native runs, push (in the sync command batch)
host_textarea_set_editable(handle, bool)
host_textarea_set_wrap(handle, bool)
host_textarea_drain(handle) -> (attr_string, userChanged) ' serialize native runs -> spans + did-user-edit
```

Per plan-13-A §7 there is no shared mutable state and therefore no "atomic reset": a user edit
pushes a length-prefixed `ValueChanged` record into the event pipe, and `sync` folds it into the
shadow. A `TextArea`'s serialized `AttributeString` is the largest record kind in the protocol —
size it explicitly, and make the native side coalesce consecutive `ValueChanged` records for one
node (last-wins) before writing, so a fast typist cannot flood the pipe.

## Layout / ABI Impact

- **`AttributeString` adds NO new memory primitive.** It is a compound `{ text: String, lut:
  attrTable }` composed from an existing `String` + a table, both already
  copy/freeze/scope-drop/thread-transfer-able; all *existing* `String` golden output is unchanged.
  `mfb spec memory` gains a note describing the compound + the reserved `U+F0000`–`U+F01FF` control
  block and that `text` is a `String` at the byte level; it changes no existing entry.
- **New builtin record/enum type IDs** (`AttributeString`, `Attribute`, `Color`, `AttrName`) use
  the **high reserved type-ID range** (term:: `TermColor`/`TermSize` precedent) to avoid the
  `FIRST_TABLE_TYPE_ID` collision.
- **`app::Widget` union gains one variant** (`TextArea`), plus one row in
  `src/builtins/app.rs`'s `WIDGET_VARIANTS` table (plan-13-A §10, site 2 — `ir::lower`'s
  `resolve_call` is context-free and cannot consult the union registry). The widget-wide ops
  accept it with **no per-op edits**. Verify (§Validation) that variant→union widening
  typechecks for the new variant on all three checker paths and that no op needs enumeration.
  (plan-13-C adds the `Table` variant the same way.)
- **One new registered close op** (`app::destroy(RES app::TextArea)`), per plan-13-A §2.
- **The §8 host seam grows additively** (the `host_textarea_*` calls above). No existing seam call
  changes signature — plan-13-A promised this and it holds.
- **No change to the shared layout solver contract.** `TextArea` is a leaf; it is a new solver
  *input* (fill-preferring, internally scrolling), not a solver change.
- **No change to `String`, `Input`, or any plan-13-A widget.** `Input`/`Label`/`Button`/`Container`
  are untouched.

## Phases

Depends on plan-13-A (the base `app::` shadow/`sync`/seam) being landed. Ordered lowest-risk /
independently-landable first; highest-risk codegen (the attribute serializer) last, behind tests.
Each phase lists its concrete tasks and the acceptance criterion verified before it is done; fill
in `Commit:` with the hash(es) that land it.

### Phase 1 — `text::` AttributeString layer (headless, no GUI)

The rich-text type + attribute machinery, fully unit-testable with no window. Independently
valuable — a `term::` ANSI renderer (and, via a `TextArea` cell, plan-13-C's Table) can consume
it before any widget exists.

- [ ] Register the compound type (`{ text: String, lut: attrTable }`) and the `[ON]<id>`/`[OFF]<id>` span encoding over the reserved block `U+F0000`–`U+F01FF` (§3.1); reserve type IDs in the high reserved range.
- [ ] Implement the visible⇄raw position mapping (§3.4, **scalar** indices) and span insert/remove with the highest-id-wins same-name rule resolved at read/flatten (§3.3).
- [ ] Add the typed `Attribute` descriptors (`text::bold`/`italic`/`underline`/`strike`/`font`/`foreground`/`background`/`size`), `text::plain`, `Color`, `AttrName`.
- [ ] Add `text::setAttribute`/`removeAttribute`/`clearAttributes`/`getAttributes`, `toString`/`toAttributeString`, and the `&` operator overload.
- [ ] Tests (headless): span stream + LUT well-formedness (each id one `ON` before one `OFF`); overlapping spans coexist; `setAttribute` over a *visible* range marks the right scalars despite intervening markers (§3.4); **`toString` strips the whole `U+F0000`–`U+F01FF` block including id carriers** (the bug §3.1 calls out); same-name highest-id-wins **without trimming the loser** (assert the underlying span survives and is revealed by `removeAttribute`); different names coexist; `clearAttributes` splits straddling spans preserving precedence order; `getAttributes` returns at most one `Attribute` per name; marker-invariant `len`/positions (§3.4 invariants 1–4) incl. the mid-cluster clamp; `a & b` remaps the right operand's ids (visible text = concat, both sides' spans preserved, relative precedence preserved).

Acceptance: the full `text::` AttributeString surface passes headless unit tests for encoding, position mapping, overlap precedence, and concatenation — no window required. (This phase is genuinely headless: `text::` is pure worker-side value code with no host seam.)
Commit: —

### Phase 2 — TextArea shadow + surface (plain-text path)

Wire the widget into plan-13-A's shadow/dirty model before any attribute round-trip.

- [ ] Add the `app::TextArea` shadow node + `app::Widget` variant (+ the `WIDGET_VARIANTS` row) + the `app::destroy(RES app::TextArea)` close op (registry-only, internal — not added to the user-callable `app::` call table, per plan-13-A §2); `addTextArea`/`addAttributedTextArea`; `getText`/`setText`/`getValue`/`setValue`; `editable`/`wrap`; `valueChanged` — plumbed through the plan-13-A shadow/dirty model.
- [ ] Verify variant→union widening gives the widget-wide ops (`getVisible`/`setVisible`/`getSize`/`setSize`/`frame`) for free on **all three** checker paths (plan-13-A §10).
- [ ] Plain-text path first (`toAttributeString` of plain) — an `Input`-shaped multi-line field.
- [ ] Tests: `tests/func_app_textarea_*_valid/**` and `_invalid/**`, incl. a skipped-middle-argument rejection and a `getValue` overload-by-argument-type test against the `Input` form.

Acceptance: a plain-text multi-line `TextArea` builds, sets/gets its value, and reports `valueChanged`, using only the plan-13-A model (no attribute serializer yet). Adding the variant required **zero** per-op edits.
Commit: —

### Phase 3 — macOS TextArea backend + attribute serializer

The risk concentration: the value⇄native⇄value fixed point.

- [ ] Implement the macOS backend: `NSTextView`/`NSScrollView`.
- [ ] `host_textarea_set_value` = flatten spans (highest-id-wins per name, §3.3) → `NSAttributedString` runs, applied from the `sync` command batch; `host_textarea_drain` = serialize the live `NSAttributedString` → spans, pushed as a length-prefixed `ValueChanged` event record with per-node last-wins coalescing (§4.2, §4.3).
- [ ] Tests: **value⇄native⇄value fixed-point** — flatten-then-re-span is identity on the supported attribute set (modulo id renumbering / coalescing), run through the plan-13-A `headless` host backend (`--app-host headless`) so it needs no window; the headless host's `TextArea` peer models runs as a plain interval list.

Acceptance: on-device, styled text can be typed, read back, and re-presented losslessly; the fixed-point test passes under both the headless host and AppKit; `setValue` does not echo as `valueChanged` (plan-13-A §7).
Commit: —

### Phase 4 — GTK4 TextArea backend

- [ ] Implement the GTK4 backend against the same seam: `GtkTextView`/`GtkScrolledWindow` + `GtkTextTag`s.
- [ ] The same fixed-point serializer test must pass unchanged.

Acceptance: the fixed-point test passes on GTK4 and the widget is verified on the Debian aarch64 box (plan-05 / plan-13-A §11).
Commit: —

### Phase 5 — Polish, docs, examples

- [ ] `wrap`/`editable` live updates.
- [ ] A worked attributed-editor example; spec/man updates.

Acceptance: the attributed-editor example builds and runs on both backends; docs/spec updated.
Commit: —

## Validation Plan

- **Function tests** (every overload), per repo standard:
  `tests/func_text_plain_*`, `func_text_bold_*`, `func_text_setAttribute_*`,
  `func_text_foreground_*`, `func_app_addTextArea_*`, `func_app_getValue_*` (TextArea overload), …
  each with `_valid/**` and `_invalid/**` (arity + type + union-widening coverage, incl. a
  concrete-type op rejecting a wrong variant to prove `compatible()` stays directional).
- **AttributeString headless proofs** (no window): span stream + LUT encoding golden and the
  well-formedness invariant (each id one `ON` before one `OFF`); overlapping spans coexist;
  **position mapping** — `setAttribute` over a visible range marks exactly those scalars across
  intervening markers, and the §3.4 round-trip/marker-invariance invariants (1–4) hold, incl. the
  mid-cluster endpoint clamp; same-name **highest-id-wins with the loser span left intact** vs
  different-name coexist; `removeAttribute` drops the effective span and reveals the one beneath;
  `clearAttributes` splits straddling spans preserving precedence; `getAttributes` returns at most
  one `Attribute` per name; `toString` strips the **entire** `U+F0000`–`U+F01FF` block (markers
  *and* id carriers); `a & b` visible text = concat with correct id remap (no collision, both
  sides' spans preserved, relative precedence preserved); visible-scalar `len`; the
  **value⇄native⇄value fixed-point** serializer test on each backend (flatten-to-native then
  re-span-back is identity on the supported attribute set, modulo span-id renumbering and
  same-name coalescing) — runnable without a window via `--app-host headless`.
- **Runtime proof** (real behavior, not just golden output): an attributed TextArea program — set
  styled text, edit it, read it back with attributes intact, confirm no `valueChanged` echo on
  `setValue`; on-device on macOS and the Debian aarch64 box.
- **Doc sync**: `mfb spec memory` (AttributeString = `{String, attrTable}` compound; reserved PUA
  block), `mfb spec package` (the `text::`/`app::` surface + new types), `mfb spec language` (if the
  `&` overload or type interaction needs a note); man pages per `.ai/man_template.md` /
  `.ai/man_type_template.md` / `.ai/man_package_template.md`; keep this plan's `Last updated`
  current and remove it in the commit that lands the final phase (per `.ai/planning.md`).
- **Acceptance**: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green; and the
  plan-13-A canonical program still byte-identical (no regression to the base widgets/ABI).

## Open Decisions

- **Home for `AttributeString` / `text::` / `Color`: shared `text::` vs `app::`.** — **Locked:
  shared `text::` package.** The type is not GUI-specific; `term::` can render the same
  `AttributeString` to ANSI and a plan-13-C Table cell hosting a `TextArea` displays it, making a
  shared type strictly more valuable. `app::` depends on `text::` for the TextArea value type
  (§3.3, §4).
- **`setAttribute` extent: range vs single position.** — **Locked: range**
  `setAttribute(value, attr, start, end)` over visible scalar indices (a single `pos` can't
  express an extent). A caret-mode "attribute for text typed after here" is a distinct future op,
  added separately if wanted. (§3.3)
- **Style representation: `[ON]<id>`/`[OFF]<id>` spans (locked) vs current-style-id / pure
  toggles.** — **Locked: paired `[ON]<id>`/`[OFF]<id>` spans + a per-value LUT, no RESET** (§3).
  Spans overlap freely, mutation is local (`setAttribute` = one pair + one row; `removeAttribute` =
  delete a pair), and render flattens overlaps into runs. Rejected: a stateful "current-style = one
  id" (can't represent overlaps without recomputing merged style-sets on every edit) and pure inline
  toggles (repeat payloads per cell). Accepted cost: id-remap on `&` and a flatten at render.
- **Same-name overlap: resolve at insert vs at read.** — **Locked: at read (highest id wins).**
  Resolving at insert (trim/split the older same-name span) is what "the new span wins on its
  range" naively implies, but it contradicts the locked "mutation is local / no re-segmentation of
  neighbours" and makes `setAttribute` O(existing spans). Resolving at read costs nothing extra —
  the flatten sweep already visits every covering span — and makes `removeAttribute` a true
  inverse, revealing the span underneath. `clearAttributes` remains the one re-segmenting op.
  (§3.3)
- **Position units: scalars vs graphemes.** — **Locked: Unicode scalar values**, matching
  `strings::find`/`mid`/`left`/`right` (`mfb man strings`), with `graphemes*` the documented
  exception there and here. An `AttributeString` indexed by graphemes whose `toString` output is
  then sliced by scalar-indexed `strings::` ops would silently corrupt spans. Cost: an endpoint
  may land mid-cluster; it is clamped outward before a marker is written (§3.4 invariant 4).
- **`Attribute` value typing.** — **Locked: typed constructor functions** (`text::bold()`,
  `text::font(s)`, `text::foreground(c)`, `text::size(p)` → opaque `Attribute`). Type-safe and
  discoverable; a `{name, value}` record can't hold Boolean + String + Color in one statically typed
  field. Alternative kept in reserve: a `UNION AttrValue { Boolean String Integer Color }` field if
  an open-ended data-driven form is ever needed. (§3.3)
- **Editable-attributed depth in TextArea v1.** — *Recommend full round-trip* (flatten-on-push /
  serialize-on-drain), since the native view owns the attributed model between syncs so the cost is
  one bounded serializer pair per backend (§4.2). Fallback if a backend serializer proves
  unfaithful: ship *editable-plain + read-only-attributed* first (user edits collapse to plain;
  attributes only when `editable := FALSE`), then upgrade — no surface change. (§4.2)
- **PUA control range.** — **Locked: reserved block `U+F0000`–`U+F01FF` (SPUA-A)**, with
  `ON`=`U+F0000`, `OFF`=`U+F0001`, and id byte-carriers in `U+F0100`–`U+F01FF`. The block must
  cover the carriers — an earlier draft reserved only `U+F0000`–`U+F00FF`, so `toString` would
  have left carrier codepoints in the visible text (§3.1). Font *family* ships now
  (`text::font`) — the family name lives in the LUT row's `str`, not the stream, so no inline
  variable-length problem. Confirm no target-font glyph collisions on either backend.
- **LUT container: flat `List OF (id,name,value)` vs `Map id → row`.** — *Recommend a flat list
  ordered by id for v1* (simple, transfer-trivial; lookup scans or a reverse index); a `Map` is an
  optimization if lookup shows up hot. Optional payload interning (dedup identical `(name,value)`)
  is a later add. (§3)

## Non-Goals (v1)

- **Images/inline attachments** in `AttributeString` and **paragraph-level** attributes (alignment /
  lists / indentation) — the attribute set is bold / italic / underline / strike / font /
  foreground / background / size only (§3.3). (Font *family* is in v1 via `text::font`; what's
  excluded is font weight/variant axes, embedded images, and block-level formatting.)
- The **Table** widget — split to [plan-13-C](plan-13-C-app-table.md).
- Everything plan-13-A already lists as a non-goal (menus, native dialogs, animation/timers, theming)
  remains out of scope.

## Summary

The engineering risk lives in one place, isolated behind headless tests before any on-device work:
**the `AttributeString` attribute serializer** — the span⇄native-runs flatten/serialize pair per
backend (Phases 3–4). It is bounded because the *native view owns the attributed model between
syncs*; we never build a text engine, only a faithful round-trip, gated by a headless fixed-point
test.

Everything else is reuse: `AttributeString` is a compound of an existing `String` + a per-value
attribute LUT (no new memory primitive, no thread-transfer work); `TextArea` is a new `app::Widget`
variant that inherits the widget-wide ops via `compatible()` with zero per-op edits (the first real
test of plan-13-A §10, once its **Phase 0** language amendment lands); it is a leaf to the unchanged
shared solver; and the §8 seam grows only additively. `String`, `Input`, the layout solver, and
every plan-13-A widget stay byte-for-byte untouched. The `text::` layer (Phase 1) is independently
valuable, and a `TextArea` cell is how plan-13-C's Table displays attributed text.

**Hard dependency to note:** this plan cannot begin before plan-13-A **Phase 0** (resource-union
parameters, a spec amendment plus three checker sites) and **Phase 2** (the emitted layout solver
and the `headless` host backend, which Phase 3's fixed-point test runs on). Phase 1 (`text::`) is
the exception — it touches no `app::` surface and can land any time.
