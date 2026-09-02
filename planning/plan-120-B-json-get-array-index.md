# plan-120-B: json::get / json::getOr — array-index path steps

Last updated: 2026-09-01
Effort: medium (1h–2h)
Depends on: plan-120-A (family order only; no design dependency)

Let a `json::get`/`json::getOr` path step select an array element. Today the
path is object-keys-only by documented design (`func_get.rs:21-26`: "there is
no numeric-index form"), so any array traversal is manual `MATCH` code — the
review flagged it as the one usability gap in an otherwise Node-superior pair
(Node has no path reader at all).

References:

- `src/codegen/builtins/json/func_get.rs` (DESC + `__json_get` MFBASIC body —
  a `FOR EACH` over path with a `MATCH` on the current variant),
  `func_get_or.rs` (same shape, defaulting).
- RFC 6901 (JSON Pointer) array-token grammar — the precedent for which
  strings count as indexes: `0` or a nonzero digit followed by digits; no
  leading zeros, no sign, no `-` support here (we have no append semantics).
- `tests/acceptance/src/json.mfb` get/getOr cases.

## Prerequisites

Family gate in plan-120-A.

| Must be true | Command | Status |
|---|---|---|
| plan-120-A landed | `ls planning/plan-120-A* → planning/completed/` | **MET** — A committed as `2ccc4cce5` on `worktree-P-120` with every gate green (95 cargo-test binaries / 0 failures, 734/734 acceptance, artifact-gate clean). The family runs as one integration branch, so "landed" means "landed on the branch this letter builds on"; the archive move to `planning/completed/` happens for every letter when the branch merges. |

## 1. Goal

- `json::get(parse("{\"items\":[10,20]}"), ["items", "1"])` returns the
  `JsonNum 20`; out-of-range or non-index tokens on an array fail with
  `ErrNotFound` (get) / return the default (getOr); all existing object-path
  behavior is unchanged.

### Non-goals (explicit constraints)

- No signature change: the path stays `List OF String` (an index is written
  as its decimal string, RFC-6901 style). No new overloads.
- Object lookup semantics untouched: on a `JsonObj`, a token like `"1"` is a
  KEY, never an index — the token's meaning depends only on the current
  variant, so no existing program changes behavior (arrays failed before).
- No negative/relative indexing, no `-` end-marker.

## 2. Current State

- `__json_get`: `MATCH current` with a `JsonObj(obj)` arm (map lookup via
  `collections::getOr` + missing-key detection) and an else arm failing
  `ErrNotFound`. `__json_getOr` mirrors with default-return.
- Acceptance pins: "array is not traversable" cases exist
  (`grep -n "JsonArr" tests/acceptance/src/json.mfb` — census at execution;
  the not-traversable expectation flips to indexable and needs the 4-question
  review from AGENTS: written by the original json plan to pin the
  documented no-index contract, which THIS plan changes deliberately — the
  behavior change is the point, so the case is updated, not weakened).

## 3. Design Overview

In both bodies, add a `CASE JsonArr(arr)` arm: if the token matches the
index grammar (`0`, or nonzero digit then digits — checked in MFBASIC with
`strings::` predicates; `toInt` AFTER the grammar check so `"+1"`/`"01"`/
`" 1"` stay non-indexes) and `0 <= i < len(arr.items)`, step into
`collections::get(arr.items, i)`; otherwise fail `ErrNotFound` (get) /
return the default (getOr). Scalars keep failing as today.

Risk: low — pure MFBASIC body edit in a `Body::mfb` package; the subtle spot
is keeping `get` and `getOr` byte-parallel (two bodies, one contract — the
review checklist is a diff of the two arms).

Byte-identity NOT a gate (bodies change → json fixtures' goldens churn;
regenerate).

Rejected: a typed path (`List OF` union of key/index) — new public type for
a solved string grammar; RFC 6901 `-` append token — meaningless for a
reader.

## Phases

### Phase 1 — implement + pin

- [x] `func_get.rs`/`func_get_or.rs`: add the `JsonArr` arm per §3 to both
      bodies; keep them structurally parallel.
      The grammar check went into ONE new helper, `__json_arrayIndex`
      (`helper_array_index.rs`), which both arms call — so the two bodies
      cannot drift on what counts as an index, only on what they do with a
      miss (FAIL vs return the default). See Correction B-C1 for the overflow
      guard the helper carries.
- [x] DESC rewrite in both: the token rule (RFC-6901 grammar), the
      variant-decides-meaning rule, out-of-range → ErrNotFound/default;
      examples gain an array step.
      Also updated: both `INTRO` lines, the `value`/`path` parameter
      descriptions on both members, and the package `MODULE_DESC` paragraph
      that still said "The path readers operate only on object members".
- [x] `tests/acceptance/src/json.mfb`: new cases — index hit, index miss
      (range), `"01"`/`"+1"`/`"1 "` non-index tokens fail on arrays, `"1"`
      as an object KEY still works, nested `["items","1","name"]` mix;
      update the flipped no-index pins.
      Seven new `TCASE`s (four on `get`, three on `getOr`, paired). **No pin
      needed flipping** — see Correction B-C2.
- [x] Regenerate churned goldens; man render gates
      (`mfb man json get`, `man-run-examples.sh json --run`).

Acceptance: the goal example runs; acceptance suite green at full count;
full `cargo test --no-fail-fast`; `scripts/artifact-gate.sh all` with
regenerated goldens.

**MET.** §1's goal example, run end to end against the built binary:

```
goal: 20
variant: JsonNum 20      <- MATCHed, so it is the JsonNum the goal names,
                            not merely something that stringifies to "20"
```

- `mfb test tests/acceptance` → **741 pass, 0 fail** (734 before this letter;
  the 7 new `TCASE`s all present in the output by name).
- `scripts/man-run-examples.sh json --run` → **14/14 built and ran** (12
  before). Both new examples print exactly what their pages document:
  `json::get` example 2 → `"b"` then `{"name":"a"}`; `json::getOr` example 3 →
  `20`, `"none"`, `"none"`.
- `scripts/man-census.sh --memory-scope` → 0 unclassified hits (after
  Correction B-C3).
- get/getOr parallelism, the letter's stated care point: the two `JsonArr` arms
  are line-for-line identical apart from the miss action — `FAIL error(77050004,
  …)` in `get` versus `RETURN defaultValue` in `getOr`. Both call the same
  `__json_arrayIndex`, so the two members cannot disagree about what an index is.
- `scripts/test-accept.sh` → **1348 ran, 7 mismatches**, the same
  json-importing set letter A churned (5 json `.ir` dumps, the
  `inline-trap-union-bind-rt` `.ir`, and
  `func_json_stringify_invalid_runtime.ir`). Inspected before regenerating:
  `json_behavior.ir` gained 193 lines and lost 87, and **every one of the 87
  removed lines carries a `"line"` field** — i.e. pure line-number shift, no
  semantic removal. The additions are the new `#json_arrayIndex` function body
  and the two `JsonArr` arms. Regenerated with `sync-goldens.sh` (22 files
  across 8 tests) and `regen-ncodesum.sh` (141 refreshed, only the 5 json sums
  changed).
- `scripts/artifact-gate.sh all` after regeneration → **1327 tests, 1490
  builds, 1828 goldens checked, 0 diffs.**
- `cargo fmt` both roots: one cosmetic rewrap of `func_get.rs`'s `INTRO`
  assignment (the widened intro line pushed past the width). The string VALUE
  is unchanged, so no golden is affected.
Commit: —

## Validation Plan

- Tests: the Phase 1 acceptance cases (both get and getOr, hit/miss/grammar).
- Doc sync: both DESCs; `func_get.rs` intro line ("path of object keys" →
  "path of object keys and array indexes").
- Acceptance: family standard.

## Open Decisions

- None — the token grammar is fixed to RFC 6901's (recommended in §3).

## Corrections

**B-C1 — the plan's `toInt` ordering is necessary but not sufficient; a wide
digit run also has to be refused.** §3 says "`toInt` AFTER the grammar check so
`"+1"`/`"01"`/`" 1"` stay non-indexes". That is right and was done, but it leaves
a hole the plan did not name: `"9999999999999999999"` (19 digits) PASSES the
RFC 6901 grammar, so the grammar check waves it through to `toInt`, which
overflows a 64-bit Integer and raises `ErrOverflow`. On `json::get` that is the
wrong code; on `json::getOr` it is a **contract break** — that member's whole
promise is that it never fails.

`__json_arrayIndex` therefore refuses any token longer than 18 digits before
converting (18 is the widest decimal that cannot overflow). No list can hold
10^18 items, so such a token is out of range under any reading and the observable
outcome — `ErrNotFound` / the default — is unchanged. Pinned both ways in the
acceptance suite, including an explicit `expectNTrap` on the `getOr` side.

**B-C2 — no "arrays are not traversable" pin needed flipping.** §2 predicted
pins to update and asked for a census. Measured: the two existing array pins are
`getStr(json::parse("[1,2,3]"), ["x"])` and
`json::getOr(json::parse("[1,2]"), ["x"], …)` — both use the token `"x"`, which
is not an index under RFC 6901 and therefore still finds nothing. They pass
unchanged and were kept as-is; the enclosing `TCASE` was renamed from
"a non-object at any step raises ErrNotFound" to "a step that finds nothing
raises ErrNotFound", since "non-object" stopped being the reason.

This means the plan's AGENTS 4-question review of a to-be-flipped pin was not
needed: nothing was flipped, and the letter is purely additive at the test level.

**B-C3 — "JSON Pointer" is banned man-page vocabulary.** The obvious way to
cite the token grammar — "spelled the way JSON Pointer (RFC 6901) spells one" —
reds `scripts/man-census.sh --memory-scope`:

```
HIT      json::get   53  A token counts as an index only if it is spelled the way JSON Pointer (RFC 6901)
unclassified memory-vocabulary hits: 1
```

`pointer` is on the banned memory-vocabulary list (AGENTS.md: no C/Rust memory
words on a man page), and the census matches the word regardless of the RFC's
proper name surrounding it. Reworded to "spelled the way RFC 6901 spells one",
which loses nothing — the RFC number is the citation that matters — and the gate
returns to 0. The phrase survives in `helper_array_index.rs`'s MFBASIC comment,
which is compiled source rather than rendered documentation and is not scanned.

**B-C5 — a bug this letter introduced, found and fixed before landing: a
combining-mark grapheme broke `json::getOr`'s never-fails contract.** The first
implementation reused `__json_isDigit` for the trailing characters. That helper
is `RETURN ch >= "0" AND ch <= "9"` — a **lexicographic string compare**, which
is correct for its existing caller (the number scanner, which feeds it one
scanned character) but wrong for a path token, because `strings::graphemes`
yields whole grapheme CLUSTERS and a cluster is compared as a string.

`"1"` + U+0308 COMBINING DIAERESIS is one cluster that starts with `"1"`, so it
sorts inside `["0", "9"]` and passed the digit test. The token then reached
`toInt`, which raised. Measured, before the fix:

```
[combining-1]     getOr: "fallback"
[1 + combining-1] getOr: FAILED 77050003 Text parse or non-finite numeric ...
[fullwidth-1]     getOr: "fallback"
[plain-1]         getOr: 20
```

That second line is the defect: `json::getOr` is documented — by this very
letter — to never fail, and here it did. `json::get` was wrong too, raising
`ErrInvalidFormat` where its contract says `ErrNotFound`.

Fix: use a substring test, `strings::contains("0123456789", ch)`, which cannot
match a multi-scalar cluster. This mirrors `__json_isNonZeroDigit` — which is
exactly why the FIRST character was never affected and why the single-cluster
case `"1<U+0308>"` already returned the default. `__json_isDigit` itself is left
alone; it is not wrong, it is being asked the wrong question.

After the fix, all six probe rows return the default except `[plain-1]`, which
still resolves to `20`. Pinned in the acceptance suite on both members,
including `expectNTrap` on the `getOr` side (a plain `expectString` would not
distinguish "returned the default" from "failed"), plus non-ASCII digit
lookalikes — fullwidth ONE U+FF11 and Arabic-Indic ONE U+0661 — which a reader
might reasonably expect to work and which RFC 6901 excludes.

**Lesson worth carrying past this plan**: a per-character predicate written for
a scanner is not automatically safe on caller-supplied text. The scanner's
inputs are single ASCII scalars by construction; a path token is arbitrary.

**B-C4 — one scope item the plan did not list.** The package `MODULE_DESC` in
`mod.rs` states the path-reader contract on the package's front page ("The path
readers operate only on object members"), and §"Doc sync" named only the two
member DESCs and `func_get.rs`'s intro line. Left alone it would have contradicted
the members it introduces, so it was rewritten in the same commit.

## Summary

A contained MFBASIC body change; the only care point is get/getOr parallelism
and flipping the old "arrays are not traversable" pins consciously.
