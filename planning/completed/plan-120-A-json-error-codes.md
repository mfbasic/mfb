# plan-120-A: json — specific error codes and truthful number docs

Last updated: 2026-09-02
Overall Effort: huge (>3d — the whole plan-120 family, letters A–G)
Effort: medium (1h–2h)
Depends on: nothing

plan-120 fixes the findings of the 2026-09-01 json-vs-Node review (paired
probe programs, `/tmp/jsoncmp` + node twins; all numbers below reproduced
there) — everything except M2 (replacer), M4 (native-value bridge), and M5
(rawJSON), which are deferred by decision. The letters, in implementation
order:

- **A (this doc)** — specific error codes for I4 (lone surrogates), I5 (depth
  cap), M6 (non-finite stringify), and stop swallowing `toFloat`'s real code
  (I1/I2 reporting); fix the man text that contradicts behavior.
- **B** — `json::get`/`getOr` array-index path steps.
- **C** — stringify byte-shape: stop escaping `/` (I6), emit `-0` as `0`
  (I8), specify object order (I9).
- **D** — pretty-printing overload (M3).
- **E** — `json::parse` reviver overload (M1).
- **F** — correctly-rounded `toFloat(String)` (I3 root cause; unblocks I1).
- **G** — exact scientific formatter mode + ECMAScript-style number rendering
  in `json::stringify` (I7; fixes the "cannot serialize numbers below
  ~1e-25" failures).

**This letter changes which errors are raised, not what succeeds.**

**Dependency graph** (read off each letter's `Depends on:` line and verified
during execution — the family runs in topological order, and the letters are
identifiers, not a sequence):

```
A ──> B ──> C ──> D ──> E
│           │
│           └────────┐
└──> F ──────────> G ┘
```

Only four edges are real design dependencies; the rest are family ordering:

| Edge | Kind | Why |
|---|---|---|
| C → D | **design** | D wraps the compact renderer, so C's byte shape must be final first |
| A → F | **design** | F changes what converts; A's propagation defines what json then reports |
| F → G | **design** | G's shortest-digit search verifies round trips through `toFloat` |
| C → G | **design** | G inherits C's `-0` → `0` rule (G's own Prerequisites row) |
| A → B, B → C, D → E | ordering only | stated as "family order only" in each letter |

So A, B, C, D, E, F, G — the order §Introduction lists — is a valid topological
order, and it is the one being followed. A session with more parallelism
available could run F immediately after A, concurrently with B/C/D/E, and join
before G.

References:

- Review evidence: MFB rejects `"\uD800"`, 300-deep arrays, and non-finite
  stringify all with the one generic `77050003 ErrInvalidFormat`;
  `__json_toNumber` (`src/codegen/builtins/json/helper_to_number.rs:11-16`)
  TRAPs `toFloat` and re-fails with 77050003, discarding the real code.
- Error-code registry: `src/codegen/builtins/errorcode/mod.rs` (49 constants;
  general family `7705xxxx` allocated through `77050023 ErrBadImageFile`) —
  and the spec table `src/docs/spec/diagnostics/02_error-codes.md`, which per
  AGENTS.md must be updated in the same change.
- Existing codes to REUSE: `ErrFloatNaN 77050013`, `ErrFloatInf 77050014`,
  `ErrOverflow 77050010` (already what `toFloat` raises on overflow,
  `builder_conversions.rs:905`).
- The false doc claim: `func_parse.rs:45-53` says magnitude beyond binary64
  is "approximated at parse time rather than rejected" — measured behavior:
  `1e400`, `1e-30`, `5e-324` are all REJECTED (spike P01/X03/X04).
- Acceptance suite: `tests/acceptance/src/json.mfb` (asserts 77050003 today
  for these paths — update the specific cases, per the never-weaken rules the
  expected CODES change, the expected failures do not).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Codes 77050024/25 unallocated | `grep -c "7705002[45]" src/codegen/builtins/errorcode/mod.rs` → 0 | **MET** (re-measured 2026-09-02 at execution: 0 hits; highest allocated general code is `77050023 ErrBadImageFile`) |

## 1. Goal

- `json::parse` of a lone/unpaired surrogate escape fails with a dedicated
  surrogate code; a >256-deep document fails with a dedicated depth code;
  `json::stringify` of a non-finite number fails with `ErrFloatNaN`/
  `ErrFloatInf`; a number `toFloat` cannot represent propagates `toFloat`'s
  own code (`ErrOverflow`, …) instead of 77050003; and `mfb man json parse`
  states what actually happens.

### Non-goals (explicit constraints)

- No accept/reject behavior changes: everything that parsed still parses,
  everything rejected is still rejected — only codes and docs move.
- Depth limit stays 256; lone surrogates stay rejected (MFB strings are
  UTF-8); non-finite stringify stays failing. (Their Node divergences become
  *documented, coded* behavior — that is this letter's whole point.)
- Malformed-but-ordinary JSON (bad grammar, unknown escapes, trailing text)
  keeps `77050003 ErrInvalidFormat`.

## 2. Current State

- One code for everything: parse and stringify raise `77050003` from ~20
  `FAIL error(77050003, "invalid JSON format")` sites across the json
  helpers (`grep -rn 77050003 src/codegen/builtins/json/ | wc -l` → census
  at execution; UNMEASURED exactly, the surrogate sites live in
  `helper_parse_unicode_escape.rs`/`helper_decode_escape.rs`, the depth site
  in `helper_depth_limit.rs` consumers, the non-finite site in
  `helper_stringify_number.rs`/`helper_is_invalid_number_text.rs`).
- `__json_toNumber` swallows: `helper_to_number.rs:13-15`.
- errorCode allocation scheme: flat global constants, `7705` = general
  family, next free `77050024`.

## 3. Design Overview

Two new constants + two reuses + one un-swallow:

| Finding | Code | New? |
|---|---|---|
| I5 depth > 256 | `ErrDepthExceeded 77050024` — "Structural nesting exceeds the implementation depth limit." | new |
| I4 lone/unpaired surrogate escape | `ErrInvalidSurrogate 77050025` — "A `\u` escape encodes an unpaired surrogate; strings are Unicode text." | new |
| M6 non-finite stringify | reuse `ErrFloatNaN 77050013` / `ErrFloatInf 77050014` | no |
| I1/I2 number out of Float range | propagate `toFloat`'s own error (delete the TRAP re-wrap in `__json_toNumber`, or re-fail with `err.code`) | no |

New constants are generic (not json-prefixed) deliberately — regex's nesting
cap and future parsers can share `ErrDepthExceeded`; naming is an Open
Decision row. Both land in `errorcode/mod.rs` AND the spec table
(`02_error-codes.md`) in the same commit.

Risk: near zero (code plumbing + docs). Byte-identity is NOT a gate — the
error-message data objects change, which churns goldens for json-using
fixtures (the "new standard error message churns goldens" lesson); regenerate
and prove the delta is confined to json-importing fixtures.

Rejected: json-private codes per finding site (the registry is global by
design); mapping I2 to Node's `Infinity` (MFB traps non-finite floats at
every observation boundary — `float_format.rs:4-5` — so parse must not mint
one; the Node divergence is documented instead).

## Phases

### Phase 1 — codes

- [x] Census the `77050003` FAIL sites in `src/codegen/builtins/json/`
      (command in §2); classify each as surrogate / depth / non-finite /
      number-range / genuinely-malformed; record the table here.

      `grep -rn "77050003" src/codegen/builtins/json/ | wc -l` → **35** (34 `FAIL`
      sites + 1 prose mention in `func_parse.rs:62`). Classification:

      | Class | Sites | Count | Re-pointed to |
      |---|---|---|---|
      | surrogate | `helper_parse_unicode_escape.rs:15,18,21,25,31` | 5 | `77050025` |
      | depth | `helper_parse_value.rs:21` | 1 | `77050024` |
      | non-finite stringify | `helper_stringify_number.rs:26,39` | 2 | `77050013`/`77050014` |
      | number-range | `helper_to_number.rs:14` | 1 | propagates `err.code` |
      | genuinely malformed | the other 25 `FAIL` sites | 25 | unchanged `77050003` |
      | prose | `func_parse.rs:62` | 1 | rewritten in Phase 2 |

      The two `helper_code_point_to_string.rs` guards (`:17`,`:20`) stay
      `77050003`: they catch a code point outside `0`–`1114111`, which is a
      malformed `\u` escape and not a surrogate. `helper_stringify_number.rs:49`
      (no rendering round-trips) also stays `77050003` — see Corrections C3.
- [x] `errorcode/mod.rs`: add `ErrDepthExceeded 77050024`,
      `ErrInvalidSurrogate 77050025`; spec table row for each in
      `src/docs/spec/diagnostics/02_error-codes.md`.
      Verified rendered: `mfb spec diagnostics error-codes | grep 7705002[45]`
      shows both rows; `table_matches_registry` (the drift guard, which asserts
      the spec table reproduces the registry *in registration order*) is green.
      No `data_objects.rs` row is needed: that gating exists only for errors
      raised from native code via `raise_error_into`, and both new codes are
      raised from MFBASIC bodies with their own literal messages.
- [x] Re-point the classified sites (surrogate → 77050025, depth → 77050024,
      non-finite stringify → ErrFloatNaN/ErrFloatInf as detected,
      `__json_toNumber` → propagate `err.code` and message).
      The Boolean predicate `__json_isInvalidNumberText` became the SUB
      `__json_requireFiniteNumberText` (file renamed to match) so the NaN/Inf
      distinction is made where it is detected rather than duplicated at both
      call sites.
- [x] `tests/acceptance/src/json.mfb`: update the expected codes for exactly
      those cases (the suite pins codes; the CASES stay).
      The lone-surrogate pin moved out of the "string errors" case into its own
      `TCASE`, widened to five unpaired shapes plus a positive pin that a
      well-formed pair still parses; a new `TCASE` pins `1e400`/`-1e400` →
      `ErrOverflow` while keeping `01`/`1e` on `ErrInvalidFormat`. The
      deep-nesting `.run` fixture's expected code moved `77050003` → `77050024`.

Acceptance: the three review probes re-run against the new binary report
77050025 (P03/P19), 77050024 (P23-deep300), and 77050010 (P01-1e400 —
`ErrOverflow` surfacing through parse); full `cargo test --no-fail-fast` +
`scripts/test-accept.sh` (full count) green; goldens regenerated with the
delta confined to json-importing fixtures.

**MET.** Probe transcript (rebuilt release binary, probe project under `/tmp`):

```
P03-lone-high: FAIL 77050025 invalid JSON string: high surrogate escape is not followed by a low surrogate escape
P19-lone-low:  FAIL 77050025 invalid JSON string: low surrogate escape has no preceding high surrogate escape
P03b-pair-ok:  OK                       <- a well-formed pair still parses
P23-deep300:   FAIL 77050024 invalid JSON format: nested too deeply
P01-1e400:     FAIL 77050010 JSON number 1e400 is not representable: Arithmetic overflow ...
P-malformed:   FAIL 77050003 invalid JSON format   <- ordinary malformed JSON unmoved
```

Gates:

- `mfb test tests/acceptance` → `Tests: 734  Pass: 734  Fail: 0`, with all 31
  top-level groups present (`grep -a "^\* " ` on the captured output — the
  output contains NUL bytes from the control-character cases, so a plain `grep`
  silently reports nothing; `-a` is required to see it at all).
- `scripts/test-accept.sh` → **1348 ran, 9 mismatches**, every one a
  json-importing fixture: the 5 `.ir` dumps of `json_codegen_cover_rt`,
  `json_behavior`, `json_number_roundtrip_rt`, `json_parse_deep_scalar_scan_rt`
  and `inline-trap-union-bind-rt`; `func_json_stringify_invalid_runtime.ir`; and
  the deep-nesting fixture's `build.log` + `.ast` + `.ir`. Delta confined exactly
  as predicted. Inspected before regenerating: `build.log` moved
  `err:77050003` → `err:77050024` on both lines, the `.ast` shifted by the 2
  comment lines added to that fixture's source, and the `.ir` replaced the
  `#json_isInvalidNumberText` conditional with an `eval` of
  `#json_requireFiniteNumberText`. Regenerated with
  `scripts/sync-goldens.sh` (22 files across 8 tests).
- `scripts/artifact-gate.sh all` → the 5 `json_codegen_cover_rt.*.ncodesum`
  sentinels drifted (all five targets), which `sync-goldens.sh` does not touch;
  regenerated with `scripts/regen-ncodesum.sh`, which refreshed 141 goldens of
  which **only those 5 changed**. Re-run clean.
- `cargo test --no-fail-fast`: unit binary **3728 passed, 0 failed** (392s),
  repository workspace 5 passed, 0 failed; integration suites green. The
  `errorcode` guards in particular — `table_matches_registry`,
  `table_has_no_duplicate_names_or_codes`, and
  `every_builtin_declared_error_is_a_table_name` (which validates the new
  `errors:` entries name real table rows) — all pass.
- `cargo fmt` both roots: no churn.
Commit: 2ccc4cce5

### Phase 2 — truthful docs

- [x] `func_parse.rs` DESC: replace the "approximated rather than rejected"
      magnitude claim with the real contract (precision is approximated;
      magnitude beyond Float raises the propagated `toFloat` code); document
      the three specific codes.
      Split into precision (silently rounded, as in JavaScript) vs magnitude
      (raises `ErrOverflow`), with the JS divergence named. The depth paragraph
      now cites `ErrDepthExceeded`, and the surrogate paragraph was pulled out
      of the escape list into its own paragraph citing `ErrInvalidSurrogate`.
      No plan-number note was added to the rendered prose (a man page must not
      cite plan docs); the plan-120-F revisit is tracked in Corrections C2.
- [x] `func_stringify.rs` DESC: name ErrFloatNaN/ErrFloatInf for non-finite.
      Also documented the 25-place search's real failure mode truthfully — see
      Corrections C3.
- [x] `helper_stringify_number.rs` comment: correct the "plain
      `toString(Float)` is the shortest-round-trip formatter" claim
      (measured: it is fixed-point with a 2-place default —
      `float_format.rs:1-3`).
- [x] `mod.rs` MODULE_DESC: the "very large or very precise values may lose
      precision" sentence was measurably wrong in both halves — added because
      Phase 2 is the truthful-docs phase and this is the package's front page.
      Now: precision rounds silently, magnitude raises (`1e400` → `ErrOverflow`
      at parse, `1e-30` → `ErrInvalidFormat` at stringify).
- [x] **(task added during execution — see Correction C7)** Update the
      `errors:` declarations so the rendered Errors tables match what the
      bodies now raise: `func_parse.rs` gains `ErrInvalidSurrogate`,
      `ErrDepthExceeded` and `ErrOverflow` alongside `ErrInvalidFormat`, and
      `func_stringify.rs` gains `ErrInvalidFormat`, `ErrFloatNaN` and
      `ErrFloatInf` (it declares none today and therefore renders no Errors
      section at all). Re-run the render + man gates and confirm golden
      neutrality.
      Verified rendered: `mfb man json parse` now tables all four codes
      (77050003 / 77050010 / 77050024 / 77050025) and `mfb man json stringify`
      renders an Errors section for the first time (77050003 / 77050013 /
      77050014). Golden-neutral, measured: a scoped `test-accept.sh` over the
      8 json fixtures passed with no regeneration, confirming the `errors:`
      field — like `Parameter.desc` (Correction C5) — is a man-render-only
      input that never reaches the assembled source.
- [x] Render gates: `mfb man json parse` / `json stringify`,
      `scripts/man-census.sh --memory-scope`,
      `scripts/man-run-examples.sh json --run`.
      Measured: `man-census.sh --memory-scope` → `unclassified memory-vocabulary
      hits: 0`; `man-run-examples.sh json --run` → `examples: 12  built: 12
      ran: 12  failed: 0`. The renderer auto-derived the new See-also rows
      (`errorCode::ErrDepthExceeded`, `errorCode::ErrInvalidSurrogate`,
      `errorCode::ErrOverflow`) from the codes named in the DESC.

Acceptance: rendered pages match behavior; man gates green; full suite green;
fmt both roots + `cargo check --all-targets`.

**MET.** `mfb man json parse` renders the corrected magnitude/precision split,
the surrogate paragraph and the depth code, and the renderer derived a See-also
listing `errorCode::ErrDepthExceeded`, `ErrInvalidFormat`, `ErrInvalidSurrogate`
and `ErrOverflow` from the codes now named in the prose.
`mfb spec diagnostics error-codes` renders both new rows.
`scripts/man-census.sh --memory-scope` → `unclassified memory-vocabulary hits: 0`.
`scripts/man-run-examples.sh json --run` → `examples: 12  built: 12  ran: 12
failed: 0`. `cargo check --all-targets` clean; `cargo fmt` both roots: no churn.
Commit: 2ccc4cce5 (one commit carries both phases — the doc corrections in Phase 2 were only provable against the code changes in Phase 1, so splitting them would have left an intermediate commit whose rendered pages contradicted its behavior)

## Validation Plan

- Tests: acceptance json suite (updated codes); the review probe programs
  kept as new rt fixtures if convenient, else the acceptance cases extended
  to cover lone-surrogate/depth/1e400 codes explicitly.
- Doc sync: Phase 2 list + spec error-codes table (build input).
- Acceptance: family standard — full `cargo test --no-fail-fast`,
  `scripts/test-accept.sh`, `scripts/artifact-gate.sh all` (regenerated),
  fmt, check `--all-targets`.

## Open Decisions

- ~~Constant names — `ErrDepthExceeded`/`ErrInvalidSurrogate` (recommended,
  generic) vs json-specific names.~~ **CLOSED: the generic names, as
  recommended.** Both describe the mistake, not the reader that caught it, so
  the regex engine's own nesting cap (`__REGEX_DEPTH_LIMIT`) and any future
  parser can raise the same two codes. Codes 24/25 re-verified free at land
  time (Prerequisites row above).

## Corrections

**C1 — the review's I1 blames the wrong component; plan-120-F's premise is
partly false.** The plan says (§References, and plan-120-F's header) that
`toFloat("5e-324")` and `toFloat("1e-30")` "FAIL outright" and that valid JSON
documents are therefore unreadable. Measured on this branch with a probe that
separates the two calls:

```
1e-30  toFloat: OK -> 0.00000000000000000000
5e-324 toFloat: OK -> 0.00000000000000000000
1e-30  parse:   OK          (parse alone)
1e-30  parse+stringify: FAIL 77050003
```

`toFloat` accepts both. What fails is `json::stringify`, at
`helper_stringify_number.rs:49` — the 25-place fixed-point search finds no
rendering that round-trips, because `1e-30` needs its first significant digit at
place 30. So this is **I7 / plan-120-G territory (rendering), not I1 / plan-120-F
(parsing)**. Consequences, applied:

- plan-120-F's Phase 1 task "root-cause the `1e-30`/`5e-324` rejections" is
  **already answered here** — there is no rejection in `toFloat` to root-cause.
  F keeps its real content (correct rounding, the I3 1-ULP defect, the torture
  corpus); its *range* claim is withdrawn. Marked in F's own Corrections.
- plan-120-G's diagnosis is confirmed rather than weakened: it already names
  `5e-324` and `1e-30` as stringify failures needing scientific rendering.
- G's Prerequisites row "plan-120-F landed (correct `toFloat`)" stays: G's
  shortest-digit search still verifies round trips through `toFloat`, so F's
  rounding correctness is a genuine dependency even though its range claim was
  wrong.

**C2 — `func_parse.rs`'s magnitude claim was false in the direction the plan
said, and the fix is now permanent, not provisional.** The plan asked for a note
that the contract "changes again when plan-120-F lands". Per C1 it does not: F
changes *rounding*, not what converts. `1e400` overflows binary64 by 92 orders of
magnitude and will still raise `ErrOverflow` after F, because plan-120-F's own
Non-goals keep overflow raising. The DESC therefore states the contract flatly.

**C3 — M6's non-finite stringify path is unreachable, measured.** The plan
treats "non-finite stringify" as a live failure to re-code. Probing it:

```
mknan trapped at construction: 77050013   (0.0 / 0.0, caught in the constructing FUNC)
mkinf trapped at construction: 77050015   (1.0 / 0.0)
nan stringify: 0                          (the trapped fallback 0.0 is what arrives)
```

`observe_float` traps a non-finite `Float` at its construction site, so no
`json::JsonNum` can be built around one and the guard inside
`__json_stringifyNumber` never fires from ordinary MFBASIC. The guard is kept and
now reports `ErrFloatNaN`/`ErrFloatInf` as the plan specified — it is a real
defensive check that predates this plan, not new dead code — but the letter's
acceptance for M6 is "the guard names the right code", not "a program observes
it". `func_stringify.rs`'s DESC already stated this unreachability correctly and
was left standing.

Separately: `helper_stringify_number.rs:49`'s FAIL (no rendering round-trips) is
a *different* site from the two non-finite guards and is NOT part of M6. It stays
`77050003` here and is deleted by plan-120-G, which replaces the 25-place search.
Its real reachability (C1) is now documented rather than left implied.

**C4 — adding a code touches a THIRD place the plan did not list.** §3 says the
new constants "land in `errorcode/mod.rs` AND the spec table
(`02_error-codes.md`) in the same commit". There is a third: the guard
`table_has_no_duplicate_names_or_codes` (`errorcode/mod.rs`) holds
`LEGACY_ROWS: usize = 45` plus an `ADDED_SINCE_MIGRATION` name list, and asserts
`names.len() - ADDED_SINCE_MIGRATION.len() == LEGACY_ROWS`. Adding two constants
without listing them reds it:

```
assertion `left == right` failed: the migration's legacy rows must all still be present
  left: 47   right: 45
```

That list is the post-migration changelog, so the fix is to add both names with
their rationale comment (the shape the three existing entries use), NOT to bump
`LEGACY_ROWS` — bumping it would let a genuinely lost legacy row hide behind a
new one, which is the exact thing the split count exists to prevent. Green after:
all 6 `errorcode` tests pass.

**C5 — a doc bug found while reading, fixed here (AGENTS: never leave a bug
you found).** `func_stringify.rs`'s `value` parameter carried `json::get`'s
parameter text verbatim — "The value to read from. … *traversal only succeeds
through JsonObj members*" — which is meaningless for a serializer that renders
every variant, and appears on `mfb man json stringify`'s Parameters table. It
was a copy-paste from `func_get.rs` (the two strings were byte-identical, and
the pasted line even kept `get`'s indentation). Replaced with text describing
what `stringify` actually accepts. In scope because Phase 2 of this letter is
the truthful-docs phase and this is the same rendered table it corrects.

**C7 — the plan missed the `errors:` declarations, and nothing gates them.**
A member's rendered **Errors** table is derived from the `errors: vec![...]`
field on its `Implementation`, not from the prose. Measured before fixing:

```
$ mfb man json parse   → Errors table lists ONLY 77050003 ErrInvalidFormat
$ mfb man json stringify → no Errors section at all (errors: vec![])
```

So this letter, which exists to give `json::parse` three more codes, would have
shipped a page still claiming it raises one. `json::stringify`'s empty list is
worse and pre-dates this plan: that member could always fail (the
no-rendering-round-trips `FAIL` at `helper_stringify_number.rs:49`), and now has
three distinct codes, yet its page renders no Errors section whatsoever.

Nothing catches this. `every_builtin_declared_error_is_a_table_name` checks that
each declared name EXISTS in the errorCode table; there is no check that the
declared set is COMPLETE with respect to what the body raises. That is the
`&'static str` hazard AGENTS.md warns about — "no compiler gate catches a doc
error; `mfb man` output is the only verification" — so the only defense is
rendering the page and reading it, which is what found this.

Added as an explicit Phase 2 task above rather than folded in silently.

**C8 — the helper rename leaves one dangling citation, deliberately not
fixed.** `planning/old_man/builtins/json/stringify.md:65` cites
`[[src/codegen/builtins/json/package.mfb:__json_isInvalidNumberText]]`. Both
halves of that citation are stale, and the second half was stale before this
plan: `src/codegen/builtins/json/package.mfb` does not exist (confirmed by
`ls`) — the builtin migration replaced it with the per-member `func_*.rs` /
`helper_*.rs` files. `planning/old_man` is the *archived* pre-migration man
source (`src/cli/man.rs:6`, "ported to `planning/old_man`"), not a rendered or
gated document, so the correct action is to leave the archive alone rather than
partially freshen a citation into a file that is gone. Recorded here so a future
citation sweep does not mistake it for a regression this letter introduced.

**C9 — one site the plan's §2 mis-located.** §2 says the non-finite site lives in
"`helper_stringify_number.rs`/`helper_is_invalid_number_text.rs`". The FAILs were
in `helper_stringify_number.rs` only; `helper_is_invalid_number_text.rs` held the
Boolean predicate and raised nothing. Rewriting it as the SUB
`__json_requireFiniteNumberText` is what moved the FAIL into that file, so the
plan's description is true of the result rather than of the starting state.

## Open Decisions

*(closed — see below)*

## Summary

Plumbing-only letter that turns three silent catch-alls into precise,
documented codes and stops discarding `toFloat`'s diagnosis; it also plants
the doc corrections the number letters (F/G) later build on.

**Outcome.** Landed as planned: two new registry codes (`ErrDepthExceeded
77050024`, `ErrInvalidSurrogate 77050025`), six re-pointed `FAIL` sites, and
`__json_toNumber` re-raising `err.code` instead of discarding it. Golden delta
was confined to json-importing fixtures exactly as predicted (9 acceptance
mismatches + 5 `.ncodesum` sentinels, all inspected before regeneration).

The letter also turned out to be the family's reconnaissance pass. Because it
had to read the number path to write truthful docs, it **falsified plan-120-F's
I1 premise** (`toFloat` does not reject `1e-30`/`5e-324`; `json::stringify`
does — Correction C1) while **confirming F's I3 premise** with a concrete
1-ULP vector (F-C1b), and it captured the Node oracle sets that C, D, E, F and
G were each going to have to derive independently. Those are recorded in the
respective letters rather than here.

Three seams the plan did not know about were found and closed: the
`ADDED_SINCE_MIGRATION` guard (C4), the `errors:` declarations that drive the
rendered Errors tables and are gated by nothing (C7), and a `get`-to-`stringify`
copy-paste in a parameter description (C5).

## Family merge-back (all seven letters)

`main` had advanced by 53 commits while plan-120 ran, so it was merged into
`worktree-P-120` and every gate re-run on the merged tree before landing.

**Nine conflicts, all `.ncodesum`** — five `crypto_codegen_cover_rt` and four
`crypto-ec-valid`. Both sides had legitimately changed them: plan-120-E's
indirect-call tag check on this branch (it adds a check at every call through a
`FUNC`-typed value, and crypto's callbacks are such calls), and main's own work.
A `.ncodesum` is a drift sentinel rather than a behavioural test, so neither
side's text is the answer — the value for the merged code is. They were
regenerated from the merged tree, and `regen-ncodesum.sh` refreshing 141 goldens
while changing exactly those 9 is the evidence that the merge disturbed nothing
else.

| Gate, on the merged tree | Result |
|---|---|
| `cargo test --no-fail-fast` | **4477 passed, 0 failed** |
| `scripts/test-accept.sh` | **1351 test(s) ran, 0 mismatches** |
| `scripts/artifact-gate.sh all` | **1330 tests, 1493 build(s), 1834 golden(s), 0 diff(s)** |
| acceptance TESTING app | **758 / 758** |
| `cargo fmt --all` + the `repository/` workspace | clean |

### What the family delivered

Seven letters, all seven closed:

| Letter | Change |
|---|---|
| A | `ErrDepthExceeded` / `ErrInvalidSurrogate`, six re-pointed FAIL sites |
| B | array traversal in `get`/`getOr`, by decimal index |
| C | `JSON.stringify`'s byte shape: no `\/`, `-0` renders `0` |
| D | `stringify(value, indent)` pretty-printing |
| E | `parse(text, reviver)` — and the indirect-call error-propagation fix it uncovered |
| F | correctly-rounded `toFloat(String)` (Eisel–Lemire + exact fallback) |
| G | ECMAScript number rendering, byte-identical to `JSON.stringify` |

Two compiler defects were found and fixed along the way that no letter set out
to find: **errors did not propagate out of a call through a `FUNC`-typed value**
(E-C1 — silently wrong for a scalar return, SIGSEGV for a pointer-typed one),
and **`toFloat` was not correctly rounded** (F, the root of the review's I3).
The first was found because a reviver needed to be able to fail; the second
because G's rendering could not be verified without it.

The two interop probes that motivated the family both close:
`same=30 different=0 render-fail=0`, where the 2026-09-01 review recorded
`1e-7` reading back as a *different* double in Node and `1e-30`/`5e-324` having
no JSON form at all.
