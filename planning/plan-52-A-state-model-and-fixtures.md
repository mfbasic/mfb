# plan-52-A: the resource STATE model — spec + failing fixtures

Status: **COMPLETE.** §15.5 written; 11 fixtures landed (guards passing, pending failing
for their documented reasons); all four audits have written verdicts. Two audits changed
their consuming sub-plans materially: the `.mfp` one **unblocked** plan-52-D (no format
change needed), and the `thread::transfer` one produced `bugs/bug-257` — a hole plan-52
does not close. A third finding, `bugs/bug-256`, was fixed en route: a `STATE` with a
`String` field could not be built at all, which had been misattributed in plan-52-C §2 and
was blocking the disclosure question (now resolved: **not** a disclosure primitive).

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

- [x] Add the §3 table to `src/docs/spec/language/15_resource-management.md`, with the
      escape rule and the "attach only at the owning binding" statement. — landed as
      **§15.5 "What `STATE` means in each position"**.
- [x] State that `RESOURCE` carries no STATE and *why* (§Non-goals above).
- [x] Cross-reference `./mfb spec architecture escape-analysis` for the borrow rule the
      asymmetry depends on.
- [x] Per `.ai/specifications.md`, keep the embedded spec current — verified
      `mfb spec language resource-management` renders the table with no leaked `[[ ]]`.

Acceptance: `mfb spec language resource-management` renders the table; a reader can
answer all four positions × two spellings without reading compiler source.
Commit: —

### Phase 2 — guard fixtures (rows 1/2/3/6/10)

Pins what already works, so C and D cannot silently regress it.

- [x] `tests/rt-behavior/resources/resource-state-bare-param-valid` — row 1 (bare param ←
      stateful; the owner's state survives the borrow: prints `42`).
- [x] `tests/syntax/resources/` — rows 2, 3, 6, 10, each asserting today's actual
      diagnostic:
      - row 2 `resource-state-bare-param-write-invalid` → `TYPE_STATE_INVALID` ✓
      - row 3 `resource-state-bare-param-read-invalid` → `TYPE_CALL_ARGUMENT_MISMATCH` ✓
      - row 6 `resource-state-bare-return-invalid` → `TYPE_RETURN_MISMATCH` ✓ (accidental)
      - row 10 `resource-borrow-return-invalid` → `TYPE_RESOURCE_BORROW_INVALIDATE` ✓
- [x] Row 3's golden records the *current* poor error (`TYPE_CALL_ARGUMENT_MISMATCH` on
      `io.print`, argument type `Unknown`). The `TODO(plan-52-C)` lives in the fixture's
      source header — `build.log` is exact-compared generated output and cannot carry one.

Acceptance: all five pass. Full suite: **977 tests, 0 real mismatches**; the only
failures were the pending fixtures' un-goldened artifacts (see Phase 3).
Commit: —

### Phase 3 — pending fixtures (rows 4/5/7/8/9) + audit

The failing half. Safe alone: fixtures only, no rules move.

- [x] `tests/syntax/resources/resource-state-param-mismatch-invalid/` (row 5) and
      `resource-state-param-attach-invalid/` (row 4). Both **build clean today**, as the
      matrix predicts.
- [x] **Runtime proof (row 5), observed:** the two-disagreeing-borrows program builds and
      misreads its payload — `Cursor{pos:Integer}=42` read through a `STATE Label`
      parameter dies with a bogus `Write or flush operation failed`. Row 4 observed
      **attaching** to a stateless owner (prints `7`). Both on fresh binaries (`rm -rf
      build`, timestamp-checked — the stale-`build/*.out` trap is real).
      **Not kept as a fixture**: plan-52-C makes both programs compile errors, so the
      permanent artifacts are the two syntax fixtures above; an rt-behavior fixture would
      have to be deleted in the same session it was written.
- [x] `tests/syntax/resources/resource-return-state-invalid/` +
      `resource-return-union-state-invalid/` (row 8 — both build clean today, confirming
      the verify rules never fire on a return) and
      `tests/rt-behavior/resources/resource-state-return-rt/` (row 7 — fails with exactly
      `TYPE_RETURN_MISMATCH`, the documented pending reason).
- [x] Row 9 `resource-state-bare-binding-invalid` as a pinned-pending case: today it fails
      at `RETURN f` with `TYPE_RETURN_MISMATCH` — i.e. unreachable, exactly as the matrix
      says — and flips to `TYPE_STATE_MISMATCH` on the bare binding once D lands.

**Note on the pending fixtures' goldens.** Rows 4/5/8 build today, so the harness emits
`.ast`/`.ir` actuals for them and reports "unexpected actual" against a golden set that
holds only `build.log`. That is the *intended* transient state: once C and D reject these
programs no artifacts are produced and the failures resolve themselves. Do not add
`.ast`/`.ir` goldens for them — that would have to be undone in the same session.
- [x] **Audit:** confirm no in-tree fixture depends on param-attach.
      **VERDICT: none.** Exactly two in-tree fixtures declare a STATE on a *param* —
      `resource-state-field-assign-valid` (`seek(RES s AS File STATE Cursor)`) and
      `resource-state-mutation-valid` (`advance(RES f AS File STATE FileState)`) — and in
      both the **owner** declares the same STATE, so the param never allocates. Every other
      STATE use in `tests/` is a binding. plan-52-C §3's assumption holds; rejecting
      param-attach breaks nothing in-tree.
- [x] **Audit:** does the STATE survive an exported signature in `.mfp`?
      **VERDICT: see plan-52-D §4** — resolved there against the writer, since that is the
      sub-plan it gates.
- [x] **Audit:** `resolver/mod.rs:599` sets `return_state_type: None` for re-exports —
      does `FUNC alias AS pkg::openTagged` drop the STATE?
      **VERDICT: the premise is wrong — not a re-export site.** `resolver/mod.rs:599` sits
      inside `#[cfg(test)] mod tests` (opened at `:547-548`), in the test helper
      `fn func(name, params) -> Function`. It is a fixture builder defaulting an unused
      field, and re-exports do not flow through it. The real re-export path is unaffected
      by it; plan-52-D Phase 3 confirms re-export behavior directly instead.
- [x] **Audit:** `thread::transfer` moves the state pointer without consulting either type
      string — can an `ISOLATED FUNC` entry declare a different STATE than the sender's
      binding? **VERDICT: yes — confirmed at runtime, filed as `bugs/bug-257`.** A sender
      attaching `Cursor{pos:Integer}=99` and a worker declaring `STATE Label{name:String}`
      build clean and type-confuse across the thread boundary (bogus `Allocation failed`).
      **plan-52-C and plan-52-D do not close it**: `thread::accept`'s static return type is
      a bare `File`, so the receiver's binding reads as a legal *attach* while
      `emit_resource_state_init`'s null-check silently **adopts** the sender's payload.
      Closing it needs the STATE on the plane type (a language-surface change) — its own
      plan, out of scope here. Recorded in plan-52-C §Open Decisions.

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
