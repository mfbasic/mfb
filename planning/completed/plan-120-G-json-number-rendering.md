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

- [x] Read the formatter's copy-out stage; decide mode-flag vs sibling
      symbol (record here); implement the ~~`p`-significant scientific
      rendering with digit-`p` rounding~~ **18-digit truncated stream** — the
      rounding moved out of assembly, see Correction G-C1.

      **Decision: sibling symbol**, `_mfb_rt_float_to_string_sci`, for three
      reasons the §2 reading anticipated and one it did not:

      1. The rounding bound is a different shape. The fixed path rounds after
         `prec` FRACTION digits from the limb remainder; significant-digit work
         rounds after `p` digits from the first non-zero, which for a large
         value falls *inside the integer digits* — a case the fixed path has no
         code for, since `prec >= 0` only ever rounds in the fraction.
      2. The buffers want opposite things. Reaching a subnormal's first
         significant digit means stepping over ~320 fraction zeros; the fixed
         path stores every fraction digit and caps at 255. Sharing the layout
         would mean growing its fraction buffer to ~350 bytes for digits it
         never emits.
      3. `x1` would silently change meaning from "fraction places" to
         "significant digits" on a symbol with existing callers.
      4. *(Not anticipated)* The fixed formatter's output is pinned byte-for-byte
         by goldens across five targets, so threading a mode through it puts a
         600-instruction function at risk for a feature that never runs there.

      The cost — two copies of the decompose-and-place preamble — is recorded in
      the module header rather than hidden.
- [x] ~~Unit-style rt fixture: sci renderings of the F corpus at fixed `p`
      values vs precomputed strings (incl. the `9.99…→1e+X` ripple case).~~ —
      **merged into the Phase 2 fixture; see Correction G-C2.** `json::sciParts`
      is `internal_only`, so no test program can call it, exactly as
      `strings::genCat` is reachable only through the predicates that use it.
      The one fixture covers both layers, with vectors chosen so a digit-stream
      fault is distinguishable from a placement fault: subnormals and the
      largest finite value exercise the stream, the four boundary exponents
      exercise placement, and `9.999999999999999e22` exercises the ripple.

      The digit stream was also probed directly during development, by
      temporarily un-hiding the member, and matched the Rust reference on the
      first run for all 16 vectors (`0.1` → sticky 1, `100000000000000005`,
      exp −1; `5e-324` → `494065645841246544`, exp −324).

Acceptance: ~~sci fixture green on macOS; `artifact-gate` 0 diffs (nothing
consumes the mode yet)~~; full cargo test green.

**MET, restated.** "Nothing consumes the mode yet" could not hold: the helper is
reachable only through `json::sciParts`, which the json package's own body calls,
so the two phases necessarily land together and the gate is Phase 2's below.

Commit: 62524eeb4 (the proven reference), 0f4ba0466 (the emitted helper)

### Phase 2 — json rendering + parity corpus

- [x] Rewrite `__json_stringifyNumber` per §3.2 (~~integer form~~ → sci search →
      ECMA placement; delete the 25-place loop and its reachable FAIL).

      The integer-form pre-check turned out to be unnecessary: it falls out of
      the placement rule's `n >= count` branch, which pads the digits out to the
      point, so `100` renders `100` and `9007199254740992` renders itself
      without a separate path. One fewer branch than the plan drew.

      The 25-place loop and its `FAIL` are gone. A `FAIL` remains as an
      unreachable invariant guard — 17 significant digits identify every
      binary64 — and `func_stringify.rs` keeps `ErrInvalidFormat` in its errors
      list for that reason, with the comment saying it can no longer fire.

      Three new MFBASIC helpers carry the work: `__json_roundDigits`
      (half-to-even with the sticky recomputed from the dropped digits),
      `__json_placeDigits` (the ECMAScript rules), and `__json_roundTrips` (see
      Correction G-C3).
- [x] Parity test: for the full F corpus + review S/X sets, expected strings
      captured verbatim from Node and asserted byte-equal; boundary
      exponents −7/−6/20/21 explicitly present.

      Landed as `tests/rt-behavior/json/json-number-rendering-rt`: **157
      vectors, 0 wrong**. Every expected string was produced by running
      `JSON.stringify(Number(literal))` on the same literal in Node v24.12.0 and
      pasted in, not derived here. All four boundary exponents are present from
      both sides (`1e20`/`1e21`, `1e-6`/`1e-7`), along with both exact ties,
      the ripple case, the largest finite value, and the two subnormals that
      used to fail outright.

      Beyond the fixture, the implementation was checked against Node over
      **2025 random doubles plus the shapes a random sweep misses**:
      `same=2025 different=0`.
- [x] Inspect + regenerate drifted goldens against the Node oracle;
      re-run the review probes end-to-end (S02→`1e+21`, S04→`5e-324`,
      S03→`1e-7`, S07→`1.7976931348623157e+308`).

      Drift was much smaller than "every fixture stringifying a non-integral
      number churns" predicted: `test-accept` reported **8 mismatches**, of
      which **7 were pure `.ir` line-number drift** from the json package gaining
      helpers. Exactly one fixture changed behaviour — `json-number-roundtrip-rt`
      — and it was repaired rather than re-baselined (Correction G-C4).

      All four review probes green end-to-end, and re-running plan-120-F's
      interop check with G in place turns its four RENDER-FAILs into renderings:
      **`same=30 different=0 render-fail=0`**, where before G it was
      `same=26 render-fail=4`.

      | probe | output |
      |---|---|
      | S02 | `1e21` → `1e+21` |
      | S03 | `1e-7` → `1e-7` |
      | S04 | `5e-324` → `5e-324` |
      | S07 | `1.7976931348623157e308` → `1.7976931348623157e+308` |
- [x] Doc sync: `func_stringify.rs` DESC number section (new rendering
      contract, Node-identical); `mod.rs` MODULE_DESC reviewed against the new
      reality.

      `func_stringify.rs` DESC now states the placement rule with its
      boundaries, the half-to-even tie rule, and that every finite `Float` has a
      rendering. MODULE_DESC's claim that a value "too small to render in 25
      decimal places (`1e-30`) parses but fails at `json::stringify`" was
      exactly the sentence this letter falsified, and is replaced by the
      Node-identical contract; the parse-side precision loss it also describes
      is untouched, because that part is still true.

      Added beyond the task: `mfb spec stdlib json`'s **Number formatting**
      section still described the 25-place search, so it was rewritten with the
      new algorithm and citations to the four files that implement it, and the
      error table's `ErrInvalidFormat` row corrected to say `stringify` retains
      the code only as an unreachable guard.

      Gates: `man-census.sh --memory-scope` 0 unclassified;
      `man-run-examples.sh json --run` 18/18; `spec_citations_resolve` and all
      26 `docs::` tests green.
- [x] Remote-box runs of the parity fixture (x86-64 + Windows) — new native
      formatter code needs cross-arch runtime proof. **All green, on every
      target rather than the two asked for:**

      | box | target | result |
      |---|---|---|
      | — | macos-aarch64 (host) | `checked=157 wrong=0` |
      | 2223 | linux-aarch64 glibc | 1 passed, 0 failed |
      | 2228 | linux-x86_64 glibc | 1 passed, 0 failed |
      | 2227 | linux-x86_64 musl | 1 passed, 0 failed |
      | 2229 | linux-riscv64 musl | 1 passed, 0 failed |
      | 2230 | windows-x86_64 | `checked=157 wrong=0` |

Acceptance: parity corpus byte-equal to Node on macOS + both remote arches;
no finite Float fails (fuzz a few thousand random bit patterns, skipping
non-finite, asserting stringify succeeds and `toFloat` round-trips);
full `cargo test --no-fail-fast` + `scripts/test-accept.sh` + regenerated
`artifact-gate.sh all`; fmt + check `--all-targets`.

**MET, every clause measured:**

| Gate | Result |
|---|---|
| parity corpus vs Node | `json-number-rendering-rt` **157 vectors, 0 wrong**, expectations captured from Node v24.12.0 |
| parity beyond the fixture | **same=2025, different=0** over random doubles plus the shapes a sweep misses |
| no finite Float fails | 2025 random bit patterns rendered and read back; plan-120-F's interop re-run is **same=30 different=0 render-fail=0**, where before G it was `same=26 render-fail=4` |
| macOS + remote arches | `checked=157 wrong=0` on **all six** targets (the table above) |
| `scripts/test-accept.sh` | **1351 test(s) ran, 0 mismatches** |
| `scripts/artifact-gate.sh all` | **1330 tests, 1493 build(s), 1834 golden(s), 0 diff(s)** |
| `cargo test --no-fail-fast` | **4453 passed.** One unrelated macOS TLS test timed out under load from a concurrent peer harness run and passes alone in 2.85s — its own failure message names that as the CPU-starvation signature and says to re-run it alone before calling it a regression |
| acceptance TESTING app | **758 / 758** |
| fmt + `cargo check --all-targets` | clean, no warnings |
| man / spec gates | `man-census --memory-scope` 0 unclassified; `man-run-examples json --run` 18/18; `spec_citations_resolve` + all 26 `docs::` green |

Commit: 62524eeb4, 0f4ba0466

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

**G-C1 — the rounding moved out of the native helper, and the split was proven
before it was relied on.** §3.1 puts "correctly rounded at digit `p`" in the
native code, and §3.2 has MFBASIC search `p = 1..=17`. Taken literally that is
one native call per candidate, with the trickiest arithmetic in the letter —
half-to-even plus an all-nines ripple that must carry into the exponent — living
in hand-written assembly across five architectures.

It is emitted differently. `_mfb_rt_float_to_string_sci` returns the first 18
significant digits **truncated**, the exponent, and a sticky flag; rounding, the
ripple, the search and the placement are MFBASIC string work. Rounding an
18-digit truncation at `p`, with the sticky recomputed from the digits being
dropped, is exactly rounding the true value at `p` — proven, not assumed, by
`the_two_factorings_agree`, which runs both formulations over 20,000 doubles plus
the curated shapes and requires identical output.

Two things fall out. One native call serves the whole search instead of
seventeen. And the part most likely to be wrong is now in a language where it can
be read, which matters more than it sounds: the native side worked on its first
run, while the MFBASIC side needed a fix (G-C3).

**G-C2 — Phase 1's fixture cannot exist as specified.** It asks for a fixture
over "sci renderings at fixed `p`", but the digit stream is reached through
`json::sciParts`, which is `internal_only` and therefore not callable from a test
program — the same position `strings::genCat` is in, and for the same reason: it
is an implementation detail with no meaning as public surface.

Making it public to test it would have been the tail wagging the dog. Phase 1 and
Phase 2's fixtures are merged instead, with vectors chosen so the two layers stay
distinguishable on failure: subnormals and the largest finite value exercise the
digit stream, the four boundary exponents exercise placement only, and the
all-nines vector exercises the ripple that joins them. The stream was
additionally probed directly during development by temporarily un-hiding the
member — 16 vectors, all matching the Rust reference first time.

**G-C3 — a short candidate can overflow the parser mid-search.** Not in the plan
and not in the reference, because it is a difference between MFBASIC and Rust
rather than an algorithm question: the first candidate for
`1.7976931348623157e308` is `2e+308`, and `toFloat` **raises** on overflow where
Rust's `parse` saturates to infinity and simply compares unequal. So the largest
finite double failed to serialize at all — traded one unserializable value for
another, which would have been an unpleasant thing to ship in the letter whose
whole point is that every finite Float renders.

The round-trip check is now `__json_roundTrips`, which traps: a candidate that
does not denote a Float does not round-trip, and the search moves on to a longer
one. Underflow needs nothing — a too-small candidate becomes zero and compares
unequal the ordinary way.

**G-C4 — golden drift was far smaller than predicted, and the one real change
was a test that measured the wrong thing.** §3 warns that "every fixture
stringifying a non-integral number churns". In fact `test-accept` reported 8
mismatches and **7 were pure `.ir` line-number drift** from the json package
gaining helpers.

The eighth, `json-number-roundtrip-rt`, was a genuine behaviour change:
`0.000000000000123` now renders `1.23e-13`, which is what Node prints. The
fixture asserted that `json::stringify(json::parse(text))` returns the **input
text**, and that is not bug-304's contract — bug-304 is about not losing
significant digits, and text identity merely happened to hold for the inputs
chosen. Re-baselining it would have preserved a check that no longer means
anything.

It now asserts **value** identity (`toFloat(out) = toFloat(text)`), which is
bug-304's actual guarantee and a stronger statement than text equality was, while
still printing the rendering so the golden pins the exact bytes. All twelve of
its vectors pass, including the one whose text changed.

**G-C5 — a test oracle that was wrong in exactly the way the letter is about.**
The reference's fuzz test first compared against Rust's `{:e}` formatting, on the
reasoning that both languages produce the shortest round-tripping decimal. They
do — but where two equally short forms both read back exactly, Rust picks the
half-away-from-zero one and ECMA-262 picks the **even** one.
`877566786661990.25` is `...990.3` in Rust and `...990.2` in Node.

The implementation was right and the oracle was wrong, which cost a debugging
round to establish. The fuzz test now asserts the two properties that actually
define the output — it reads back exactly, and nothing shorter does — and leaves
agreement with Node to the curated table and the 50,018-value on-demand sample.
Recorded because the plan's §3.1 box flagged this exact tie-break as the one
place the design could go wrong, and it turned out to be the one place the
*test harness* went wrong too.


*(fill during execution)*

## Summary

With F's correct parser underneath, this letter finishes number interop:
shortest digits via bounded search on the existing exact formatter, Node's
placement rules as the byte-level spec, and the "finite but unserializable"
class eliminated — leaving MFB↔Node number traffic bit-faithful in both
directions.
