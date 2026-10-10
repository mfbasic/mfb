# plan-13-H: `app::TextArea`

Last updated: 2026-10-04 (value type repointed to the landed `astrings::AttributedString`)
Effort: medium (1h–2h)
Depends on: plan-13-D (shadow tree + solver). The value type is plan-89's
`AttributedString` (the `astrings` package), which has already landed.
Feature-wide precondition: plan-13 master §Prerequisites.
Produces: the `app::TextArea` widget and the attribute serializer on both backends.

A multi-line attributed text editor whose value is an `AttributedString`, the
always-in-scope built-in type that the `astrings` package operates on.

The single behavioral outcome: styled content round-trips through user edits —
value → native → user edit → value — with spans preserved.

**This document is what remains of the old plan-13-H after its Phase 1 was split out.**
That phase became plan-13-B (`text::AttributeString`), which plan-89 superseded
(2026-08-08) with `astrings::AttributedString`. plan-89 A–E have landed, so the value
type this widget carries already exists and nothing here waits on it.

References (read first):

- `mfb spec stdlib astrings` (`src/docs/spec/stdlib/15_astrings.md`) — the semantic model
  of the value this widget carries: inclusive scalar ranges, the never-coalesced overlay,
  and read-time resolution.
- `mfb man astrings` — the per-function surface (`fromString`, `addAttribute`,
  `removeAttribute`, `clearAttributes`, `getAttributes`, the `Attribute` constructors).
- `src/codegen/builtins/astrings/mod.rs` — the package registration, including the
  `internal_only` overlay bridges `readSpans`/`writeSpans`/`scalarLen`.
- `planning/completed/plan-89-{A..E}-*.md` — how the type was designed and landed.
- `planning/completed/superseded-plan-13-B-app-textarea.md` §3–§4 (the 2026-07-09
  original) — the widget design this preserves.
- `planning/plan-13-G-events-input.md` — the `Input` drain protocol TextArea reuses
  exactly: text + user-edited + submit latches at `sync`, program-set-wins-this-frame.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-D has landed (shadow tree; solver for the fill case) | `rg -n '_mfb_rt_app_layout' src/` | **NOT MET** |
| plan-89 has landed (`AttributedString`, the `astrings` package) | `ls planning/completed/plan-89-{A,B,C,D,E}-*` → 5 matches, and `rg -n 'name: "addAttribute"' src/codegen/builtins/astrings/` → 1 match | **MET 2026-10-04** |
| plan-13-G has landed (the Input drain protocol) | `rg -n 'host_input_drain' src/` | **NOT MET** |
| A backend exists to render into (13-E and/or 13-F) | `rg -n 'host_present' src/target/` | **NOT MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- `app::TextArea` as a shadow node kind + an `app::Widget` variant (+ its `WIDGET_VARIANTS`
  row) + an internal `app::destroy(RES app::TextArea)` close op.
- `addTextArea`/`addAttributedTextArea`; `getText`/`setText`/`getValue`/`setValue`;
  `editable`/`wrap`; `valueChanged`.
- A **leaf** to the solver with a fill-preferring intrinsic size — a solver *input*, not a
  solver change.
- The attribute serializer: `AttributedString` ⇄ `NSAttributedString` (macOS) and
  ⇄ `GtkTextBuffer` tags (GTK4), preserving overlapping spans and precedence. All eight
  `Attribute` members map: the five flags (`Bold`, `Italic`, `Underline`, `Strike`,
  `Overline`), `Font`, `FontSize`, and the packed `0xAARRGGBB` `Foreground`/`Background`.

### Non-goals (explicit constraints)

- **No solver change.** TextArea is a leaf. If the solver needs to know about it, the
  design is wrong.
- **No new `astrings::` surface and no change to `AttributedString`.** plan-89 owns the
  value type and its semantics; this widget consumes them. If the serializer seems to
  need a semantic change (for example, coalescing spans), the serializer is wrong.
- **No rich-text editing UI** (toolbars, shortcuts). The widget carries attributed content;
  authoring affordances are the program's.
- **Adding this variant must require zero per-op edits** — variant→union widening gives
  the widget-wide ops for free. If it does not, 13-A or 13-C is incomplete.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| New seam ops | **5** (`host_create_textarea`, `_set_value`, `_set_editable`, `_set_wrap`, `_drain`) | 2026-07-09 plan-13-H §4.3 |
| — × 3 backends | **15 implementations** | macOS, GTK4, headless |
| `app::` callables this unit adds | ~10 (the `app::` half of the old B's 25) | the surface section |
| Widget variants after this lands | 6 concrete + 1 union | master |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `AttributedString` is independent of `app::` | **CONFIRMED** | it landed under plan-89 with no `app::` code; `rg -n 'app::' src/codegen/builtins/astrings/` → no match |
| The serializer can reach the span overlay | **CONFIRMED, internal only** | the overlay has no user-visible fields. Native code reads and writes it through the `internal_only` bridges `readSpans`/`writeSpans` (`src/codegen/builtins/astrings/func_read_spans.rs`), which users cannot call |
| Foreground/background colors exist on the value | **CONFIRMED** | `AttrTypeNumber` `Foreground`/`Background`, packed `0xAARRGGBB` (`mfb spec stdlib astrings` §The `Attribute` model). The superseded 13-B note that plan-89 omits colors is out of date |
| TextArea is a solver *input*, not a solver change | **CONFIRMED, conditional** | true only if the solver already handles `Size < 0` fill — which 13-D delivers. Verify before relying on it |
| Variant→union widening gives widget-wide ops for free | **UNVERIFIED — an acceptance criterion** | assert it on **all three** checker paths; "zero per-op edits" is the test |
| The value⇄native⇄value round trip is a fixed point | **UNVERIFIED — the risk concentration** | proven by round-trip tests, not by construction |

## 3. Design Overview

Two pieces: the widget (mechanical, reuses the Input shape) and the serializer (the risk).

**Where design uncertainty concentrates: the serializer's fixed point.** `AttributedString`
stores overlapping spans over inclusive scalar ranges, never merged or coalesced on write,
and resolves them at read time: for each member, the covering span with the **highest
start** wins, and ties go to the later insertion (`mfb spec stdlib astrings` §Read-time
resolution). `NSAttributedString` and
`GtkTextBuffer` tags each have their own overlap model. The round trip
value → native → user edit → value must be a **fixed point**: text the user did not touch
must come back with identical spans, or every edit slowly degrades the document's styling.

That is the one property worth proving exhaustively, and it is why the serializer lands
behind the plain-text path rather than with it.

**Where correctness risk concentrates:** span drift across edits. An off-by-one at an edit
boundary does not crash — it extends bold by one character, every time, until the document
is entirely bold. Test insertion at a span start, at a span end, and between two adjacent
spans.

**Rejected alternative:** *store attributes natively and read them back on demand.*
Rejected: the native models are lossy relative to `AttributedString`'s overlap semantics, so
the value would degrade on every read even without edits.

## Compatibility / Format Impact

- **New:** `app::TextArea`, 5 seam ops × 3 backends, one `WIDGET_VARIANTS` row, one close
  op.
- **Unchanged:** the solver, `AttributedString` and the `astrings` package, the seam
  contract's existing 26 ops.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — the widget, plain-text path

- [ ] Shadow node kind + `app::Widget` variant + `WIDGET_VARIANTS` row + the internal
      `app::destroy(RES app::TextArea)` close op.
- [ ] `addTextArea`/`addAttributedTextArea`; `getText`/`setText`/`getValue`/`setValue`;
      `editable`/`wrap`; `valueChanged`.
- [ ] **Verify variant→union widening gives `getVisible`/`setVisible`/`getSize`/`setSize`/
      `frame` for free on all three checker paths.**
- [ ] Tests: `tests/syntax/app/textarea-*`, incl. a skipped-middle-argument rejection and a
      `getValue` overload-by-argument-type test against the `Input` form.

Acceptance: a plain-text multi-line TextArea builds, sets/gets its value and reports
`valueChanged`, using only 13-D's model. **Adding the variant required zero per-op edits** —
if it did not, stop and fix 13-C/13-A rather than patching here.
Commit: —

### Phase 2 — the serializer, one backend (the risk)

- [ ] macOS: `AttributedString` ⇄ `NSAttributedString`, preserving overlapping spans and
      precedence. Convert between `AttributedString`'s scalar indices and
      `NSAttributedString`'s UTF-16 ranges; test with a non-BMP scalar inside a span.
- [ ] Round-trip tests: value → native → **no user edit** → value is identical; then edits
      at a span start, at a span end, and between adjacent spans.

Acceptance: the round trip is a **fixed point** for untouched content, and an edit at each
of the three boundary cases moves exactly the spans it should. This is the phase that
decides whether the design works.
Commit: —

### Phase 3 — the second backend

- [ ] GTK4: `AttributedString` ⇄ `GtkTextBuffer` tags. GTK iterators count characters
      and byte offsets are UTF-8; repeat the non-BMP-scalar test.
- [ ] Re-run the whole Phase 2 matrix.

Acceptance: identical round-trip results on GTK4.
Commit: —

## Validation Plan

- Tests: syntax fixtures for the surface; round-trip tests for the serializer, per backend.
- Coverage check: `tests/syntax/app/` is golden-backed; the round-trip proofs are
  on-device and stated as such.
- Runtime proof: macOS and the Debian aarch64 GTK4 box.
- Doc sync: `src/docs/spec/stdlib/` + `src/docs/man/builtins/app/` — **not
  `src/docs/spec/package/`**, which the 2026-07-09 draft named and which is the binary
  container format (master §2.5).
- Acceptance: the project's full suite.

## Open Decisions

1. **Whether `getValue` returns `AttributedString` or `String`.** Recommended
   `AttributedString`, with `getText` as the plain accessor — otherwise the attributed value
   is unreachable without a second call and the overload set gets ambiguous against
   `Input`.
2. **Whether `wrap` is a layout property or a widget property.** Recommended widget: the
   solver treats TextArea as a leaf, and wrapping changes its intrinsic height, which the
   measure fn-ptr already reports.

## Corrections

<!-- Filled in during execution. -->

- 2026-10-04 — **The value type is plan-89's `AttributedString`, which has landed.**
  plan-13-B (`text::AttributeString`, inline markers) was superseded on 2026-08-08 by
  plan-89 (`astrings::AttributedString`, a separate overlay). This plan still named 13-B
  and gated on `rg 'AttributeString' src/builtins/`, a path that no longer exists, so the
  row read NOT MET for a type that has existed since plan-89. The dependency,
  prerequisite row, references and design text now point at `AttributedString`. Two
  facts changed with it: overlap resolution is highest-start-wins, ties to the later
  insertion (not highest-id-wins), and colors exist as packed `Foreground`/`Background`.

- 2026-07-20 — **Phase 1 (`text::AttributeString`) split out as plan-13-B.** The old
  header said "Depends on plan-13-C being landed"; that phase's own text called it
  "independently valuable" and "genuinely headless". It blocks on nothing.
- 2026-07-20 — **`Depends on:` moved into the header.** The old document buried it 466
  lines in, inside the Phases section, where a reader deciding what to land first never
  sees it.
- 2026-07-20 — Documentation destination corrected to `stdlib/` + `man/builtins/`; the
  draft named `mfb spec package`, the binary container format.

## Summary

The engineering risk is the serializer's fixed point. `AttributedString`'s overlap model and
the two native models do not agree, so the round trip must be proven to leave untouched
content byte-identical — otherwise every edit degrades the document a little and nobody
notices until the whole thing is bold.

The widget itself is mechanical: it reuses the `Input` drain shape exactly and is a leaf to
the solver. Its one interesting assertion is that adding a variant required **zero**
per-op edits — which is the real test of whether 13-A's widening actually works.

What is left untouched: the solver, `AttributedString` and the `astrings` package, and the
seam's existing ops.
