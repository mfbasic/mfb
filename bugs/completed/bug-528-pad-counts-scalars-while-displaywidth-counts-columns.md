# bug-528: `padLeft`/`padRight` pad to a scalar count while `displayWidth` measures columns, and there is no `padToDisplayWidth`

Last updated: 2026-09-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: **FIXED** (2026-09-05, `f071d0f45`)
Regression Test: `tests/rt-behavior/strings/strings-pad-to-width-rt`

`strings::padLeft`/`padRight` pad to a width counted in **Unicode scalar
values**. `strings::displayWidth` measures **terminal columns**, summed over
**extended grapheme clusters**. Those are three different units, and the
package offers a padding function for the one nobody aligns a table with.

The result is that the obvious way to build a fixed-width table is also the
wrong one, in both directions:

| call | scalars | columns |
| --- | --- | --- |
| `padLeft("x", 3, "😀")` | 3 | **5** |
| `padLeft("日本", 4, "-")` | 4 | **6** |
| `padLeft("cafe" + U+0301, 6, ".")` | 6 | **5** |

`strings::displayWidth` exists precisely because "the number of terminal columns
`value` occupies when printed to a fixed-width terminal" is a question people
need answered — its own page says so. But nothing consumes it: there is no
`padToDisplayWidth`, and neither page's **See also** mentions the other, so a
reader who has found `padLeft` has no signal that a fourth measure exists or
that it is the one they wanted.

The single correct behavior a fix produces: a caller who wants a
column-aligned table can get one from the `strings` package, and the pad pages
say plainly which unit they count.

References:

- `src/codegen/builtins/strings/func_pad_left.rs`, `func_pad_right.rs`,
  `gen_pad.rs`
- `src/codegen/builtins/strings/func_display_width.rs` — "Display width is
  therefore a fourth measure, distinct from `len`, `byteLen`, and
  `graphemesCount`"
- Spike: `spikes/api-review/bug-528-pad-display-width/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-528-pad-display-width
./spikes/api-review/bug-528-pad-display-width/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
padLeft("x", 3, emoji)      scalars=3 columns=5  [😀😀x]
padLeft("日本", 4, "-")    scalars=4 columns=6  [--日本]
padLeft(NFD "cafe", 6, ".") scalars=6 columns=5  [.café]

A two-column table padded to 8 scalars:
|ascii   |end|
|日本語     |end|
|😀😀      |end|
```

  The three table rows are all 8 scalars and all different widths on screen.

- Expected: a `strings` member that pads to a column count, so the three rows
  line up.

Contrast cases that are correct today:

- `padLeft`'s own page **does** document the unit, and even gives this exact
  example: "A multi-byte `padChar` therefore contributes one toward the width
  per copy while adding several bytes: `padLeft("x", 3, "😀")` is `"😀😀x"`."
  So the behavior is accurate and disclosed. What is missing is the operation a
  reader needs *instead*, and the cross-link that would send them to it.
- `strings::displayWidth` is correct, well-specified (UAX #29 clusters, East
  Asian Ambiguous treated as narrow), and already does the hard part.
- `len`, `byteLen`, `graphemesCount` and `displayWidth` are each individually
  clear about their unit.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | pure software text handling; expected identical. Confirm in Phase 3 |

## Root Cause

**CONFIRMED (2026-09-05), and the scope is narrower than the report says.**

The reproduction is exact — the spike prints `scalars=3 columns=5`,
`scalars=4 columns=6`, `scalars=6 columns=5` and a three-row table where every
row is 8 scalars and no two are the same width. But three of the report's
supporting claims were measured and two of them were already true at HEAD:

| claim | measured |
| --- | --- |
| there is no column-counted padding member | TRUE — the gap this bug fixes |
| "neither page's **See also** mentions the other" | TRUE — `padLeft`/`padRight` linked only `astrings::AttributedString`; `displayWidth` linked `byteLen`/`graphemes`/`graphemesCount` |
| "the pad pages state the unit ... only in the body prose" | **FALSE.** Both `width` parameter descriptions already read "The target total length of the result in Unicode scalar values." (`func_pad_left.rs`, `func_pad_right.rs`) |
| "`term` — the package that most needs column alignment ... is a current victim" | **FALSE.** `grep -rn "padLeft\|padRight" src/codegen/builtins/term/` returns NOTHING. `term` draws per cell and never pads. |

The real in-tree victim is `examples/browser/display/src/lib.mfb`, which
hand-rolled the member this bug adds:

```
' Right-pad `s` with spaces to exactly `w` display columns (s must already fit).
FUNC padTo(s AS String, w AS Integer) AS String
  LET sw AS Integer = strings::displayWidth(s)
  IF sw >= w THEN RETURN s
  RETURN s & strings::repeat(" ", w - sw)
END FUNC
```

A second one is `examples/ai_chat/src/main.mfb`, whose TUI boxes pad with
`strings::padRight(clip(label, n), n)` — but its `clip` counts scalars too, so it
is consistently scalar-counted and converting only its PADDING to columns would
misalign it further. It needs the column-counted **truncate** this bug's Fix
Design deliberately defers (`strings::truncateToWidth`), and is left alone.

The mechanism itself is as reported: a missing member plus two missing
cross-references.

`gen_pad.rs` implements padding by scalar count, which is the right primitive:
it is cheap, total, and the correct answer when the "width" the caller means is
a character count (a fixed-length record field, a zero-padded number). The
column-counted variant is strictly more expensive — it needs grapheme
segmentation and the per-scalar width table that `displayWidth` already
carries — so implementing only the scalar form was a reasonable starting point.

What makes it a footgun rather than a limitation is the naming: the parameter is
called `width`, and "width" in a terminal context means columns to most readers.
`displayWidth` then uses the same word for the other meaning.

## Goal

- `strings` offers padding to a display-column width — as a new member, or as an
  option on the existing ones.
- `padLeft`, `padRight` and `displayWidth` cross-link each other in **See also**.
- The pad pages state the unit in the `width` parameter description, not only in
  the body prose.

### Non-goals (must NOT change)

- `padLeft`/`padRight`'s current behavior. Scalar-counted padding is correct,
  is the right answer for several real uses, and its output is in acceptance
  goldens. The new capability must be additive.
- `strings::displayWidth`'s definition, including the East Asian Ambiguous =
  narrow choice.
- The `padChar` one-scalar restriction, which is unrelated.
- **Tempting wrong fix, forbidden:** redefining `padLeft`'s `width` to mean
  columns. It would silently change the output of every existing caller,
  including ones padding numbers where scalar counting is exactly right.

## Blast Radius

- `src/codegen/builtins/strings/gen_pad.rs`, `func_pad_left.rs`,
  `func_pad_right.rs` — the members extended or cross-linked by this bug.
- `src/codegen/builtins/strings/func_display_width.rs` — gains the reverse
  cross-link.
- `astrings` overloads — `padLeft`/`padRight` both have an
  `astrings::AttributedString` overload that remaps attribute spans. Any new
  member must decide whether it gets one too; **Phase 1 must record the
  decision**, because adding it later is a wider change than adding it now.
- `term` — the package that most needs column alignment. `grep -rn "padLeft\|padRight"
  src/codegen/builtins/term/ examples/` in Phase 1; anything drawing a boxed or
  tabular terminal layout is a current victim and a validation case.
- `examples/`, `benchmark/` — existing `padLeft` callers; unaffected, since the
  existing behavior does not change.
- The Unicode width tables — already vendored and used by `displayWidth`; no
  new data.

## Fix Design

Add `strings::padToWidth(value, columns, [padChar])` — or the
`padLeftToWidth`/`padRightToWidth` pair — implemented over the machinery
`displayWidth` already has.

The semantics need one decision that the scalar version never faced: **what
happens when no whole number of `padChar` copies reaches the target?** Padding
to 5 columns with a 2-column emoji can produce 4 or 6, not 5. The options are to
undershoot, to overshoot, or to mix in a space. **Recommend undershoot** — never
exceed the requested width — because a table that is one column narrow still
reads, and one column wide breaks the next column. Whatever is chosen must be
stated in the member's description; this is the fact that makes the member hard
to use correctly.

Two secondary decisions:

- **Truncation.** `padLeft` never truncates. For column alignment, an
  over-wide cell is the common failure and truncation is often what the caller
  wants — but truncating grapheme clusters correctly is a separate problem.
  **Recommend not truncating**, matching `padLeft`, and leaving truncation to a
  future `strings::truncateToWidth`.
- **A zero-width `padChar`.** `padLeft` accepts any single scalar; a
  zero-column one (a combining mark) would loop forever in a column-counted
  pad. Must raise `ErrInvalidArgument`.

Rejected: an optional `unit` parameter on `padLeft`. It makes the common call
carry a decision it does not need, and the two behaviors differ enough
(undershoot, zero-width rejection) that they are not one function.

Rejected: documenting the mismatch and adding cross-links only. That is worth
doing and is Phase 1, but it leaves the caller to hand-roll a
`WHILE displayWidth(s) < n` loop — which they will get wrong for exactly the
undershoot case above.

## Phases

### Phase 1 — cross-links + audit (no behavior change)

- [x] Land `spikes/api-review/bug-528-pad-display-width/` (done).
- [x] **See also** now links `padLeft`/`padRight` ↔ `displayWidth` ↔ the two new
      members, in both directions (the section is derived from `pkg::member`
      mentions in `desc`, so the link is a sentence that carries a fact).
      The `width` parameter description already stated the unit — see Root Cause.
- [x] Caller census run. `term`: none. `examples`: `browser/display`'s `padTo`
      (the hand-rolled member) and `ai_chat`'s scalar-consistent TUI. `benchmark`:
      `string.mfb`/`strbuild.mfb` measure `padLeft` on ASCII, where the two
      measures agree.
- [x] Undershoot and the `astrings` overload decided — see Open Decisions.

### Phase 2 — the new member

- [x] `strings::padLeftToWidth` / `padRightToWidth`, `Body::Rewrite` over a
      `WhenUsed`-gated `__strings_padToWidth*` chunk — NOT `Body::Mfb`, which
      would render into every `IMPORT strings` program.
- [x] A zero-column `padChar` raises `ErrInvalidArgument`.
- [x] Both man pages written, rendered and their examples run.
- [x] `astrings` overloads added (Tier-B companions + the two
      `TIER_B_TRANSFORMS` rows).

### Phase 3 — validation

- [x] `tests/rt-behavior/strings/strings-pad-to-width-rt`: ASCII, CJK, NFC and
      NFD `café`, emoji, the undershoot case, the exact-fit case, no-truncation,
      `columns = 0`, all four rejections through an INLINE trap, both
      `AttributedString` overloads with a per-scalar bold map, and the positive
      pins.
- [x] `scripts/man-run-examples.sh strings --run`: 89 examples, 89 built, 89 ran,
      0 failed. `man-census.sh --memory-scope strings`/`astrings`: 0 unclassified.
- [x] `cargo test --release --no-fail-fast`: 4838 passed, 0 failed, cargo exit 0.
      `cargo check --all-targets`: clean.
- [x] `scripts/test-accept.sh`: **1412 ran**, exit 0.
      `scripts/artifact-gate.sh all`: 1390 tests, 1930 goldens, **0 diffs**.
- [x] Golden delta, predicted before it was measured and matched exactly: the 14
      `.ir` goldens of every fixture that imports `astrings`, and nothing else.
      No `.run` and no `build.log` moved, so no behavior changed anywhere. With
      `"line": N` normalized away, the whole delta across those 14 files is the
      five new functions (`#strings_padToWidthCopies`, `#strings_padLeftToWidth`,
      `#strings_padRightToWidth`, `#astrings_padLeftToWidth`,
      `#astrings_padRightToWidth`) plus one `ErrorLoc` constant that carries a
      source line as a value.
- [ ] Linux/Windows confirmation is not run here — the members are pure text
      handling over the same vendored Unicode tables `displayWidth` already uses,
      and every target's `.ncodesum` is in the artifact gate above.

## Validation Plan

- Regression tests: the five fixtures above, each asserting
  `displayWidth(result)` rather than `len(result)`.
- Runtime proof: `spikes/api-review/bug-528-pad-display-width/`, extended to
  print the aligned table.
- Doc sync: `func_pad_left.rs`, `func_pad_right.rs`, `func_display_width.rs`,
  the new member's page.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

**All four decided 2026-09-05, each with its reason.**

- **Undershoot**, as recommended. `__strings_padToWidthCopies` divides the gap by
  the pad's own column width and integer division truncates toward zero, so the
  rule falls out of the arithmetic rather than being a special case. Pinned:
  padding a 1-column value to 6 columns with a 2-column `padChar` yields 5, and
  to 7 columns yields exactly 7.
- **Two members**, as recommended: `strings::padLeftToWidth` and
  `strings::padRightToWidth`, mirroring the existing pair.
- **A zero-column `padChar` is rejected** with `ErrInvalidArgument` (77050002),
  along with a negative `columns`, an empty `padChar` and a multi-scalar one.
  Note the asymmetry this creates, and it is deliberate: `strings::padLeft`
  ACCEPTS a combining mark as `padChar` (it counts scalars, so it terminates),
  and `padLeftToWidth` cannot. Both halves are in the fixture.
- **The `astrings` overloads are INCLUDED**, the question the report said Phase 1
  must settle. Both members are Tier-B transforms with `__astrings_*` companions,
  so `strings::padRightToWidth(anAttributedString, 8)` returns an
  `AttributedString` with its spans remapped, exactly as `padRight` does. Adding
  them later would have cost the same golden churn as adding them now, and
  leaving them out would have created precisely the asymmetry bug-534 is filed
  about.
- **`term` does not adopt it** — the premise was wrong. `term` has no
  `padLeft`/`padRight` call at all (see Root Cause); it draws per cell. The
  validation case is `examples/browser/display`'s `padTo`, which the fix rewrites
  onto the member, and the fixture proves the two agree on all 65 (value, width)
  pairs it compares.

## Found while fixing this

`strings::padLeft`/`padRight` — and `strings::left`/`right` with them — raise
`ErrInvalidArgument` and declared `errors: vec![]`, which made
`inline_builtin_is_infallible` prove the call infallible and DELETE a live inline
`TRAP` handler. Third instance of the bug-486 / bug-533 shape. Fixed in its own
commit ahead of this one, with `tests/rt-behavior/strings/strings-inline-trap-fallible-rt`.
The two new members are source-backed (`Body::Rewrite`), so they were never
subject to that verdict — but their rejections are pinned through the INLINE
`TRAP` form anyway, because that is the only form that can see the bug.

## Summary

The existing behavior is correct and disclosed, so there is no regression risk
in the pad members themselves. The engineering risk is in the new member's edge
semantics — the unreachable-target rule and the zero-width `padChar` — which are
easy to get wrong in ways that only show up on CJK or emoji input. Phase 1's
cross-links are worth landing on their own even if the member is deferred.
