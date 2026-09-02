# plan-120-A: json — specific error codes and truthful number docs

Last updated: 2026-09-01
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
| Codes 77050024/25 unallocated | `grep -c "7705002[45]" src/codegen/builtins/errorcode/mod.rs` → 0 | MET (verify again at execution — codes race between sessions) |

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

- [ ] Census the `77050003` FAIL sites in `src/codegen/builtins/json/`
      (command in §2); classify each as surrogate / depth / non-finite /
      number-range / genuinely-malformed; record the table here.
- [ ] `errorcode/mod.rs`: add `ErrDepthExceeded 77050024`,
      `ErrInvalidSurrogate 77050025`; spec table row for each in
      `src/docs/spec/diagnostics/02_error-codes.md`.
- [ ] Re-point the classified sites (surrogate → 77050025, depth → 77050024,
      non-finite stringify → ErrFloatNaN/ErrFloatInf as detected,
      `__json_toNumber` → propagate `err.code` and message).
- [ ] `tests/acceptance/src/json.mfb`: update the expected codes for exactly
      those cases (the suite pins codes; the CASES stay).

Acceptance: the three review probes re-run against the new binary report
77050025 (P03/P19), 77050024 (P23-deep300), and 77050010 (P01-1e400 —
`ErrOverflow` surfacing through parse); full `cargo test --no-fail-fast` +
`scripts/test-accept.sh` (full count) green; goldens regenerated with the
delta confined to json-importing fixtures.
Commit: —

### Phase 2 — truthful docs

- [ ] `func_parse.rs` DESC: replace the "approximated rather than rejected"
      magnitude claim with the real contract (precision is approximated;
      magnitude beyond Float raises the propagated `toFloat` code — and note
      this changes again when plan-120-F lands); document the three specific
      codes.
- [ ] `func_stringify.rs` DESC: name ErrFloatNaN/ErrFloatInf for non-finite.
- [ ] `helper_stringify_number.rs` comment: correct the "plain
      `toString(Float)` is the shortest-round-trip formatter" claim
      (measured: it is fixed-point with a 2-place default —
      `float_format.rs:1-3`).
- [ ] Render gates: `mfb man json parse` / `json stringify`,
      `scripts/man-census.sh --memory-scope`,
      `scripts/man-run-examples.sh json --run`.

Acceptance: rendered pages match behavior; man gates green; full suite green;
fmt both roots + `cargo check --all-targets`.
Commit: —

## Validation Plan

- Tests: acceptance json suite (updated codes); the review probe programs
  kept as new rt fixtures if convenient, else the acceptance cases extended
  to cover lone-surrogate/depth/1e400 codes explicitly.
- Doc sync: Phase 2 list + spec error-codes table (build input).
- Acceptance: family standard — full `cargo test --no-fail-fast`,
  `scripts/test-accept.sh`, `scripts/artifact-gate.sh all` (regenerated),
  fmt, check `--all-targets`.

## Open Decisions

- Constant names — `ErrDepthExceeded`/`ErrInvalidSurrogate` (recommended,
  generic) vs json-specific names. Codes 24/25 re-verified free at land time.

## Corrections

*(fill during execution)*

## Summary

Plumbing-only letter that turns three silent catch-alls into precise,
documented codes and stops discarding `toFloat`'s diagnosis; it also plants
the doc corrections the number letters (F/G) later build on.
