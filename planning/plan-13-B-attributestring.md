# plan-13-B: `text::AttributeString`

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: **nothing.** This is pure worker-side value code with no host seam and no
window. It can land before 13-A.
Produces: `text::AttributeString`, the `[ON]`/`[OFF]` span encoding + per-value LUT, the
typed `Attribute` descriptors, and `setAttribute`/`removeAttribute`/`clearAttributes`/
`getAttributes`/`toString`/`toAttributeString`/`&`. Consumed by 13-H (TextArea) and
optionally 13-I (a `TextArea` table cell).

A rich-text value type: a `String` plus overlapping character-attribute spans (bold,
italic, underline, strike, font, foreground, background, size).

The single behavioral outcome: styled text round-trips through construction,
concatenation, attribute add/remove and flattening with overlapping spans preserved and
precedence resolved — verified entirely headless.

**This unit was buried.** It was plan-13-H's Phase 1, under a header reading "Depends on
plan-13-C (the base `app::` shadow/`sync`/seam) being landed" — while the phase itself
says it is *"independently valuable"* and *"genuinely headless: `text::` is pure
worker-side value code with no host seam."* The header was wrong. Splitting it out means
a `term::` ANSI renderer, or anything else wanting attributed text, can consume it without
a GUI existing.

References (read first):

- `planning/old-plans/superseded-plan-13-B-app-textarea.md` §3 — the design this preserves: the `[ON]<id>` /
  `[OFF]<id>` span encoding over the reserved block `U+F0000`–`U+F01FF`, the per-value
  LUT, and the highest-id-wins same-name rule resolved at read/flatten.
- `src/builtins/strings.rs` — the existing `text::`/`strings::` surface this extends.
- `src/binary_repr/builder.rs:244` — `FIRST_TABLE_TYPE_ID`; new builtin record types must
  use the high reserved range or collide (the `term::` `TermColor`/`TermSize` precedent).

## Prerequisites

None — that is the point of splitting it out.

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| The high reserved type-ID range is still available | `rg -n 'FIRST_TABLE_TYPE_ID' src/binary_repr/` → `builder.rs:244`, `reader.rs:642`, `:672` | **MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run it
> before you continue and again before you decide to stop. If you stop, report every row.

## 1. Goal

- A compound `AttributeString` type (`{ text: String, lut: attrTable }`) with the span
  encoding over `U+F0000`–`U+F01FF`.
- Visible⇄raw **scalar** position mapping, so a caller addressing "characters 3–7" marks
  the right scalars despite intervening markers.
- Overlapping spans coexist; same-name conflicts resolve **highest-id-wins at read time**
  without trimming the loser, so removing the winner reveals what was underneath.
- Typed descriptors: `text::bold`/`italic`/`underline`/`strike`/`font`/`foreground`/
  `background`/`size`, plus `text::plain`, `Color`, `AttrName`.
- `toString` strips **the whole** `U+F0000`–`U+F01FF` block, id carriers included.
- `a & b` concatenates: visible text is the concat, both sides' spans survive, relative
  precedence is preserved, and the right operand's ids are remapped.

### Non-goals (explicit constraints)

- **No widget, no window, no host seam.** If this unit needs one, the split is wrong.
- **No rendering.** This is a value type; drawing it is 13-H's (or a future `term::`
  renderer's) job.
- **No attribute semantics beyond storage and precedence.** `size` is a number, `font` is
  a name — this unit does not resolve fonts or validate colors against a display.
- **Do not change `String`.** `AttributeString` is a distinct type; a plain `String`
  never silently carries spans.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Typed attribute descriptors to add | **8** | `bold`, `italic`, `underline`, `strike`, `font`, `foreground`, `background`, `size` |
| New `text::` functions | **7** | `setAttribute`, `removeAttribute`, `clearAttributes`, `getAttributes`, `toString`, `toAttributeString`, `plain` (+ the `&` operator overload) |
| Reserved private-use scalars for the encoding | **512** (`U+F0000`–`U+F01FF`) | the block plan-13-H §3.1 reserves |
| `strings.rs` size, for scale | *(measure before starting)* | `wc -l src/builtins/strings.rs` |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `text::` is pure worker-side value code with no host seam | **CONFIRMED** | plan-13-H's own Phase 1 acceptance says so; nothing in the surface touches a window |
| Builtin record types need the high reserved ID range | **CONFIRMED** | `FIRST_TABLE_TYPE_ID`, `binary_repr/builder.rs:244` — the `term::` `TermColor`/`TermSize` precedent |
| This unit depends on plan-13-C | **FALSE** | plan-13-H's header said so; its Phase 1 body contradicts it. Nothing here needs the shadow tree, `sync`, or the seam |
| The encoding survives `toString` correctly | **UNVERIFIED — an acceptance criterion** | plan-13-H §3.1 explicitly calls out the bug where id carriers are not stripped |

## 3. Design Overview

Preserved from plan-13-H §3 unchanged: text plus an out-of-band span stream, encoded as
paired `[ON]<id>` / `[OFF]<id>` markers drawn from a reserved private-use block, with a
per-value lookup table mapping id → attribute.

**Where design uncertainty concentrates:** the **visible⇄raw position mapping**. Every
public operation addresses *visible* positions while the storage is raw scalars with
markers interleaved. An off-by-one here does not crash — it applies bold to the wrong
character, which is exactly the kind of wrongness that survives casual testing. Phase 1
therefore leads with the mapping and its invariants, not with the descriptors.

**Where correctness risk concentrates:** three specific traps plan-13-H already
identified, all of which are easy to get subtly wrong and hard to notice:

1. **`toString` must strip the whole block, id carriers included.** Stripping only the
   `[ON]`/`[OFF]` sentinels leaves the id scalars in the visible text — invisible in most
   fonts, and corrupting every length and comparison downstream.
2. **Same-name conflicts resolve at read time without trimming the loser.** If the loser
   is trimmed on write, removing the winner reveals nothing instead of the underlying
   span.
3. **Mid-cluster clamping.** A visible position may land inside a grapheme cluster; the
   mapping must clamp deterministically rather than splitting one.

**Rejected alternative:** *a parallel array of `(start, end, attr)` ranges alongside the
string.* Rejected in plan-13-H and the rejection stands: every string operation
(`concat`, `slice`, insert) would have to maintain the array in lockstep, and the first
one that forgets silently desynchronizes text from style. Inline markers move with the
text by construction.

**Rejected alternative:** *store attributes per character.* Rejected: quadratic memory for
long documents, and it loses the overlap/precedence structure the API exposes.

## Compatibility / Format Impact

- **New:** an `AttributeString` type, 8 attribute descriptors, 7 `text::` functions, an
  `&` overload, and reserved type IDs in the high range.
- **Unchanged:** `String` and every existing `text::`/`strings::` function; no existing
  program's behavior.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — encoding + position mapping (the uncertain part)

- [ ] Register the compound type and reserve its type IDs in the high range.
- [ ] Implement the `[ON]<id>` / `[OFF]<id>` encoding over `U+F0000`–`U+F01FF`.
- [ ] Implement the visible⇄raw **scalar** mapping with its invariants, including the
      mid-cluster clamp.
- [ ] Tests: marker-invariant `len` and positions; the clamp; a `setAttribute` over a
      *visible* range marks the right scalars despite intervening markers.

Acceptance: the mapping is correct across a value with markers interleaved at every
position, including at the ends and mid-cluster. If the mapping is not clean here, the
rest of the unit is built on sand.
Commit: —

### Phase 2 — spans, precedence, and the three traps

- [ ] Span insert/remove; overlapping spans coexist.
- [ ] Same-name **highest-id-wins resolved at read/flatten**, loser untrimmed.
- [ ] `toString` strips the whole `U+F0000`–`U+F01FF` block **including id carriers**.
- [ ] `clearAttributes` splits straddling spans, preserving precedence order.
- [ ] `getAttributes` returns at most one `Attribute` per name.
- [ ] Tests: one per trap in §3 — assert the losing span **survives** and is revealed by
      `removeAttribute`; assert `toString` output contains no scalar in the reserved
      block; assert different names coexist.

Acceptance: all three traps have a test that fails against the naive implementation.
Commit: —

### Phase 3 — descriptors, concatenation, surface

- [ ] The 8 typed descriptors, `text::plain`, `Color`, `AttrName`.
- [ ] `setAttribute`/`removeAttribute`/`clearAttributes`/`getAttributes`/`toString`/
      `toAttributeString`.
- [ ] The `&` operator overload with right-operand id remapping.
- [ ] Tests: `a & b` — visible text is the concat, both sides' spans preserved, relative
      precedence preserved, no id collision.

Acceptance: the full surface passes headless unit tests. No window, no widget, no seam.
Commit: —

## Validation Plan

- Tests: entirely headless unit tests. This unit has no runtime-proof obligation because
  it has no observable behavior beyond its own values — which is precisely why it can
  land before everything else.
- Coverage check: these are unit tests in the compiler's own suite and are in the
  denominator by construction. No golden dir needed.
- Runtime proof: none applicable and none claimed.
- Doc sync: a `text::` spec section and man pages for the new surface.
- Acceptance: the project's full suite.

## Open Decisions

1. **Whether `AttributeString` should be a distinct type or a `String` with a side
   table.** Recommended distinct (as designed) — a `String` that sometimes carries spans
   would make every existing `strings::` function's behavior conditional.
2. **Whether the reserved block should be 512 scalars or smaller.** Recommended keep 512;
   it bounds the number of *live span ids per value*, and a value with more than 512
   overlapping spans is pathological. Record the cap as an error, not a silent wrap.
3. **Whether `term::` should consume this immediately.** Recommended: not in this unit —
   it is listed as "independently valuable" and that is enough. An ANSI renderer is its
   own plan.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **Split out of plan-13-H, where it was Phase 1 under a false header
  dependency.** plan-13-H's header says "Depends on plan-13-C (the base `app::`
  shadow/`sync`/seam) being landed"; its own Phase 1 says this layer is "independently
  valuable" and "genuinely headless: `text::` is pure worker-side value code with no host
  seam". The body was right. This unit blocks on nothing.

## Summary

The engineering risk is subtle wrongness, not breakage. A mis-mapped visible position
bolds the wrong character; a `toString` that strips sentinels but not id carriers leaves
invisible scalars in every string that leaves the type; a same-name conflict resolved by
trimming makes `removeAttribute` reveal nothing. None of those crash, and none are caught
by a test that only checks the happy path — so each has a test written against the naive
implementation.

The structural point is that this never needed a GUI. It sat behind an x-large plan and a
false header dependency; on its own it is a medium unit that blocks on nothing and that
something other than a TextArea may well want first.

What is left untouched: `String`, every existing `text::`/`strings::` function, and every
existing program.
