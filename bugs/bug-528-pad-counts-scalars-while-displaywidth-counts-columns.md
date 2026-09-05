# bug-528: `padLeft`/`padRight` pad to a scalar count while `displayWidth` measures columns, and there is no `padToDisplayWidth`

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `tests/` — new `rt_strings_pad_display_width` fixture (Phase 1)

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

Not a defect — a missing member plus two missing cross-references.

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

- [ ] Land `spikes/api-review/bug-528-pad-display-width/` (done).
- [ ] Add the **See also** cross-links between `padLeft`/`padRight` and
      `displayWidth`, and state the unit in the `width` parameter description.
      This is the cheap half and is worth landing alone.
- [ ] `grep -rn "padLeft\|padRight" src/codegen/builtins/term/ examples/ benchmark/`
      — list the callers that are actually building terminal layouts.
- [ ] Decide the undershoot/overshoot rule and the `astrings` overload
      question; write both into this file.

Acceptance: the cross-links render; the caller list is written down; both
decisions are recorded.
Commit: —

### Phase 2 — the new member

- [ ] Implement column-counted padding over the `displayWidth` machinery.
- [ ] Reject a zero-column `padChar` with `ErrInvalidArgument`.
- [ ] Write the man page, including the undershoot rule and a worked example
      with CJK, emoji and a combining sequence.
- [ ] Add the `astrings` overload if Phase 1 decided in favour.

Acceptance: the spike's three-row table lines up when built with the new member.
Commit: —

### Phase 3 — validation

- [ ] Add fixtures: ASCII, CJK, emoji, NFD combining, and the undershoot case.
- [ ] Regenerate `.ncodesum` goldens; `cargo test --no-fail-fast`;
      `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh strings --run`.
- [ ] Confirm identical output on Linux and Windows.

Acceptance: full suite green; the new member's output is byte-identical across
platforms; existing `padLeft` output is unchanged.
Commit: —

## Validation Plan

- Regression tests: the five fixtures above, each asserting
  `displayWidth(result)` rather than `len(result)`.
- Runtime proof: `spikes/api-review/bug-528-pad-display-width/`, extended to
  print the aligned table.
- Doc sync: `func_pad_left.rs`, `func_pad_right.rs`, `func_display_width.rs`,
  the new member's page.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Undershoot vs. overshoot when the target is unreachable. **Recommend
  undershoot.**
- One member (`padToWidth` with a side argument) vs. two
  (`padLeftToWidth`/`padRightToWidth`). **Recommend two**, mirroring the
  existing pair — a side argument on a padding function is one more thing to
  get backwards.
- Whether `term` should adopt it. **Recommend yes** as the validation case: if
  the new member does not make `term`'s layouts correct, it is the wrong member.

## Summary

The existing behavior is correct and disclosed, so there is no regression risk
in the pad members themselves. The engineering risk is in the new member's edge
semantics — the unreachable-target rule and the zero-width `padChar` — which are
easy to get wrong in ways that only show up on CJK or emoji input. Phase 1's
cross-links are worth landing on their own even if the member is deferred.
