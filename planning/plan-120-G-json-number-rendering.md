# plan-120-G: ECMAScript-style number rendering in json::stringify

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-120-F (the shortest-search verifies round trips through `toFloat`; with the old 1-ULP `toFloat` this letter would enshrine wrong digits)

Give `json::stringify` Node-compatible number rendering (review I7) and make
every finite Float serializable (the S04/S05 failures). Today
`__json_stringifyNumber` (`helper_stringify_number.rs`) searches fixed-point
renderings only (`toString(value, places)`, places ≤ 25), so:

- `1e21` emits `1000000000000000000000` and the max double emits **309
  digits** (Node: `1e+21`, `1.7976931348623157e+308`);
- `1e-7` emits a 25-digit fixed expansion (Node: `1e-7`);
- `5e-324` and `1e-30` **fail with an error** — a finite number that cannot
  be serialized at all (needs up to 1074 fixed-point fraction digits; the
  native formatter caps at 255).

Fix: add an exact scientific mode to the native formatter and render
numbers with ECMAScript's Number-to-String placement rules (decimal for
`1e-6 ≤ |x| < 1e21`, exponential outside), which makes MFB and Node
byte-identical on numbers.

References:

- `src/codegen/string/format/float_format.rs` — the exact `%.*f` formatter
  (limb-based, no libc). The scientific mode is the same digit stream with
  different placement: `%.*e`-equivalent needs the digits starting at the
  first significant digit plus the decimal exponent — the limb machinery
  already produces the digit stream; the new mode changes where the point
  and exponent go.
- `helper_stringify_number.rs` — the shortest-search loop this letter
  re-targets (search significant digits 1..=17 in scientific space instead
  of fractional places 1..=25).
- ECMAScript `Number::toString` placement rules (the spec algorithm Node
  implements) — the executable oracle is Node itself; expected strings are
  captured verbatim into the tests.
- Review probes S01–S08, X01–X05 — the before/after matrix.

## Prerequisites

Family gate in plan-120-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-120-F landed (correct `toFloat`) | the F corpus test exists and is green | NOT MET |
| plan-120-C landed (`-0` rule) | `ls planning/plan-120-C*` archived | NOT MET |

## 1. Goal

- For every value in the F corpus plus the review set, `json::stringify`
  emits **byte-identical** output to `JSON.stringify` in Node — including
  `1e+21`, `1e-7`, `5e-324`, `1e-30`, `1.7976931348623157e+308` — and no
  finite Float fails to serialize.

### Non-goals (explicit constraints)

- `toString(Float[, places])`'s public behavior is unchanged — the
  scientific mode is a new internal entry (a mode flag on the existing
  helper or a sibling symbol), not a new user-facing `toString` form.
- Integral values in the decimal range keep emitting integer form (`100`,
  not `100.0`) — already Node-identical.
- Non-finite handling stays as plan-120-A coded it (ErrFloatNaN/Inf).
- Parsing is untouched (scientific INPUT already parses).

## 2. Current State

- Formatter: exact fixed-point only, precision ≤ 255 fraction digits,
  640-byte digit buffer (`float_format.rs:38-48`); "no inf/NaN path" — the
  scientific mode inherits that invariant.
- The search loop caps at 25 places with a FAIL fallback
  (`helper_stringify_number.rs:36-49`).
- UNMEASURED (Phase 1): the cleanest seam for "digits + exponent" — either
  a mode flag (`x2` = style) on `_mfb_rt_float_to_string` or a sibling
  `_mfb_rt_float_to_sci`; and whether MFBASIC-side assembly of the final
  string (given a sci rendering) is simpler than emitting placement
  natively. Decide by reading the formatter's copy-out stage.

## 3. Design Overview

1. **Native**: scientific rendering `d.dddd…e±XX` with `p` significant
   digits (1..=17), exact digits from the existing limb passes, correctly
   rounded at digit `p` (the same rounding logic the fixed path applies),
   exponent computed from the first-significant-digit position. Buffer
   needs are far below the existing 640 bytes.
2. **json (MFBASIC)**: `__json_stringifyNumber` becomes: try integer form
   (unchanged); else search `p = 1..=17` scientific renderings for the
   shortest whose `toFloat` (now correct, per F) round-trips — 17 always
   suffices for binary64, so the FAIL fallback becomes unreachable and is
   replaced by an invariant failure; then apply ECMAScript placement:
   exponent in `[-6, 20]` → expand to plain decimal (shift the point /
   pad zeros — pure string work in MFBASIC), else keep exponential with
   Node's exact spelling (`e+21`, `e-7`, no zero-padding of the exponent).
3. `-0` keeps plan-120-C's `0` rule (checked before the search).

**Risk** concentrates in two places: the native rounding at digit `p`
(off-by-one at a `…999` ripple = wrong digits — covered by the corpus), and
the placement rules' edge exponents (−6, −7, 20, 21 boundaries — each gets
a Node-verbatim test). Golden drift: every fixture stringifying a
non-integral number churns; the delta must match the Node oracle
line-for-line before regeneration.

Byte-identity NOT a gate (numbers change shape by design); the gate is
Node-byte-equality on the corpus.

Rejected: Ryū/Grisu shortest-digit algorithms (new large algorithm; the
search-with-correct-parse achieves shortest with ≤17 bounded iterations on
already-proven machinery); emitting Rust-formatted digits at compile time
(runtime values exist); libc `%e` (no-libc principle).

## Phases

### Phase 1 — native scientific mode

- [ ] Read the formatter's copy-out stage; decide mode-flag vs sibling
      symbol (record here); implement the `p`-significant scientific
      rendering with digit-`p` rounding.
- [ ] Unit-style rt fixture: sci renderings of the F corpus at fixed `p`
      values vs precomputed strings (incl. the `9.99…→1e+X` ripple case).

Acceptance: sci fixture green on macOS; `artifact-gate` 0 diffs (nothing
consumes the mode yet); full cargo test green.
Commit: —

### Phase 2 — json rendering + parity corpus

- [ ] Rewrite `__json_stringifyNumber` per §3.2 (integer form → sci search →
      ECMA placement; delete the 25-place loop and its reachable FAIL).
- [ ] Parity test: for the full F corpus + review S/X sets, expected strings
      captured verbatim from Node and asserted byte-equal; boundary
      exponents −7/−6/20/21 explicitly present.
- [ ] Inspect + regenerate drifted goldens against the Node oracle;
      re-run the review probes end-to-end (S02→`1e+21`, S04→`5e-324`,
      S03→`1e-7`, S07→`1.7976931348623157e+308`).
- [ ] Doc sync: `func_stringify.rs` DESC number section (new rendering
      contract, Node-identical); `planning/speed.md`-style closing note in
      the review context is NOT needed — but `mod.rs` MODULE_DESC's
      "very large or very precise values may lose precision" phrasing gets
      reviewed against the new reality (precision loss is parse-side only
      now).
- [ ] Remote-box runs of the parity fixture (x86-64 + Windows) — new native
      formatter code needs cross-arch runtime proof.

Acceptance: parity corpus byte-equal to Node on macOS + both remote arches;
no finite Float fails (fuzz a few thousand random bit patterns, skipping
non-finite, asserting stringify succeeds and `toFloat` round-trips);
full `cargo test --no-fail-fast` + `scripts/test-accept.sh` + regenerated
`artifact-gate.sh all`; fmt + check `--all-targets`.
Commit: —

## Validation Plan

- Tests: sci-mode fixture, Node-parity corpus, the random-bit-pattern fuzz
  (bounded, seeded — no `Date.now` class flakiness).
- Runtime proof: remote-box parity runs; the re-run review probe transcript.
- Doc sync: Phase 2 list.
- Acceptance: family standard.

## Open Decisions

- Mode flag vs sibling helper symbol — decided by Phase 1's read of the
  copy-out stage (flag recommended if the placement share is high).

## Corrections

*(fill during execution)*

## Summary

With F's correct parser underneath, this letter finishes number interop:
shortest digits via bounded search on the existing exact formatter, Node's
placement rules as the byte-level spec, and the "finite but unserializable"
class eliminated — leaving MFB↔Node number traffic bit-faithful in both
directions.
