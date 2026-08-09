# plan-89-D: Tier-B attribute-preserving transforms

Last updated: 2026-08-08
Effort: large (3h–1d)
Depends on: **plan-89-A** (the type + text slot) and **plan-89-B** (the attribute overlay + storage,
so transforms have spans to carry). Also consumes **plan-89-C Phase 1**'s frozen Tier-A/B partition.
If A or B is not complete, D cannot start, full stop.

Makes the string-returning `strings::` transforms accept an `AttributedString` and return an
`AttributedString`, transforming the text exactly as the `String` version does while carrying the
attribute spans through the same edit. This is the semantic core of the feature.

**Single behavioral outcome:** for every Tier-B function `t`, `toString(strings::t(a, …)) ==
strings::t(toString(a), …)` — the plain text of the attributed result equals the `String` transform of
the plain text — and the attribute spans are remapped to the correct new positions per the group
rules below.

References (read first): plan-89-A §2/§3; plan-89-B §3 (span storage + inclusive ranges);
`src/target/shared/code/builder_search.rs:653` `lower_mid`, `:72` `lower_find`;
`builder_strings_builtins.rs:2135` `lower_strings_left_right`; the C Phase-1 tier table.

## Prerequisites

See plan-89-A. plan-89-A and plan-89-B landed; plan-89-C Phase 1 tier table exists.

## 1. Goal

- Each Tier-B `strings::` function has an `AttributedString → AttributedString` overload holding the
  **text invariant** above.
- Attribute spans are remapped per position-mapping group (§3).
- Case/normalization transforms (`upper`/`lower`/`caseFold`/`normalizeNfc`) transform the text
  faithfully and **drop attributes** (documented), because they change scalar counts within a span.

### Non-goals (explicit constraints)

- **Never violate the text invariant.** The visible text of the result must equal the `String`
  transform of the input's text — no per-run re-transform that could diverge (e.g. context-sensitive
  case mapping).
- **No attribute *semantics* change** — spans move, they are not merged or reinterpreted; higher-
  start-wins is unchanged (resolved at read time by B/E).
- **`replace` inserts plain (unattributed) replacement text** in v1.
- **`String` overloads unchanged.**

## 2. Current State

Tier-B functions today take and return `String`, lowering via native-direct arms (`lower_mid`,
`lower_strings_left_right`, `replace`, `trim*`, `pad*`, `repeat`) that walk scalar indices to byte
offsets (`emit_ascii_scalar_fastforward` + `emit_scalar_skip_continuations`,
`private/unicode.rs:54/89`). The attribute overlay (B) stores inclusive-bound spans `(start, end,
attr, seq)` on the `AttributedString`.

### Measured populations

| What | Count | Command / basis |
|---|---|---|
| Tier-B candidates (from `mfb man strings`) | ~13 | left, right, mid, padLeft, padRight, repeat, replace, stripPrefix, stripSuffix, trim, trimChars, trimEnd, trimStart |
| Of which drop attributes (case/normalize) | 4 | upper, lower, caseFold, normalizeNfc |

> Exact membership = plan-89-C Phase 1's frozen table (authority). Count above is the man
> categorization, to be reconciled with that table before implementation.

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Case/normalize can change scalar count within a span | CONFIRMED | Unicode full case mapping (ß→SS) and NFC recomposition change scalar counts; a 1:1 span remap is impossible, so these drop |
| The text invariant is testable per function | CONFIRMED | assert `toString(t(a)) == strings::t(toString(a))` directly |

## 3. Design Overview — position-mapping groups

Every Tier-B transform = (1) run the existing `String` text transform on the visible text; (2) remap
the stored spans by the same edit. Group by how positions map (all inclusive bounds):

- **Slice / trim** (`mid`, `left`, `right`, `trim*`, `stripPrefix`, `stripSuffix`): the kept text is a
  contiguous scalar window `[w0, w1]`. Each span `[s,e]` → clip to the window, then shift by `−w0`;
  drop if empty.
- **Pad** (`padLeft`, `padRight`): inserts plain scalars at one end. `padLeft` shifts every span by
  `+padCount`; `padRight` leaves spans; pad scalars carry no attributes.
- **Repeat** (`repeat(a, n)`): replicate the text `n×`; replicate each span at each copy's offset.
- **Replace** (`replace(a, needle, repl)`): piecewise remap. Spans fully inside a match → dropped;
  spans straddling a match boundary → clipped to the surviving side; the inserted `repl` is **plain**;
  everything after each match shifts by the cumulative `(len(repl) − len(needle))`.
- **Case / normalize** (`upper`/`lower`/`caseFold`/`normalizeNfc`): transform text whole-string
  (invariant), **drop all spans**.

**Where correctness risk concentrates (schedule last):** `replace`'s cumulative piecewise remap and
the slice/trim clip-and-shift at inclusive bounds. Each group gets a test asserting both the text
invariant and the span positions via `getAttributes` at boundary scalars.

**Rejected alternative:** *per-run transform + reconcat for case/normalize to preserve attributes.*
Rejected — it can produce different text than `strings::upper` on the whole string (final-sigma and
other context-sensitive mappings), breaking the text invariant. Dropping attributes is honest.

## 4. Detailed Design

Each Tier-B overload: resolver returns `AttributedString` for an `AttributedString` first arg; codegen
(a) loads the text slot and runs the existing `String` transform to produce the new text, (b) builds
the new overlay by applying the group's span remap to the old overlay, (c) assembles a new
`AttributedString`. Reuse the scalar-walk helpers for computing match/window offsets. The four
case/normalize overloads skip (b) and emit an empty overlay.

## Compatibility / Format Impact

- **New:** `AttributedString → AttributedString` overloads of the Tier-B `strings::` functions.
- **Unchanged:** all `String` overloads; error codes; the text results (invariant).

## Phases

### Phase 1 — slice/trim + pad + repeat (well-defined maps)

- [x] Overloads + span remap for slice/trim (`mid`/`left`/`right`/`trim`/`trimStart`/`trimEnd`/
      `trimChars`/`stripPrefix`/`stripSuffix`), pad (`padLeft`/`padRight`), and `repeat`. Implemented
      as `__astrings_*` source-companion bodies: run the `String` transform on the visible text, then
      remap spans via `__astrings_windowSpans` (clip to kept window + shift to origin) / `shiftSpans`
      (padLeft) / per-copy replication (repeat). The window origin per function is computed from scalar
      counts of the `String` results (leading-trimmed for trim/trimChars).
- [x] Tests: `tests/rt-behavior/astrings/tier-b-transforms-rt/` — text invariant per function (`inv=`)
      + a per-scalar bold map proving span positions (`mid`→`BB---`, `left`→`BBB`, `trim`→`BB`,
      `trimStart`→`BB--`, `trimEnd`→`--BB`, `padLeft`→`---BBBBB------`, `repeat`→`BBBBB------BBBBB------`).

Acceptance: MET. Text invariant holds and spans land at the correct inclusive positions for every
function in this group.
Commit: 10e150d18

### Phase 2 — case/normalize (drop attributes)

- [x] Overloads for `upper`/`lower`/`caseFold`/`normalizeNfc` returning an `AttributedString` with the
      transformed text and an empty overlay (`fromString(strings::t(toString(a)))` — no span remap).
- [x] Tests (in `tier-b-transforms-rt`): text invariant incl. the scalar-count-changing `ß`→`SS`
      (`upperEszett=STRASSE inv=STRASSE bold=-------`) with the overlay empty afterward.

Acceptance: MET. Text matches `strings::upper(toString(a))` etc.; result has no attributes.
Commit: 10e150d18

### Phase 3 — `replace` (piecewise remap; correctness risk last)

- [x] `replace` overload with the cumulative piecewise span remap: find non-overlapping matches, walk
      the kept segments emitting each span clipped to the segment and re-based to the new origin
      (`__astrings_remapSegment`) — which yields drop-inside, clip/split-straddle, and shift-after
      uniformly; the inserted replacement is plain.
- [x] Tests: `tests/rt-behavior/astrings/tier-b-replace-rt/` — `"aXbXc"` bold-all, replace `X`→`YY`
      (multiple matches, length-changing): text invariant `aYYbYYc`, spans split around matches with
      plain inserts (`flags=B--B--B`); a span inside a match dropped + a span after matches shifted
      (`flags2=------I`).

Acceptance: MET. Every `replace` remap case passes and the text invariant holds with multiple matches
and a length-changing replacement.
Commit: 10e150d18

## Validation Plan

- Tests: rt-behavior per group asserting the text invariant AND span positions via `getAttributes`;
  the ß case for drop; multi-match `replace`.
- Coverage check: fixtures call each overload's codegen arm.
- Runtime proof: a fixture that styles text, transforms it, and prints `getAttributes` + `toString`.
- Doc sync: `strings::` man pages note `AttributedString` overloads + the case/normalize drop rule.
- Acceptance: `cargo test --bin mfb`; `artifact-gate.sh <exe> all`.

## Open Decisions

1. **`stripPrefix`/`stripSuffix` tier.** **Resolved: Tier-B (slice/trim group).** By the hard rule
   (C §4.1) a function whose result re-expresses the input's text returns `AttributedString`; strip*
   returns the text with a leading/trailing run removed (a contiguous window), so it modifies and
   stays in D. C's Phase-1 table must list them Tier-B to stay consistent.
2. **Concatenation.** If a string `&`/`concat` exists for `AttributedString`, its remap (keep both
   sides, shift the right operand) belongs in Phase 1; the map found no `strings::` `&` operator, so
   it is out unless C/B surface a `concat`. Recommended: out of v1 unless already present.
   Decision: AttributedString & AttributedString (both sides, no mixing)

## Corrections

- **Tier-B is `.mfb` source-companion bodies over B's bridge, not native-direct overloads (§4).**
  Each `strings::<t>(AttributedString, …)` routes to a `__astrings_<t>` companion body (dispatch: a
  `StringsResolver` return-type override yields `AttributedString`; an IR-lowering
  `implementation_name` split — `strings::tier_b_transform_impl` — targets the companion when arg0 is
  `AttributedString`). The body runs the existing `String` transform for the text (so the text
  invariant holds by construction) and remaps spans with `readSpans`/`writeSpans`. This keeps the
  risky inclusive-bound clip/shift and `replace` piecewise arithmetic in `.mfb`, consistent with B.
- **padLeft/padRight's optional `padChar`.** The companion bodies take a required `padChar`; the
  2-arg form is handled by (a) an `.mfb` default `padChar AS String = " "` and (b) an IR-lowering fill
  of `" "` for the 2-arg `AttributedString` call (so the routed companion always receives 3 args).
  The native `String` forms are untouched (they default `padChar` in codegen), so no `String` IR/
  golden churns.
- **Concatenation (Open Decision 2) implemented.** `AttributedString & AttributedString` types as
  `AttributedString` (frontend `infer_binary`), and the IR lowering rewrites the `Binary "&"` to a
  `__astrings_concat` companion call (text concatenated, right operand's spans shifted by the left's
  scalar length). `String & String` is unchanged.
- **The window origin for trim/trimChars** is computed from scalar counts of the `String` results
  (`leading = scalarCount(text) − scalarCount(trimStart(text))`; trimChars counts leading in-set
  scalars) — there is no `trimCharsStart`, so trimChars counts the leading run directly.

## Summary

The engineering risk is `replace`'s piecewise remap and inclusive-bound clip/shift arithmetic, fenced
by per-group tests on both the text invariant and span positions. Case/normalize honestly drop
attributes to preserve the text invariant. Untouched: `String` overloads and text results.
