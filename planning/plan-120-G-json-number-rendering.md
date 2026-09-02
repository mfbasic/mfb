# plan-120-G: ECMAScript-style number rendering in json::stringify

Last updated: 2026-09-02
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

  **Confirmed and localized by plan-120-A's Correction C1**: these values parse
  fine (`toFloat` accepts both) and fail at `helper_stringify_number.rs:49`,
  the FAIL after the 25-place search. The review filed them under I1 as a
  `toFloat` defect; they are this letter's, not plan-120-F's. That makes the
  "no finite Float fails to serialize" goal below the *only* fix for this class.

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

  **Node v24.12.0 is present on this machine (`/Users/justinzaun/local/bin/node`)
  and the oracle set was captured during plan-120-A's execution.** Verbatim
  `JSON.stringify(v)` output — these are the expected strings for Phase 2's
  parity test, no re-derivation needed:

  | Value | Node output |
  |---|---|
  | `1e21` | `1e+21` |
  | `1e20` | `100000000000000000000` |
  | `1e-6` | `0.000001` |
  | `1e-7` | `1e-7` |
  | `1e-21` | `1e-21` |
  | `1e-30` | `1e-30` |
  | `5e-324` | `5e-324` |
  | `1.7976931348623157e308` | `1.7976931348623157e+308` |
  | `-0` | `0` |
  | `100` | `100` |
  | `2.5` | `2.5` |
  | `3.141592653589793` | `3.141592653589793` |
  | `9.999999999999999e20` | `999999999999999900000` |
  | `1.0000000000000001e21` | `1.0000000000000001e+21` |
  | `123456789012345678901` | `123456789012345680000` |

  The boundary rows confirm §3.2's placement rule exactly as written: decimal
  while the decimal exponent is in `[-6, 20]`, exponential outside it. The two
  rows either side of the upper boundary are the load-bearing ones —
  `9.999999999999999e20` stays decimal (`999999999999999900000`, note the
  zero-padding to the right of the significant digits) while
  `1.0000000000000001e21` goes exponential. `123456789012345678901` shows the
  same padding behaviour arising from shortest-digits rather than from the
  input's own spelling.

  Also confirmed: `-0` renders `0`, which is plan-120-C's rule, so C and G
  agree at the boundary and neither needs a special case for the other.
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
- ~~UNMEASURED (Phase 1): the cleanest seam for "digits + exponent"~~ —
  **the copy-out stage was read during plan-120-A's execution
  (`float_format.rs:503-585`, the `assemble` block); findings recorded here so
  Phase 1 starts from them:**

  - Entry contract is `x0` = f64 bits (finite), `x1` = precision `0..=255`;
    the returned String is arena-allocated in the assemble block itself.
  - The digit STREAM and the placement are already separate stages. Integer
    digits are written *backward* ending at `DIGITS_INT_END` and fraction
    digits *forward* from that same boundary, so assemble is a straight
    two-run copy: sign, then `[ip, int_end)`, then `'.'` + `prec` bytes from
    `DIGITS_INT_END`. A scientific rendering is the same two runs with
    different cursors plus an `e±XX` tail — no new digit machinery.
  - The one thing a mode flag cannot reuse is WHERE rounding happens: the
    fixed path rounds after `p` FRACTION digits (the `e2 < 0` remainder test),
    while scientific must round after `p` SIGNIFICANT digits. That is a
    different loop bound, not a different algorithm.
  - Consequence for the Open Decision: a mode flag would have to change both
    the rounding bound and the copy-out, i.e. branch in two places, while
    `x1`'s meaning silently changes from "fraction places" to "significant
    digits". A **sibling symbol** keeps `_mfb_rt_float_to_string`'s contract
    exactly as documented and as its callers rely on. Phase 1 should confirm
    by trying the flag first only if the shared placement really dominates;
    the reading above says it does not.

## 3. Design Overview

1. **Native**: scientific rendering `d.dddd…e±XX` with `p` significant
   digits (1..=17), exact digits from the existing limb passes, correctly
   rounded at digit `p` (the same rounding logic the fixed path applies),
   exponent computed from the first-significant-digit position. Buffer
   needs are far below the existing 640 bytes.

   > **The tie-break is load-bearing and must be round-half-to-EVEN.** This was
   > validated ahead of implementation during plan-120-A's execution, by
   > simulating this whole algorithm (search `p = 1..=17`, then §3.2's
   > placement) against Node v24.12.0 over 199,918 pseudo-random doubles from
   > raw bit patterns, plus the curated set above:
   >
   > | Rounding at digit `p` | Mismatches vs Node |
   > |---|---|
   > | half-away-from-zero (what `toExponential` does) | **63 / 199,915** |
   > | **half-to-even** | **0 / 199,918** |
   >
   > A worked example of the failure mode — `v = 2188699164681338.2`, whose
   > exact value is `2188699164681338.25`, an exact tie at 17 significant
   > digits:
   >
   > ```
   > p=15 -> 2.18869916468134e+15    roundtrips=false
   > p=16 -> 2.188699164681338e+15   roundtrips=false
   > p=17 -> ...8382  (half-to-even)      roundtrips=true   <- Node agrees
   >         ...8383  (half-away-from-0)  roundtrips=true   <- Node disagrees
   > ```
   >
   > Both candidates round-trip, so the round-trip check cannot tell them
   > apart — only the tie-break can. Getting this wrong would put ~0.03% of
   > values silently out of step with Node, which is the exact class of defect
   > this family exists to remove.
   >
   > The good news: `float_format.rs`'s header already specifies
   > "exact half → ties-to-even on the last emitted digit (the rounding every
   > correct printf produces)", so the sci mode inherits the right behaviour by
   > reusing the fixed path's rounding rather than writing a new one. Do that,
   > and add a fixture for this exact vector.

   **The search itself is validated too**: with the correct tie-break, "first
   `p` whose rendering round-trips, then ECMAScript placement" reproduced
   `JSON.stringify` byte-for-byte on every one of the ~200k samples. So §3.2's
   bounded search really is sufficient — no Ryū/Grisu needed, as §Rejected
   argues.
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
(off-by-one at a `…999` ripple = wrong digits — covered by the corpus, and
note the ripple must carry into the EXPONENT as well as the digits, e.g.
`9.99…9eN` at `p` digits becoming `1e(N+1)`), and the placement rules' edge
exponents (−6, −7, 20, 21 boundaries — each gets a Node-verbatim test; the
oracle rows for all four are in References).

The pre-implementation simulation (§3.1 box) exercised both risks across ~200k
doubles and found the tie-break to be the only place the design can go wrong,
which narrows what the fixtures most need to cover. Golden drift: every fixture stringifying a
non-integral number churns; the delta must match the Node oracle
line-for-line before regeneration.

Byte-identity NOT a gate (numbers change shape by design); the gate is
Node-byte-equality on the corpus.

Rejected: Ryū/Grisu shortest-digit algorithms (new large algorithm; the
search-with-correct-parse achieves shortest with ≤17 bounded iterations on
already-proven machinery — **now measured rather than asserted: 0 mismatches
against Node over ~200k random doubles, see the §3.1 box**); emitting
Rust-formatted digits at compile time (runtime values exist); libc `%e`
(no-libc principle).

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
