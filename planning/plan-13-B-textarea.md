# plan-13-B: `app::TextArea`

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-13-S (shadow tree + solver) **and** plan-13-T (`text::AttributeString`).
Feature-wide precondition: plan-13 master §Prerequisites.
Produces: the `app::TextArea` widget and the attribute serializer on both backends.

A multi-line attributed text editor whose value is a `text::AttributeString`.

The single behavioral outcome: styled content round-trips through user edits —
value → native → user edit → value — with spans preserved.

**This document is what remains of the old plan-13-B after its Phase 1 was split out.**
That phase (`text::AttributeString`) is now plan-13-T and blocks on nothing; the old
header's "Depends on plan-13-A being landed" was wrong for it.

References (read first):

- `planning/plan-13-T-attributestring.md` — the value type this widget carries.
- `planning/plan-13-B-app-textarea.md` §3–§4 (the 2026-07-09 original) — the design this
  preserves.
- `planning/plan-13-E-events-input.md` — the `Input` drain protocol TextArea reuses
  exactly: text + user-edited + submit latches at `sync`, program-set-wins-this-frame.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-S has landed (shadow tree; solver for the fill case) | `rg -n '_mfb_rt_app_layout' src/` | **NOT MET** |
| plan-13-T has landed (`AttributeString`) | `rg -n 'AttributeString' src/builtins/` | **NOT MET** |
| plan-13-E has landed (the Input drain protocol) | `rg -n 'host_input_drain' src/` | **NOT MET** |
| A backend exists to render into (13-M and/or 13-G) | `rg -n 'host_present' src/target/` | **NOT MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- `app::TextArea` as a shadow node kind + an `app::Widget` variant (+ its `WIDGET_VARIANTS`
  row) + an internal `app::destroy(RES app::TextArea)` close op.
- `addTextArea`/`addAttributedTextArea`; `getText`/`setText`/`getValue`/`setValue`;
  `editable`/`wrap`; `valueChanged`.
- A **leaf** to the solver with a fill-preferring intrinsic size — a solver *input*, not a
  solver change.
- The attribute serializer: `AttributeString` ⇄ `NSAttributedString` (macOS) and
  ⇄ `GtkTextBuffer` tags (GTK4), preserving overlapping spans and precedence.

### Non-goals (explicit constraints)

- **No solver change.** TextArea is a leaf. If the solver needs to know about it, the
  design is wrong.
- **No new `text::` surface.** 13-T owns the value type; this consumes it.
- **No rich-text editing UI** (toolbars, shortcuts). The widget carries attributed content;
  authoring affordances are the program's.
- **Adding this variant must require zero per-op edits** — variant→union widening gives
  the widget-wide ops for free. If it does not, 13-L or 13-A is incomplete.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| New seam ops | **5** (`host_create_textarea`, `_set_value`, `_set_editable`, `_set_wrap`, `_drain`) | 2026-07-09 plan-13-B §4.3 |
| — × 3 backends | **15 implementations** | macOS, GTK4, headless |
| `app::`/`text::` callables this unit adds | ~10 (the `app::` half of the old B's 25) | the surface section |
| Widget variants after this lands | 6 concrete + 1 union | master |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `text::AttributeString` is independent of `app::` | **CONFIRMED** | 13-T; the old B's own Phase 1 called it "genuinely headless" |
| TextArea is a solver *input*, not a solver change | **CONFIRMED, conditional** | true only if the solver already handles `Size < 0` fill — which 13-S delivers. Verify before relying on it |
| Variant→union widening gives widget-wide ops for free | **UNVERIFIED — an acceptance criterion** | assert it on **all three** checker paths; "zero per-op edits" is the test |
| The value⇄native⇄value round trip is a fixed point | **UNVERIFIED — the risk concentration** | proven by round-trip tests, not by construction |

## 3. Design Overview

Two pieces: the widget (mechanical, reuses the Input shape) and the serializer (the risk).

**Where design uncertainty concentrates: the serializer's fixed point.** `AttributeString`
encodes overlapping spans with highest-id-wins resolved at read. `NSAttributedString` and
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
Rejected: the native models are lossy relative to `AttributeString`'s overlap semantics, so
the value would degrade on every read even without edits.

## Compatibility / Format Impact

- **New:** `app::TextArea`, 5 seam ops × 3 backends, one `WIDGET_VARIANTS` row, one close
  op.
- **Unchanged:** the solver, `text::AttributeString`, the seam contract's existing 26 ops.

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
`valueChanged`, using only 13-S's model. **Adding the variant required zero per-op edits** —
if it did not, stop and fix 13-A/13-L rather than patching here.
Commit: —

### Phase 2 — the serializer, one backend (the risk)

- [ ] macOS: `AttributeString` ⇄ `NSAttributedString`, preserving overlapping spans and
      precedence.
- [ ] Round-trip tests: value → native → **no user edit** → value is identical; then edits
      at a span start, at a span end, and between adjacent spans.

Acceptance: the round trip is a **fixed point** for untouched content, and an edit at each
of the three boundary cases moves exactly the spans it should. This is the phase that
decides whether the design works.
Commit: —

### Phase 3 — the second backend

- [ ] GTK4: `AttributeString` ⇄ `GtkTextBuffer` tags.
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

1. **Whether `getValue` returns `AttributeString` or `String`.** Recommended
   `AttributeString`, with `getText` as the plain accessor — otherwise the attributed value
   is unreachable without a second call and the overload set gets ambiguous against
   `Input`.
2. **Whether `wrap` is a layout property or a widget property.** Recommended widget: the
   solver treats TextArea as a leaf, and wrapping changes its intrinsic height, which the
   measure fn-ptr already reports.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **Phase 1 (`text::AttributeString`) split out as plan-13-T.** The old
  header said "Depends on plan-13-A being landed"; that phase's own text called it
  "independently valuable" and "genuinely headless". It blocks on nothing.
- 2026-07-20 — **`Depends on:` moved into the header.** The old document buried it 466
  lines in, inside the Phases section, where a reader deciding what to land first never
  sees it.
- 2026-07-20 — Documentation destination corrected to `stdlib/` + `man/builtins/`; the
  draft named `mfb spec package`, the binary container format.

## Summary

The engineering risk is the serializer's fixed point. `AttributeString`'s overlap model and
the two native models do not agree, so the round trip must be proven to leave untouched
content byte-identical — otherwise every edit degrades the document a little and nobody
notices until the whole thing is bold.

The widget itself is mechanical: it reuses the `Input` drain shape exactly and is a leaf to
the solver. Its one interesting assertion is that adding a variant required **zero**
per-op edits — which is the real test of whether 13-L's widening actually works.

What is left untouched: the solver, `text::AttributeString`, and the seam's existing ops.
