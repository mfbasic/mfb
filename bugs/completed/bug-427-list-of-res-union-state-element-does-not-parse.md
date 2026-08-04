# bug-427: `List OF RES <ResourceUnion> STATE <S>` element type does not parse

Last updated: 2026-08-03
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: FIXED (8a5c7bbe3)
Regression Test: tests/syntax/resources/resource-union-state-collection-{valid,no-res-invalid},
tests/rt-behavior/resources/bug427_list_union_state_rt (full build + run)

A collection element that is a stateful resource cannot be spelled. A binding
such as

```basic
MUT streams AS List OF RES http::Stream STATE PendingState
```

fails to parse: the parser reads the element type `RES http::Stream`, returns,
and leaves `STATE PendingState` dangling, producing a statement-end error. The
same happens for `Map OF K TO RES <Res> STATE <S>`. `http::Stream` is a
**resource union** (every variant a resource), so the element must carry the
`RES` ownership marker — the `RES` in the reported spelling is correct — but the
`STATE` clause on the element is dropped on the floor.

The single correct behavior a fix produces: the collection-element grammar
accepts a trailing `STATE <T>` after a `RES` element and folds it into the
element type string (`List OF RES http::Stream STATE PendingState`), exactly as
the **thread resource plane** already does (`Thread OF RES File STATE Cursor TO
Integer`). The element then carries a uniform STATE type across the union's
variants, readable when an element is extracted. The bare no-`RES` form
(`List OF http::Stream STATE …`) stays rejected — resources in collections
require `RES` (`TYPE_RESOURCE_REQUIRES_RES`).

References:

- `mfb spec language resource-management` §15.5 (STATE at use positions), §15.6
  (resources in collections; `List OF RES File` spelling), and the resource-union
  STATE paragraph (uniform STATE type across variants).
- `mfb spec architecture type-name-encoding` — the canonical flat type-string
  grammar (`ListArg`), which must gain the STATE-bearing element form.
- The thread-plane precedent: `parse_resource_plane_type`
  (`src/ast/expr.rs:760`) folds `STATE` into a `RES` element string today.
- Found during plan-85 / union-STATE work (worktree-P-81); see memory note
  "Resource-union STATE: {tag,ptr} layout + 3-place close wiring".

## Failing Reproduction

Minimal source (a resource union `http::Stream` with a state record
`PendingState`), bound as a list element:

```basic
MUT streams AS List OF RES http::Stream STATE PendingState
```

- Observed: parse error — `STATE PendingState` is left unconsumed after the
  element type, reported as "Expected end of statement after binding."
- Expected: parses to a binding of type
  `List OF RES http::Stream STATE PendingState`, with the element carrying
  `STATE PendingState`.

Contrast cases:

- `Thread OF RES http::Stream STATE PendingState TO Integer` — **parses today**
  (`parse_resource_plane_type`, `src/ast/expr.rs:760`). This is the exact
  behavior the list element should mirror.
- `MUT streams AS List OF RES http::Stream` — parses today (no STATE clause).
- `MUT streams AS List OF http::Stream STATE PendingState` — must **stay
  rejected**: a bare (no-`RES`) resource element is already
  `TYPE_RESOURCE_REQUIRES_RES`; the dangling `STATE` is a second reason.

## Root Cause

`Parser::parse_type_name_inner` (`src/ast/expr.rs:604`) parses each collection
element with a plain recursive `parse_type_name()` and then `return`s
immediately, never looking for a trailing `STATE`:

- List / Result branch — `src/ast/expr.rs:653-666`: consumes an optional `RES`
  marker (`element_res`), parses `arg = parse_type_name()`, pushes
  `" OF " [+ "RES "] + arg`, and returns. No `parse_optional_state` call.
- Map / MapEntry branch — `src/ast/expr.rs:626-650`: same shape for the value
  type after `TO` (and after an optional `RES`).

By contrast the thread plane parses its `RES` element through
`parse_resource_plane_type` (`src/ast/expr.rs:760-766`), which *does* call
`parse_optional_state` (`src/ast/link_items.rs:710`) and folds the result into
`"<resource> STATE <state>"`. That helper is the template for the fix.

There is also a **second, independent** block on the reported `MUT` spelling:
`parse_optional_state` is only reached from a binding when the binding keyword is
`RES` (`src/ast/stmt.rs:68`, the `if resource { … }` guard) — but that
top-level clause attaches STATE to the *whole* type, not to a collection
element, and `MUT`/`LET` bindings never call it at all. So even folding STATE
into the element grammar is the only correct path; the top-level state clause
cannot express an element's STATE. A list-of-resources binding is a `MUT`/`LET`
binding of an ordinary (copyable) list, so the element STATE *must* live inside
the element type string, not on the binding.

## Goal

- `parse_type_name_inner` accepts `STATE <T>` after a `RES` element in the
  List and Map-value positions and folds it into the element type string
  (`List OF RES X STATE T`, `Map OF K TO RES V STATE T`).
- The resulting flat type string round-trips through every downstream consumer
  (resolver, monomorphizer, source checker, IR semantic verifier) that strips
  the `RES ` prefix — each must also strip/carry the ` STATE T` suffix on a
  collection element.
- Extracting an element (`get`, `FOR EACH`) and `RES`-binding it yields a value
  whose STATE type is the element's `T`, checked for agreement
  (`TYPE_STATE_MISMATCH`) exactly as for a concrete stateful resource.
- The `mfb spec architecture type-name-encoding` grammar and
  `mfb spec language resource-management` §15.6 are updated to sanction the
  STATE-bearing collection element (they currently describe only `List OF RES
  File`).

### Non-goals (must NOT change)

- The bare no-`RES` element rejection (`TYPE_RESOURCE_REQUIRES_RES`) — a
  resource in a collection still requires `RES`. Do not make
  `List OF http::Stream STATE …` parse.
- `Set OF …` gains nothing: a Set element must be comparable, and a resource
  handle is not; `RES` (and therefore STATE) stays rejected after `Set OF`
  (`src/ast/expr.rs:669-686`).
- Per-variant STATE. The union STATE type is uniform across variants (spec
  §15); this bug does not introduce a per-variant state form.
- The thread resource plane's existing STATE handling — leave
  `parse_resource_plane_type` and thread type strings untouched; the list/map
  fix mirrors it but must not alter it.
- Tempting wrong fix: silently accepting and **discarding** the `STATE` token in
  the list branch so the binding parses but the element type is just
  `List OF RES http::Stream`. That masks the bug (element loses its STATE) and
  is explicitly forbidden — the STATE must survive into the type string and
  through type checking.

## Blast Radius

Every consumer that special-cases the `RES ` prefix on a collection element is a
site that must also learn the ` STATE T` suffix. Populate this list with an
actual `strip_prefix("RES ")` / `"List OF RES"` / `"TO RES"` search across the
tree during Phase 1 — do not trust this seed list as complete:

- `src/ast/expr.rs:653-666` (List branch) — **fixed by this bug** (add
  `parse_optional_state` fold).
- `src/ast/expr.rs:626-650` (Map value branch) — **fixed by this bug** (same
  fold after the value's optional `RES`).
- The resolver's `List OF `/`Map OF ` + `RES ` element handling — audit;
  likely **fixed** (must strip STATE before resolving the underlying type).
- The monomorphizer's `concrete_type_name` / substitution passes that rebuild
  `List OF RES …` — audit; likely **fixed** (must recurse past STATE).
- The source checker and IR semantic verifier's element-type strip — audit;
  likely **fixed** (element STATE agreement at extraction/`RES`-bind).
- Escape analysis / resource-float on collection elements — audit; STATE does
  not change ownership floating, but the type string it inspects now carries a
  suffix. Likely **unaffected** in logic but must not choke on the longer
  string.
- Thread resource plane (`parse_resource_plane_type`) — **unaffected**; it
  already folds STATE and is the model, not a consumer of the new list form.

## Fix Design

Mirror `parse_resource_plane_type`. In the List branch (and the Map-value
branch), after parsing the element/value type, when the element carried `RES`,
call `parse_optional_state()` and, if present, append `" STATE " + state` to the
element substring before splicing it into the collection string. Reject a
`STATE` clause after a **non-`RES`** collection element with a clear parse
diagnostic (it can only be a stateful *resource* element), so the no-`RES` case
still surfaces `TYPE_RESOURCE_REQUIRES_RES` rather than a confusing dangling
token.

Then walk the type-name-encoding consumers: everywhere a collection element is
recovered by `strip_prefix("RES ")`, also `split_once(" STATE ")` to separate
the underlying resource type from its state, resolving/monomorphizing the
underlying type and carrying the state alongside — the same decomposition the
thread plane's consumers already perform.

Rejected alternative: expressing element STATE via a top-level binding clause
(`MUT streams AS List OF RES http::Stream` + a separate STATE). Rejected — the
STATE belongs to the *element*, not the list; the list is copyable data, and a
uniform element STATE must ride the element type string so `get`/`FOR EACH`
extraction can type `.state`.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a `tests/syntax/` parse fixture for
      `MUT streams AS List OF RES <ResourceUnion> STATE <S>` (and a `Map OF K TO
      RES V STATE S` sibling). Confirm they fail today with the dangling-`STATE`
      statement-end error.
- [x] Add a negative fixture asserting `List OF <union> STATE …` (no `RES`)
      stays rejected.
- [x] Run the actual `strip_prefix("RES ")` / `List OF RES` / `TO RES` search
      and write each site's verdict into Blast Radius above.

Audit verdict (per `strip_prefix("RES ")` / `List OF RES` / `TO RES` search):
the STATE suffix is **carried, not lost** by every consumer that strips only the
`RES ` prefix and keeps the remainder — because the remainder handling already
splits ` STATE T`:

- `src/resolver/resolution.rs` (List/Map/literal element strip) — **unaffected**:
  strips `RES `, then recurses through `resolve_type_name`, whose bare-resource
  arm already splits ` STATE T` (`state_type_name`/`base_resource_name`).
- `src/ir/lower.rs:collection_iteration_type` — **unaffected**: strips only
  `RES `, so the `FOR EACH` loop variable keeps `Stream STATE S`.
- `src/builtins/general.rs:list_element`/`map_parts` — **unaffected as returns**
  (keep STATE for `get`), but the item-compare resolvers needed a fix (below).
- `src/builtins/collections.rs` resolve_append/prepend/insert/set/getOr —
  **FIXED**: compared the STATE-carrying element against a STATE-stripped item
  (`ir::verify` strips resource args); now `element_accepts_item` normalizes both
  sides by `base_resource_name`.
- `src/ast/expr.rs` List/Map branches — **FIXED** (the parse gap itself).
- monomorph / native codegen / target-shared strips — **unaffected**: proven by
  the rt-behavior fixture building to a native executable and running.

Acceptance: the new positive fixtures fail for the documented reason; the
negative fixture is rejected; the audit list is complete with a verdict per site.
Commit: 8a5c7bbe3

### Phase 2 — the fix

- [x] `src/ast/expr.rs` List branch: fold optional `STATE` into the `RES`
      element (`parse_optional_element_state`, mirror `parse_resource_plane_type`).
- [x] `src/ast/expr.rs` Map value branch: same fold after the value's optional
      `RES`.
- [x] Reject `STATE` after a non-`RES` collection element with a clear
      diagnostic (`A STATE clause requires a RES collection element…`).
- [x] Extend the in-scope consumer from the audit: the collection item-compare
      resolvers (`element_accepts_item`). The resolver / monomorphizer / source
      checker / IR verifier already carry ` STATE T` (audit above).
- [x] Element extraction (`get` / `FOR EACH`) + `RES`-bind checks STATE
      agreement (`TYPE_STATE_MISMATCH`) — verified end to end.

Acceptance: Phase 1 positive fixtures pass; the negative fixture still rejects;
Non-goals unchanged.
Commit: 8a5c7bbe3

### Phase 3 — regenerate expected outputs + full validation

- [x] Generated `.ast`/`.ir`/`build.log` goldens for the new fixtures (and the
      rt-behavior `.run` execution proof); confirmed via `test-accept.sh`.
- [x] Updated `mfb spec architecture type-name-encoding` and
      `mfb spec language resource-management` §15.6 to document the STATE-bearing
      collection element.
- [x] Ran the full `cargo test --bin mfb` suite (3783 passed) and the artifact
      gate (`all`): 1159 tests, 1569 goldens checked. The only 2 diffs
      (`control-flow-if` / `parser-hello-world` `.mir`, a `%ret0`↔`%arg0` vreg
      rename on x0) are **pre-existing on clean main** — proven by regenerating
      them with a detached `300b2a2f8` release binary (both differ there too),
      and both fixtures use no collections/resources/STATE. This fix adds **zero**
      new golden diffs.

Acceptance: full suite green; the fix's golden set is exactly the intended
change (no unintended shifts); the reproduction parses, type-checks, builds, and
runs.
Commit: 8a5c7bbe3

## Validation Plan

- Regression test(s): the `tests/syntax/` positive + negative parse fixtures,
  plus a golden `.ast`/`.ir` exercising the new type string.
- Runtime proof: a small program that builds a `List OF RES <union> STATE S`,
  extracts an element, and reads/writes `.state`, run end-to-end.
- Doc sync: type-name-encoding grammar + resource-management §15.6 (both
  currently describe only `List OF RES File`).
- Full suite: `cargo test --bin mfb`; artifact gate.

## Open Decisions

- Spec sanction. §15.5 currently restricts STATE to binding/parameter/return
  (plus the thread plane); §15.6 gives `List OF RES File` as the *only* list
  spelling. This bug extends both. Confirm the intended design is "collection
  element carries a uniform STATE" before landing, since it widens the type
  grammar. (Recommended: yes — it matches the thread-plane precedent and the
  resource-union uniform-STATE rule.)

## Summary

The engineering risk is not the parser change (a direct mirror of
`parse_resource_plane_type`) but the **downstream type-string consumers**: every
place that recovers a collection element by stripping `RES ` must also split off
` STATE T`, or the element's state is silently lost or mis-resolved. The parser
patch alone would be the tempting wrong fix. Left untouched: the no-`RES`
rejection, Set elements, per-variant STATE, and the thread plane.

## STATUS: FIXED (8a5c7bbe3)

Reproduced exactly as documented: `MUT streams AS List OF RES Stream STATE S`
(and the `Map` sibling) failed to parse with a dangling-`STATE`
"Expected end of statement after binding" error at `parse_type_name_inner`
(`src/ast/expr.rs`), which parsed the `RES` element and returned without looking
for a trailing `STATE`.

Fix:
- **Parser** (`src/ast/expr.rs`): `parse_optional_element_state` folds an optional
  `STATE T` into a `RES` collection element/value, mirroring the thread plane's
  `parse_resource_plane_type`. A `STATE` after a non-`RES` element is a clear
  parse error, not a dangling token.
- **Resolver** (`src/builtins/collections.rs` + `general.rs`): the audit found the
  resolver/monomorphizer/source-checker/IR-verifier already carry ` STATE T`
  (they strip only `RES ` and the remainder handling splits STATE). The one real
  break was the collection item-compare (`append`/`prepend`/`insert`/`set`/
  `getOr`): it compared the STATE-carrying element against a STATE-stripped item
  (`ir::verify` strips resource args), which only surfaced at the codegen-time
  `TYPE_CALL_ARGUMENT_MISMATCH` — invisible to `mfb build -ast -ir`. Now
  `element_accepts_item` normalizes both sides by `base_resource_name`, while
  `get` still returns the STATE-carrying element so extraction types `.state`.

Deviation from the doc: the no-`RES` element is rejected with a **parse**
diagnostic ("A `STATE` clause requires a `RES` collection element…") rather than
by reaching `TYPE_RESOURCE_REQUIRES_RES`. This is clearer and satisfies the
requirement (stays rejected, no confusing dangling token). The bare no-`RES`
no-`STATE` form is still rejected by `TYPE_RESOURCE_REQUIRES_RES` unchanged.

Proof:
- rt-behavior fixture `bug427_list_union_state_rt` builds a
  `List OF RES Handle STATE Cursor`, mutates each element's `.state`, appends the
  stateful union resources, then extracts via `FOR EACH` and `collections::get`
  and reads `.state` back — full native build + run prints `total=12` / `first=3`.
- Extraction STATE mismatch (`RES x AS Handle STATE WrongState = get(...)`)
  correctly fires `TYPE_STATE_MISMATCH`.
- Full suite `cargo test --bin mfb`: 3783 passed, 0 failed. Artifact gate (`all`)
  has 2 diffs, both **pre-existing on clean main** (`.mir` vreg rename in
  `control-flow-if`/`parser-hello-world`, proven via a detached `300b2a2f8`
  build); this fix adds zero new golden diffs.
