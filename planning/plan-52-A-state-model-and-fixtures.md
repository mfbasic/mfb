# plan-52-A: the resource STATE model — spec + failing fixtures

Last updated: 2026-07-16
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing

Writes the resource `STATE` visibility model into the spec and pins every rule with a
fixture. **No behavior change** — the invalid-program fixtures land failing (they build
clean today), and the valid ones land as regression guards for what already works.

`STATE` appears in four positions — `RESOURCE` declaration, binding, parameter, return —
and the language has never stated what `RES x AS SfFile` versus `RES x AS SfFile STATE
FileInfo` *means* in each. The compiler's behavior is therefore incidental: two positions
are right, one is right for the wrong reason, one is wrong two ways. This sub-plan makes
the model explicit so B/C/D implement a written contract instead of re-deriving it.

The single outcome: **`./mfb spec language resource-management` §15 states the model
table below, and one fixture exists per row, each passing or failing for a documented
reason.**

References:

- `./mfb spec language resource-management` §15 — documents STATE on bindings only;
  silent on all four variance questions. The doc this sub-plan edits.
- `planning/res.md` §2 — the verified behavior table every row below rests on; §6 — the
  reasoning errors made while deriving this model, kept so they are not repeated.
  **Read first.**
- `planning/res.md` §1/§3 — the wider ownership question (Track B). **Out of scope here.**

## 1. Goal

- §15 states the model table (§3) — the four positions, both spellings, with the
  escape rule that justifies the asymmetry.
- One fixture per row of §3's table, under `tests/syntax/resources/` and
  `tests/rt-behavior/resources/`.
- The three invalid rows fail today by **building clean** (documented as pending, to be
  flipped by C and D).
- The four valid rows pass today and are pinned as regression guards.

### Non-goals (explicit constraints)

- **No behavior change.** Not one compiler rule moves in this sub-plan. Goldens must not
  shift; `scripts/artifact-gate.sh` delta must be nil.
- **`STATE` on the `RESOURCE` declaration.** Decided against (user, 2026-07-16). It
  would give one declaration site and make disagreement unrepresentable — killing the
  whole mismatch class for that resource — but forfeits the bare-param opt-out that close
  ops depend on. Decided; do not re-litigate. Revisit only if the restatement burden
  (§Open Decisions) proves worse than the opt-out is worth.
- **The `" STATE "` type-string encoding.** A structured representation is a separate
  plan.
- **Track B (resource-scoped ownership).** `planning/res.md` §1/§3. This plan assumes the
  current borrow rule, and §3's table is sound *because* of it.
- **The borrow rule itself** (`TYPE_RESOURCE_BORROW_INVALIDATE`). A dependency, not
  collateral.

## 2. Current State

`STATE` rides **inside the type string** — `"File STATE Cursor"` — recovered by
`crate::builtins::resource::state_type_name` (`src/builtins/resource.rs:231-233`, a
literal `split_once(" STATE ")`), with `base_resource_name` stripping it for resource
recognition. Every stage reads it back out of that string.

Three sites build a type string; two append the STATE:

- `src/ir/lower.rs:739-742` — **params**, appends.
- `src/ir/lower.rs:974-977` — **bindings**, appends.
- `src/ir/lower.rs:724-730` — **returns**, **does not**. Also `:1963-1970` (the `returns`
  map feeding `expression_type` for calls) and `:220-224` (imported functions, via
  `function_return_from_type`, `:2509-2515`).

`function.return_state_type` is read **nowhere** in `ir/` or `target/`. Its only
consumers are `src/syntaxcheck/mod.rs:2041` (→ `check_resource_declaration`,
`src/syntaxcheck/checking.rs:74-87` — a `check_type_reference` existence check, nothing
more) and `src/ast/serialize.rs:710` (a one-way AST→JSON dump).

The STATE verify rules (`src/ir/verify/mod.rs:825-845`) gate on
`type_.find(" STATE ")` over a **binding's** type string, so they are unreachable from a
return. `check_return_type` (`:3895-3909`) compares the returned value's inferred type
against `current_return`.

`resourceDecl` has **no STATE clause** (`src/ast/items.rs:540-567`; `ResourceDecl` is
`{visibility, name, close_fn, thread_sendable, line}`, `src/ast/types.rs:246-254`).

**Existing precedent to mirror:** `tests/syntax/resources/resource-state-invalid` and
`resource-union-state-invalid` pin `TYPE_STATE_INVALID` / `TYPE_UNION_STATE_FORBIDDEN` on
a **binding**. This sub-plan adds their return-position twins.

All 14 `STATE` uses under `tests/` are bindings and params, all agreeing. **None**
exercises a return, a disagreement, or a param-attach. That is why this shipped.

## 3. Design Overview

`RESOURCE SfFile CLOSE BY …` carries no STATE. The model:

| Position | `RES x AS SfFile` | `RES x AS SfFile STATE FileInfo` |
|---|---|---|
| **Param** | any state or none; `.state` **not** accessible | **only** a SfFile carrying FileInfo; `.state` accessible |
| **Return** | a resource with **no** state | a resource **carrying** a FileInfo |
| **Binding** | **no** state | attaches (if none) / adopts a FileInfo |

Bare means **"opaque"** as a param and **"none"** everywhere else — two readings, one
spelling. Sound **only because a borrow cannot escape**: `TYPE_RESOURCE_BORROW_INVALIDATE`
confines the opaque reading to the frame that borrowed it (verified — res.md §2 fact #9).

**The rule to write into §15:** bare erases state only where the resource *cannot
escape* (params). Where it can escape (bindings, returns), bare means **provably no
state**. Attachment happens **exactly once, at the owning binding**; params only observe.

Correctness risk here is **zero** (no behavior changes). The risk is *editorial*: if §15's
table is wrong, C and D implement the wrong thing. The escape distinction is the subtle
part, and it was got backwards twice during this plan's research — see `res.md` §6. Write
it once, explicitly.

Rejected: making a bare param mean "stateless only" (uniform across positions, kills the
laundering) — it breaks every close op, which must accept the resource whatever state it
carries. The asymmetry is the design.

## 4. The fixture matrix

| # | Case | Model | Today | Fixture |
|---|---|---|---|---|
| 1 | bare param ← stateful arg | accept, `.state` inaccessible | ✓ works | guard (rt-behavior) |
| 2 | `.state` **write** via bare param | reject | ✓ `TYPE_STATE_INVALID` | guard (syntax) |
| 3 | `.state` **read** via bare param | reject | ✓ but degrades to `Unknown`; error lands on the consumer | guard + **plan-52-C** fixes the message |
| 4 | stateful param ← **stateless** arg | **reject** | ✗ **attaches** | **pending → C** |
| 5 | stateful param ← different STATE | **reject** | ✗ **type confusion** | **pending → C** |
| 6 | bare return ← stateful value | reject | ✓ but *by accident* — the return type string drops STATE, so ALL stateful returns are rejected | guard → D re-earns it |
| 7 | stateful return ← stateful value | **accept** | ✗ `TYPE_RETURN_MISMATCH` | **pending → D** |
| 8 | union-STATE / non-defaultable STATE on a **return** | reject | ✗ **both compile** | **pending → D** |
| 9 | bare **binding** ← stateful value | **reject** | unreachable — nothing can produce a stateful resource from a call yet | **pending → D** (lands *with* D, never after) |
| 10 | `RETURN` of a borrow | reject | ✓ `TYPE_RESOURCE_BORROW_INVALIDATE` | guard — the model depends on it |

Rows 4/5/7/8/9 land **failing**. A "pending" fixture fails by *not* erroring; record the
expected code in its golden so C/D flip it by producing the diagnostic.

## Compatibility / Format Impact

None. No API, wire format, or layout changes. `.mfp` untouched. Documentation only, plus
test fixtures.

## Phases

### Phase 1 — the spec

Delivers the written contract. Safe alone: prose only.

- [ ] Add the §3 table to `src/docs/spec/language/15_resource-management.md`, with the
      escape rule and the "attach only at the owning binding" statement.
- [ ] State that `RESOURCE` carries no STATE and *why* (§Non-goals above).
- [ ] Cross-reference `./mfb spec architecture escape-analysis` for the borrow rule the
      asymmetry depends on.
- [ ] Per `.ai/specifications.md`, keep the embedded spec current — verify `mfb spec
      language resource-management` renders.

Acceptance: `mfb spec language resource-management` renders the table; a reader can
answer all four positions × two spellings without reading compiler source.
Commit: —

### Phase 2 — guard fixtures (rows 1/2/3/6/10)

Pins what already works, so C and D cannot silently regress it.

- [ ] `tests/rt-behavior/resources/` — row 1 (bare param ← stateful; assert the owner's
      state survives, e.g. `pos still = 42`).
- [ ] `tests/syntax/resources/` — rows 2, 3, 6, 10, each asserting today's actual
      diagnostic.
- [ ] Row 3's golden records the *current* poor error (`TYPE_CALL_ARGUMENT_MISMATCH` on
      the consumer) with a `TODO(plan-52-C)` noting it should name STATE.

Acceptance: all five pass; `scripts/artifact-gate.sh` delta is nil.
Commit: —

### Phase 3 — pending fixtures (rows 4/5/7/8/9) + audit

The failing half. Safe alone: fixtures only, no rules move.

- [ ] `tests/syntax/resources/resource-state-param-mismatch-invalid/` (row 5) and
      `resource-state-param-attach-invalid/` (row 4).
- [ ] `tests/rt-behavior/resources/` — the two-disagreeing-borrows runtime proof: allocate
      as `Cursor{pos:Integer}=42`, read as `Label{name:String}`; observe the wrong-type
      read. **Runtime proof required** — the build succeeds, so only running shows it.
- [ ] `tests/syntax/resources/resource-return-*-invalid/` (row 8) and
      `tests/rt-behavior/resources/resource-state-return-rt/` (row 7).
- [ ] Row 9 as a pinned-pending case (unreachable until D).
- [ ] **Audit:** confirm no in-tree fixture depends on param-attach (initial read: none —
      `resource-state-field-assign-valid`'s *owner* declares the STATE, so its param never
      allocates). Write the verdict into plan-52-C §3, which assumes it.
- [ ] **Audit:** does the STATE survive an exported signature in `.mfp`?
      `function_return_from_type` (`src/ir/lower.rs:2509-2515`) splits on `") AS "`, so
      `"FUNC(String) AS File STATE Cursor"` *would* round-trip textually — confirm the
      writer emits it. **Gates plan-52-D**; if it doesn't, libsnd stays blocked.
- [ ] **Audit:** `resolver/mod.rs:599` sets `return_state_type: None` for re-exports —
      does `FUNC alias AS pkg::openTagged` drop the STATE?
- [ ] **Audit:** `thread::transfer` (`builder_arena_transfer.rs:336-337`) moves the state
      pointer without consulting either type string — can an `ISOLATED FUNC` entry declare
      a different STATE than the sender's binding? Consumed by plan-52-C §Open Decisions.

Acceptance: each pending fixture fails for its documented reason; every audit item has a
written verdict in the sub-plan that consumes it (C §3, D §4).
Commit: —

## Validation Plan

- Tests: the 10 fixtures above, `tests/syntax/resources/` + `tests/rt-behavior/resources/`,
  per the tests-reorg convention (fixtures by name, `testutil::fixture_dir`).
- Runtime proof: row 1 and the two-disagreeing-borrows case. Per `.ai/compiler.md`'s
  runtime completion gate, a build assertion cannot distinguish "attached correctly" from
  "aliased the wrong payload".
- Doc sync: `src/docs/spec/language/15_resource-management.md` — Phase 1's deliverable,
  not an afterthought (`.ai/specifications.md`).
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh` (delta must be **nil**
  — no rule moves here), `cargo test --bin mfb`.

## Open Decisions

- **Owner-side opt-out.** Under §3 there is none: a bare binding *asserts* "no state", so
  an owner wanting only the handle must still write `STATE FileInfo`. Recommend accepting
  the restatement burden; `STATE _` ("opaque", permitted on owners, forbidden in a return)
  stays in reserve. This is the cost the rejected RESOURCE-level design would have avoided.
- **Return-type overload identity.** A callable's identity includes its return type (§6).
  Recommend treating `File` and `File STATE Cursor` as the **same** return type for
  overload purposes, so STATE never becomes a discriminator. Confirm nothing shifts in D.
- **`thread::transfer`** — Phase 3's audit. If the resource plane admits a state-type
  disagreement that is a cross-thread type confusion and may warrant its own severity.

## Summary

Pure documentation and fixtures — zero behavior change, zero golden churn. The risk is
entirely in getting §3's table right, because C and D implement it verbatim. The one idea
that must survive: **params may erase state (they cannot escape); owners may not.** That
distinction was got backwards twice while deriving this model (`res.md` §6), so it is
written into §15 explicitly rather than left to be re-derived.
