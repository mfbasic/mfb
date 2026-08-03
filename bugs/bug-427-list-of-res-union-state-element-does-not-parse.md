# bug-427: `List OF RES <ResourceUnion> STATE <S>` element type does not parse

Last updated: 2026-08-03
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/syntax/ (new) — list-of-res-union-state parse fixture

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

- [ ] Add a `tests/syntax/` parse fixture for
      `MUT streams AS List OF RES <ResourceUnion> STATE <S>` (and a `Map OF K TO
      RES V STATE S` sibling). Confirm they fail today with the dangling-`STATE`
      statement-end error.
- [ ] Add a negative fixture asserting `List OF http::Stream STATE …` (no
      `RES`) stays rejected.
- [ ] Run the actual `strip_prefix("RES ")` / `List OF RES` / `TO RES` search
      and write each site's verdict into Blast Radius above.

Acceptance: the new positive fixtures fail for the documented reason; the
negative fixture already fails (stays rejected); the audit list is complete with
a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] `src/ast/expr.rs` List branch: fold `parse_optional_state` into the `RES`
      element (mirror `parse_resource_plane_type`).
- [ ] `src/ast/expr.rs` Map value branch: same fold after the value's optional
      `RES`.
- [ ] Reject `STATE` after a non-`RES` collection element with a clear
      diagnostic.
- [ ] Extend every in-scope consumer from the audit to strip/carry ` STATE T`
      on a collection element (resolver, monomorphizer, source checker, IR
      verifier).
- [ ] Element extraction (`get` / `FOR EACH`) + `RES`-bind checks STATE
      agreement (`TYPE_STATE_MISMATCH`).

Acceptance: Phase 1 positive fixtures pass; the negative fixture still rejects;
Non-goals unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any `.ast` / `.ir` goldens the new fixtures produce; diff and
      confirm the delta is only the new type string.
- [ ] Update `mfb spec architecture type-name-encoding` (`ListArg`) and
      `mfb spec language resource-management` §15.6 to document the
      STATE-bearing collection element.
- [ ] Run the full `cargo test --bin mfb` suite and the artifact gate.

Acceptance: full suite green; golden deltas are exactly the intended change; the
reproduction parses and type-checks.
Commit: —

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
