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
| plan-120-D landed | `ls planning/plan-120-D* → planning/completed/` | **MET** — D committed as `b08678cbf` on `worktree-P-120` with every gate green (95 cargo-test binaries / 0 failures, 750/750 acceptance, artifact-gate 1830 goldens / 0 diffs, 16/16 man examples). Family order only, as stated — but D turned out to matter for a reason the plan did not foresee: it fixed the union-wrapping bug (D-C3) that this letter's overload would otherwise have hit too, since `json::parse` also takes/returns the `Json` union. |

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
- [x] `func_parse.rs`: add the 2-arg `Implementation` + `__json_parseRevive`
      / `__json_revive` helper bodies. The residual matcher risk did NOT
      materialise: `ParameterType::func(...)` in a member's PARAMETER list is
      accepted on a `Body::mfb` carrier, so the `Route`-style record wrapper
      fallback was not needed. Verified by building the overload before writing
      its body, as the task directed.
- [x] Acceptance cases: N07 twin (increment by key), index-key vocabulary
      (array of two, reviver records keys seen → `"0","1",""` order),
      bottom-up order proof (nested object; inner revived before outer),
      JsonNull return stored verbatim, reviver failure propagates (a FAIL
      inside the reviver surfaces to the caller). Landed as the
      `parse — reviver overload` TGROUP: 8 cases, all green (suite 758/758).
      Two deviations from the task as written, both strengthening it:

      1. **The revivers are state-free.** "Records keys seen" implies a global
         log, which couples every case to the order the suite runs them in.
         Each reviver instead encodes what it saw into its RETURN value, so the
         assertion is on the returned document and each case stands alone.
      2. **The order proof is a discriminator, not a trace.** A reviver that
         merely records keys passes under either walk order. `jsonRevObserve`
         rewrites `inner` to 99 and has `outer` snapshot what it RECEIVES:
         bottom-up yields `{"outer":"{\\"inner\\":99}"}`, top-down would
         yield `{"inner":1}` instead. The bytes differ, so the test can fail.

      Added beyond the five: duplicate-keys-collapse-before-revival
      (`{"a":1,"a":2}` → `{"a":3}`, pinning References detail 3, which was
      asserted but never covered) and a 1-arg-unchanged case. Every expected
      string was captured from Node v24.12.0 running the identical reviver and
      diffed byte-for-byte — all eight matched on the first comparison.
- [x] DESC/EX (`mfb man json parse` documents both forms, the key
      vocabulary, and the no-deletion divergence); man gates. DESC gained a
      **The reviver form** section covering the innermost-first order, all three
      key spellings, duplicate collapse happening before revival, the
      parse-completes-first guarantee, error propagation, and the `undefined`
      divergence; the `reviver` Parameter carries its own description, so the
      rendered Parameters table explains the callback without the reader
      having to reach DESC. Two examples added: doubling every number, and
      acting on one member plus the finished document under `""`.

      Gates: `mfb man json parse` renders both overloads in the Overloads
      block; `scripts/man-run-examples.sh json --run` → 18 built / 18 ran /
      0 failed, with the two new ones printing `{"a":2,"b":[4,6]}` and
      `"document: {\\"name\\":\\"Ada\\",\\"id\\":7}"` — the latter
      showing `name` already revived when the root call saw it, so the example
      demonstrates the order rather than merely asserting it;
      `scripts/man-census.sh --memory-scope` → 0 unclassified hits.
- [x] `scripts/artifact-gate.sh all`: 0 diffs outside new fixtures (1-arg
      path untouched). **The prediction was wrong — see Correction E-C2.** The
      first run reported 26 diffs; all 26 were localized, attributed and
      regenerated, and the gate now reads
      `1329 tests, 1492 build(s), 1832 golden(s) checked, 0 diff(s)`.
- [x] **Added task (not in the plan as written): sync
      `src/docs/spec/stdlib/04_json.md`.** Found while checking whether the
      embedded spec needed E's reviver. It did — and so did four earlier
      letters. See Correction E-C3; every letter's doc-sync task said
      `mfb man`, and only A touched `mfb spec` (and only the global
      error-code table, not the json topic). Eleven edits; the four `[[…]]`
      citations added all resolve (`spec_citations_resolve`, and all 26
      `docs::` tests, green).
- [x] **Added task: repair two assertions the overload invalidated.** The full
      `cargo test --no-fail-fast` came back `95 binaries, 2 failed`, both
      stale facts rather than broken behaviour — see Correction E-C4.

Acceptance: all Phase 1 cases green; artifact-gate clean; full
`cargo test --no-fail-fast` + `scripts/test-accept.sh` green; fmt + check.
Commit: —

## Validation Plan

- Tests: the five acceptance cases above.
- Doc sync: parse DESC/EX.
- Acceptance: family standard.

## Open Decisions

- ~~Key for the root call — `""` (recommended: Node parity) vs omitting the
  root call entirely.~~ **CLOSED on `""` (Node parity).** The matcher check
  passed (see the Phase 1 box above) and the behaviour is confirmed by probe:
  parsing `{"outer":{"inner":1}}` with a recording reviver logs
  `inner=1 | outer={"inner":1} | ""={"outer":{"inner":1}} |` — children
  first, root last under the empty-string key, matching Node v24.12.0. The Node capture in References shows the root call is
  not an incidental extra — it is the only call that sees the whole document,
  and the idiomatic reviver (`(k,v) => k === "" ? finish(v) : v`) depends on
  it. Omitting it would make the common "transform the root once" pattern
  impossible without a second walk. MFB has no reason to trip on an empty-string
  key: it is an ordinary `String`, and `json::get` already accepts `""` as a
  map key. Confirm during Phase 1 and mark closed.

## Corrections

- **A missing prerequisite, landed as part of this letter: indirect calls did
  not propagate errors.** The plan assumed a reviver could simply `FAIL` and
  have it surface to `json::parse`'s caller. It could not. A `FAIL` inside a
  function invoked through a `FUNC`-typed VALUE never checked the returned
  error tag, so per the fallible-call ABI (`mfb spec memory fallible-call-abi`)
  `x1`'s error CODE came back as the return VALUE:

      applyInt(boomInt, 1)  ->  77050002        (want: trapped)

  For an `Integer` return that is a silently wrong number; for a pointer-typed
  return — a record, a `List`, or a union such as `json::Json` — the caller
  dereferences the code as an address and dies with SIGSEGV. That is how it was
  found: `json::parse(text, reviver)` with a failing reviver crashed, which is
  exactly this phase's fifth acceptance case.

  Root cause was in `emit_function_value_call`
  (`src/codegen/engine/builder/builder_emit_helpers.rs`): an
  `if return_type.is_none()` guard copied from the direct-call emitter
  `emit_call`, where `Some(..)` legitimately means "raw result — the caller
  handles the tag itself" (its only `Some` caller is the inline-`TRAP`
  machinery). The two ordinary indirect call sites pass `Some(..)` only to
  communicate the declared return type, so propagation was suppressed with
  nobody materialising the Result. Cousin of bug-448 in the raw path next door
  (`emit_function_value_call_raw`). Fix: extract an `emit_tag_check` closure and
  run it unconditionally on both branches.

  Pinned by a new fixture,
  `tests/rt-behavior/functions/function-value-error-propagates-rt`, which
  asserts BOTH call forms for `Integer`, record and `List` returns plus the two
  success paths — the direct form was always correct, so a test checking only
  the indirect form would pass equally against a fix that broke the direct one.
  Proven RED against the reverted guard (`indirect-int  value 77050002`, then
  `exit=139` at the record case) and GREEN with the fix (all seven lines, exit 0).

  Per the skill's table this was a prerequisite no letter covered, not a
  falsified premise: the design works, it just rested on a compiler guarantee
  that did not yet hold.

- **E-C2: "0 diffs outside new fixtures (1-arg path untouched)" was a
  miscalibrated prediction.** The gate's first run reported **26 diffs**. Per
  AGENTS.md an unexpected artifact diff is a bug-hunt trigger, so each was
  localized before anything was regenerated. All 26 decompose into exactly two
  intended changes, with nothing left over:

  **12 diffs — the `json` package grew.** The 1-arg path is untouched
  *semantically*, but adding `__json_revive` and `__json_parseRevive` changes
  the package's assembled source, and every json importer embeds that. Diffing
  `inline-trap-union-bind-rt.ir` and filtering out `"line":` and the `$matchN`
  desugar counters leaves **zero `<` lines** — the change is purely additive
  plus renumbering. This is the known drift recorded for a registry
  `description`; a new helper does the same thing, only more so because it
  reaches `.ncode` as well as `.ir`. The plan's expectation should have been
  "the 1-arg path is behaviourally untouched and its goldens drift by
  construction", which is what it is corrected to.

  **14 diffs — the indirect-call fix (E-C1) adds a tag check.** These are
  `collections`/`crypto` fixtures that import no json at all, and their `.ir`
  is unchanged while `.ncode` moved — the right signature for a change below
  IR. Isolated by building `collections_codegen_cover_rt` twice, once with the
  guard reverted, and diffing the two `-ncode` dumps. With label counters
  normalized the delta is exactly:

  - 5 tag-check blocks (`cmp_imm x0,0` / `b.eq call_value_ok_N` / restore-and-
    `ret` / label) — one per indirect call site in the fixture;
  - 4 new `pending_result_{value,tag,message,source}` frame slots at offsets
    200–224, growing the frame `656 → 688` (+32 = 4 x 8);
  - every later stack offset shifted by that same +32, and label counters
    renumbered.

  One addition is worth calling out as a correctness signal rather than churn:
  the new error path emits `bl _mfb_rt_drop_owned_collection`, so a callback
  that fails inside `collections::mapValues` now releases the collection on the
  way out instead of propagating past it.

  Regenerated: 19 `.ncodesum` (via `regen-ncodesum.sh`, which refreshed 141 and
  changed exactly those 19 — bounding the blast radius) and 7 `.ir`. Six of the
  `.ir` went through `sync-goldens.sh`; `tests/byte-identity/json`'s was
  hand-copied because `test-accept` skips `byte-identity/`, and its `.ast` was
  confirmed byte-identical first, proving the fixture's own source did not move.
  7 + 19 = 26, matching the diff count exactly.

- **E-C3: the embedded spec was never synced by ANY letter of this family.**
  Checking whether `mfb spec stdlib json` needed a reviver section turned up
  that the page was stale with respect to **five** letters. It was not stale in
  a cosmetic way — it stated wrong error codes and a wrong escaping rule, both
  of which a reader would act on:

  | Claim in the page | Reality | Owed by |
  |---|---|---|
  | nesting past 256 "rejected with `77050003`" — on a line citing the very helper that changed | raises `77050024` `ErrDepthExceeded` | A |
  | "All failures raise error `77050003`" | also `77050024`, `77050025`, and `toFloat`'s re-raised `ErrOverflow` | A |
  | NaN/±inf "rejected with error `77050003`" | `77050013` / `77050014` | A |
  | number formatting: "rendered with 9 fractional digits" | a search over 1..25 places verifying the round trip | C (bug-304) |
  | "the solidus `/` is always escaped on output" | it is no longer escaped at all | C |
  | "The path addresses object fields only — there is no array-index step" | arrays are indexed by decimal token | B |
  | the get/getOr step table | had no array rows | B |
  | error-code table | listed 2 of the 7 codes json can raise | A, C |
  | "producing compact output — no spaces, no newlines, no indentation" | two indented overloads exist | D |
  | no mention of a reviver | this letter | E |

  Fixed all eleven in one pass, adding a `Revival: parse(text, reviver)` section
  and citations to the four helpers the family added
  (`__json_revive`, `__json_stringifyIndent`, `__json_arrayIndex`,
  `__json_requireFiniteNumberText`). Rendered via `mfb spec stdlib json`;
  `spec_citations_resolve` and all 26 `docs::` tests green.

  **The transferable lesson: "doc sync" in each letter meant `mfb man`, and
  every letter honoured it, so nothing was skipped through carelessness — the
  task simply never named the second surface.** A future plan touching a builtin
  should say which of the two it means, or say both.

- **E-C4: two test assertions went stale, and neither was a behaviour
  regression.** `cargo test --no-fail-fast` reported 95 binaries with exactly 2
  failures:

  - `json::tests::generic_dispatch_reaches_json` — `arity("json.parse")` is
    now `Some((1, 2))`, asserted `Some((1, 1))`. A factual mirror of the
    descriptor, and the second overload is this letter's entire point.
  - `registry::tests::agreed_argument_type_answers_where_overloads_agree` —
    written by me in plan-120-D, using `json.parse` as its example of a
    *single-implementation* member. E made that premise false.

  The second needed care rather than a number change: the guarantee under test
  is that `agreed_argument_type` and `argument_types_typed` are exact
  complements (2-or-more implementations vs exactly 1) so they can never both
  answer for one member. That property still holds — only the example went
  stale. So `json.get` takes over as the single-implementation example, and
  `json.parse` comes back as a *new* case asserting the other side: position 0
  agrees (`String`, so `agreed_argument_type` answers and `argument_types_typed`
  declines), and position 1 exists in only one overload, which is not agreement
  and must decline — otherwise a 1-arg call site would be handed the reviver's
  type. Net effect is more coverage than before, not less.

## Summary

An additive HOF overload built as a post-order walk over the untouched
parser; the single open verification is the registry matcher's handling of a
`Func` parameter, which collections already exercises.
