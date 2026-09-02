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
| plan-120-A landed | `ls planning/plan-120-A* → planning/completed/` | NOT MET |

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

- [ ] `func_get.rs`/`func_get_or.rs`: add the `JsonArr` arm per §3 to both
      bodies; keep them structurally parallel.
- [ ] DESC rewrite in both: the token rule (RFC-6901 grammar), the
      variant-decides-meaning rule, out-of-range → ErrNotFound/default;
      examples gain an array step.
- [ ] `tests/acceptance/src/json.mfb`: new cases — index hit, index miss
      (range), `"01"`/`"+1"`/`"1 "` non-index tokens fail on arrays, `"1"`
      as an object KEY still works, nested `["items","1","name"]` mix;
      update the flipped no-index pins.
- [ ] Regenerate churned goldens; man render gates
      (`mfb man json get`, `man-run-examples.sh json --run`).

Acceptance: the goal example runs; acceptance suite green at full count;
full `cargo test --no-fail-fast`; `scripts/artifact-gate.sh all` with
regenerated goldens.
Commit: —

## Validation Plan

- Tests: the Phase 1 acceptance cases (both get and getOr, hit/miss/grammar).
- Doc sync: both DESCs; `func_get.rs` intro line ("path of object keys" →
  "path of object keys and array indexes").
- Acceptance: family standard.

## Open Decisions

- None — the token grammar is fixed to RFC 6901's (recommended in §3).

## Corrections

*(fill during execution)*

## Summary

A contained MFBASIC body change; the only care point is get/getOr parallelism
and flipping the old "arrays are not traversable" pins consciously.
