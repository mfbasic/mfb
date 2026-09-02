# plan-120-E: json::parse reviver overload (M1)

Last updated: 2026-09-02
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

  **Re-captured verbatim from Node v24.12.0 during plan-120-A's execution.**
  Every Phase 1 acceptance case below now has oracle output to assert against:

  ```
  JSON.parse('{"a":1}', (k,v) => k==="a" ? v+1 : v)      -> {"a":2}

  # call order + key vocabulary, over {"o":{"i":1},"arr":[10,20],"s":"x"}
  keys in call order: i, o, 0, 1, arr, s, ""

  # array element keys are index strings
  JSON.parse("[10,20]", ...)  keys: 0, 1, ""

  # bottom-up, showing the VALUE each call receives
  inner=1 | outer={"inner":1} | ""={"outer":{"inner":1}}

  JSON.parse('{"a":1,"b":2}', k=>... null)      -> {"a":null,"b":2}
  JSON.parse('{"a":1,"b":2}', k=>... undefined) -> {"b":2}     <- the non-goal
  reviver that throws                            -> propagates ("boom")
  JSON.parse('{"a":1,"a":2}', ...)  calls: a=2 | ""={"a":2}
  ```

  Three details worth pinning that the plan states but had not measured:

  1. **The root really is called last, with key `""`**, in every shape —
     object, array, and after a nested walk.
  2. **A container is revived AFTER its children, and receives the already-
     revived subtree** (`outer={"inner":1}`), which is exactly the post-order
     rebuild §3 describes. So `__json_revive` must rebuild the child
     collection before calling the reviver on the parent, not after.
  3. **Duplicate keys collapse before revival** — `{"a":1,"a":2}` calls the
     reviver once, with `a=2`. §3 asserts this; confirmed. It falls out for
     free in MFB because `__json_parse` already collapsed last-wins into the
     map before `__json_revive` ever runs.
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

- [x] Read one collections HOF descriptor and mirror its `Func` parameter
      spelling; record the pattern here. **(Discharged during plan-120-A's
      execution — the read cost nothing and this was Phase 1's blocking
      unknown.)**

      `collections::filter` (`func_filter.rs:102-108`) is the pattern; every
      HOF member in `collections/` spells it the same way:

      ```rust
      Parameter {
          name: "predicate",
          desc: "Called once per element, in order. ...",
          aliases: &[],
          ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::Boolean),
          default: DefaultValue::None,
      },
      ```

      So it is the constructor `ParameterType::func(params_vec, return_ty)`,
      NOT a `ParameterType::Func` literal — the enum variant
      `ParameterType::Func(_, _, false)` appears only in *matchers*
      (`gen_flow.rs:68`, `func_for_each.rs:137`), where the third field
      distinguishes the shape. For this letter the reviver is concrete rather
      than generic, so no `var("T")` is needed:
      `ParameterType::func(vec![ParameterType::String, ParameterType::named("Json")], ParameterType::named("Json"))`.

      Caveat carried into the implementation task: every one of these
      precedents is `Body::abi_inline` (native lowering), so none of them
      proves a `Func` parameter works on a **`Body::mfb`** member — which is
      what this letter needs. That is the residual risk §3 flagged, narrowed
      from "how is it spelled" to "does the matcher accept it on an MFBASIC
      carrier"; verify by building the overload before writing its body.

      Partial evidence that it will: `http`'s `Route` record
      (`http/mod.rs:355-364`) carries a `handler` field typed
      `ParameterType::func(vec![named(REQUEST_TYPE)], named(RESPONSE_TYPE))`,
      and http's MFBASIC bodies call that handler — so a `Func` VALUE does flow
      through a `Body::mfb` body today. What is still unproven is a `Func` in a
      member's PARAMETER list on an MFBASIC carrier; if the matcher balks, the
      `Route`-style workaround (wrap the reviver in a one-field record) is the
      fallback, at the cost of an uglier call site. Try the direct parameter
      first.
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

- ~~Key for the root call — `""` (recommended: Node parity) vs omitting the
  root call entirely.~~ **Leaning CLOSED on `""` (Node parity), pending only
  the matcher check.** The Node capture in References shows the root call is
  not an incidental extra — it is the only call that sees the whole document,
  and the idiomatic reviver (`(k,v) => k === "" ? finish(v) : v`) depends on
  it. Omitting it would make the common "transform the root once" pattern
  impossible without a second walk. MFB has no reason to trip on an empty-string
  key: it is an ordinary `String`, and `json::get` already accepts `""` as a
  map key. Confirm during Phase 1 and mark closed.

## Corrections

*(fill during execution)*

## Summary

An additive HOF overload built as a post-order walk over the untouched
parser; the single open verification is the registry matcher's handling of a
`Func` parameter, which collections already exercises.
