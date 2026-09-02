# plan-120-E: json::parse reviver overload (M1)

Last updated: 2026-09-01
Effort: medium (1h–2h)
Depends on: plan-120-D (family order only)

Add `json::parse(text, reviver)` — the MFB analog of `JSON.parse`'s second
argument (review gap M1): a caller-supplied `FUNC(String, json::Json) AS
json::Json` invoked bottom-up on every parsed member, letting a caller
transform values (dates, wrappers, normalization) in one pass instead of
re-walking the tree.

References:

- Node semantics (probe N07): reviver runs innermost-first; called with
  (key, value); an array element's key is its index as a string; the root is
  revived last with key `""`. Node's *deletion* semantics (return
  `undefined` → drop the member) has no MFB analog — see Non-goals.
- HOF-parameter precedent in registry members: `collections.transform` /
  `filter` take `FUNC` params; `ParameterType::Func` is a registry parameter
  type (`func_spawn`-style overload + `Func` params as in collections
  descriptors).
- `src/codegen/builtins/json/func_parse.rs` (single `Implementation` today).

## Prerequisites

Family gate in plan-120-A.

| Must be true | Command | Status |
|---|---|---|
| plan-120-D landed | `ls planning/plan-120-D* → planning/completed/` | NOT MET |

## 1. Goal

- `json::parse("{\"a\":1}", fn)` where `fn(key, v)` returns `v+1` for key
  `"a"` yields the same result as Node's probe N07 (`a = 2`); call order is
  bottom-up with Node's key vocabulary (member key / index-as-string / `""`
  root).

### Non-goals (explicit constraints)

- **No member deletion.** MFB has no `undefined`; the reviver's return is
  used verbatim (returning `json::JsonNull[NOTHING]` stores a JSON null, as
  it would in a document). Documented as the one Node divergence.
- The 1-arg parse path is byte-untouched (artifact-gate pins it).
- No `context` argument (Node's rawJSON-era third reviver param) — M5 is
  deferred.
- The reviver sees the fully-parsed subtree (post-children revival), exactly
  Node's order; no streaming/SAX contract.

## 2. Current State

- `func_parse.rs`: one `Implementation`, `Body::mfb(FUNC_BODY,
  "__json_parse")`; the parse builds the tree depth-first, so the natural
  hook is a post-order walk after `__json_parse` returns (keeps the parser
  itself untouched) rather than interleaving revival into parsing.
- UNMEASURED: whether a `Func`-typed registry parameter on a `Body::mfb`
  member needs any matcher care (collections' HOF members are the working
  precedent — read one descriptor at execution and mirror it exactly).

## 3. Design Overview

Overload `parse(text, reviver AS FUNC(String, Json) AS Json)`:
`__json_parseRevive(text, reviver)` = `__json_parse(text)` then
`__json_revive("", result, reviver)`, where `__json_revive` recurses:
arrays revive each element with `toString(i)` then rebuild the list; objects
revive each member with its key then rebuild the map (insertion order
preserved — plan-120-C's contract); scalars pass through; finally
`reviver(key, node)` on the (rebuilt) node itself. Depth is bounded by the
parser's own 256 cap, so the walk needs no separate limit.

Risk: low-moderate — the map rebuild must preserve order and last-wins
duplicates are already collapsed pre-revival (parse did it), matching Node.
The overload-matcher interaction with `Func` params is the UNMEASURED item;
it is Phase 1's first task.

Byte-identity: 1-arg path pinned unchanged; new overload gated by behavior
tests mirroring the Node probe.

Rejected: interleaved revival inside the parser (touches every parse helper
for no observable difference); replacer-style key filtering (that is M2,
deferred).

## Phases

### Phase 1 — overload + walk

- [ ] Read one collections HOF descriptor and mirror its `Func` parameter
      spelling; record the pattern here.
- [ ] `func_parse.rs`: add the 2-arg `Implementation` + `__json_parseRevive`
      / `__json_revive` helper bodies.
- [ ] Acceptance cases: N07 twin (increment by key), index-key vocabulary
      (array of two, reviver records keys seen → `"0","1",""` order),
      bottom-up order proof (nested object; inner revived before outer),
      JsonNull return stored verbatim, reviver failure propagates (a FAIL
      inside the reviver surfaces to the caller).
- [ ] DESC/EX (`mfb man json parse` documents both forms, the key
      vocabulary, and the no-deletion divergence); man gates.
- [ ] `scripts/artifact-gate.sh all`: 0 diffs outside new fixtures (1-arg
      path untouched).

Acceptance: all Phase 1 cases green; artifact-gate clean; full
`cargo test --no-fail-fast` + `scripts/test-accept.sh` green; fmt + check.
Commit: —

## Validation Plan

- Tests: the five acceptance cases above.
- Doc sync: parse DESC/EX.
- Acceptance: family standard.

## Open Decisions

- Key for the root call — `""` (recommended: Node parity) vs omitting the
  root call entirely. Parity chosen unless the empty-string key trips the
  matcher somewhere.

## Corrections

*(fill during execution)*

## Summary

An additive HOF overload built as a post-order walk over the untouched
parser; the single open verification is the registry matcher's handling of a
`Func` parameter, which collections already exercises.
