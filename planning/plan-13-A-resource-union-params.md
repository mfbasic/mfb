# plan-13-A: resource-union parameters (the language amendment)

Last updated: 2026-07-20
Effort: small (<1h)
Depends on: nothing. **This is plan-13's gate — every other unit is behind it.**
Produces: the `15_resource-management.md` amendment, and all three checkers accepting
variant→union widening in a non-owning-parameter position. Consumed by 13-C and everything
after it.

The spec forbids what `app::` needs. `15_resource-management.md:30` says, verbatim:

> *"A resource value may be passed only to a function whose parameter is declared `RES`
> and explicitly names that **concrete** resource type… There is no generic resource
> supertype, no structural matching of handles, and no implicit conversion between
> resource types."*

`app::setVisible(w AS RES app::Widget)` is exactly that. So `app::` requires a
**deliberate, specified language change** — and this sub-plan is it, landed on its own,
before any GUI code exists.

The single behavioral outcome: a user-declared `UNION Stream { File Socket }` can be
passed a `File` in a `RES s AS Stream` **parameter** on all three checker paths, while
`fs::close(s AS Stream)` remains a compile error — proving the widening is directional.

References (read first):

- `src/docs/spec/language/15_resource-management.md:30` — the sentence to amend.
- `src/syntaxcheck/types.rs:117` (`compatible`) — already implements variant→union
  subsumption for *bindings*; this opens the parameter position only.
- `src/syntaxcheck/resources.rs:4` (`is_resource_type`) — resource args are
  `ExprMode::Use`.
- `src/syntaxcheck/builtins.rs:426` (`check_term_builtin_call`), `:864`
  (`normalize_builtin_call_arguments`) — checker 1.
- `src/ir/verify/mod.rs:4343` (`compatible`) — checker 3.
- `tests/rt-behavior/resources/resource-union-valid` — the existing `File→Stream` test,
  which exercises a **binding initializer**, never a call site.

## Prerequisites

None. This unit blocks on nothing and is the feature's entry gate.

> **NOTE — verify before you continue and again before you decide to stop.** The
> citations above are dated 2026-07-20; the master §2.3 records that this family's
> previous "verified" line numbers rotted by up to 932 lines in eleven days. Re-locate by
> symbol (`rg -n 'fn compatible'`), not by line, and correct this document as you go.

## 1. Goal

- A `RES` parameter may name a **resource union**; an actual of any variant widens to it.
- Widening is **variant→union only**, in **non-owning position only**, and is
  **representation-neutral** — the argument lowers to the same single handle either way,
  because a resource value already carries its own kind.
- The reverse is still rejected: a union actual into a concrete parameter is a compile
  error, so every **registered close op**, `thread::transfer` and `thread::accept` keeps
  its concrete-typed parameter and is unaffected. **No blocklist, no exemption table.**
- All three checkers agree; none can be made to disagree by a test.

### Non-goals (explicit constraints)

- **No `app::` code.** This unit ships a language capability and its tests. If it needs to
  mention `app::`, the split is wrong.
- **No change to binding, returning, or consuming a union.** `RES w AS app::Widget = …`,
  `AS RES <Union>` returns, and union-typed locals already work; only the non-owning-parameter
  position opens.
- **No structural matching, no generic supertype.** The amendment is narrow and
  directional. The spec sentence's other two prohibitions stay.
- **Do not touch `compatible()`'s binding behavior** (`syntaxcheck/types.rs:117`) — it is
  already correct and is the thing being reused.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Checkers that must learn the rule | **3** | §2.2 |
| Existing tests passing a concrete resource into a union **parameter** | **0** | `rg -rn 'RES .* AS ' tests/ \| rg -i union` — the one union test is a binding initializer |
| `compatible` implementations in the tree | 2 (`syntaxcheck/types.rs:117`, `ir/verify/mod.rs:4343`) | `rg -n 'fn compatible' src/` |
| Registered close ops that must keep concrete params | **all of them, none needing a blocklist** | close ops are dispatched via the resource registry's `close_function` (`syntaxcheck/types.rs:391`); directionality (a union actual is rejected by a concrete `RES File` param) keeps every one concrete-typed automatically — proven by `resource-union-param-invalid`'s `fs::close(Stream)` rejection, no exemption table |
| Existing tests passing a concrete resource into a union **parameter** | **now 1** (was 0) | `tests/syntax/resources/resource-union-param-valid` (added by this unit) |

### 2.2 The three checkers

The 2026-07-09 draft established this and corrected an earlier draft that had called it
"a global fix at the `term::` seam". That correction stands:

| # | Where | Change |
|---|---|---|
| 1 | `src/syntaxcheck/builtins.rs` | generalize `param_types` from one flat list to a **per-overload table**; select the overload whose params are all `expression_compatible()` with the actuals. Note `term::` infers args in `ExprMode::Read`; a resource param must use `ExprMode::Use` |
| 2 | the package's `resolve_call` | called from `src/ir/lower.rs` with **no access to the type registry**; every package does context-free `exact()` string matching (`src/builtins/net.rs`). It cannot know `app.Button` is a variant of `app.Widget`. Fix: a package-local static variant table + a `widget_or(name)` predicate, with a `#[test]` pinning it against the registered union |
| 3 | `src/ir/verify/mod.rs:4343` | per plan-20 the sole rejecter on both paths; its own `compatible()` must accept the same widening |

Checker 2 does not exist yet for `app::` (there is no `src/builtins/app.rs`). **This unit
implements the mechanism generically and 13-C supplies `app::`'s variant table** — so this
sub-plan is testable against a *user-declared* union with no GUI code anywhere.

### 2.3 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| The spec forbids resource-union parameters | **CONFIRMED** | `15_resource-management.md:30`, verbatim |
| `compatible()` already does variant→union subsumption for bindings | **CONFIRMED** | `syntaxcheck/types.rs:117` |
| No existing test covers the parameter position | **CONFIRMED** | `resource-union-valid` is a binding initializer (`RES s AS Stream = fs::createTempFile()`) |
| Resource args are already use-mode | **CONFIRMED** | `is_resource_type` ⇒ `ExprMode::Use`, `syntaxcheck/resources.rs:4`. The only consuming modes are a close op's first arg and `thread::transfer` |
| Widening is representation-neutral | **CONFIRMED by construction** | a bound union carries a tag written from the statically-known initializer type; a *parameter* has no such site, and the handle already carries its kind — so no tagged temporary is materialized |
| Directionality holds without a blocklist | **CONFIRMED** | `resource-union-param-invalid` rejects both a union→concrete-param call and a union→close-op call (`TYPE_CALL_ARGUMENT_MISMATCH`), with no blocklist; `resource-union-param-valid` accepts the widen. Both proven by fixture, not reasoning. |
| The **checker code** already satisfies the accept/reject matrix for a user-declared union parameter | **CONFIRMED (divergence from plan)** | `mfb build -ast -ir`: `File → RES s AS Stream` param → exit 0; `Stream → RES f AS File` param and `fs::close(Stream)` → rejected. Both `compatible()`s already strip `RES` + subsume variant→union and are consulted at the parameter position (`inference.rs:1265`, `calls.rs:129`). The plan's "parameter position short-circuits on an exact-name check" was false for user functions. See Corrections. |

## 3. Design Overview

One rule, three places, one direction.

`compatible()` already answers "is this actual acceptable for this expected type" with
union subsumption. The amendment is to **let that answer be reached from a parameter
position**, which today short-circuits on an exact-name check.

**Where design uncertainty concentrates:** nowhere. The subsumption logic exists and is
tested for bindings; this widens where it is consulted.

**Where correctness risk concentrates:** **directionality.** If the widening is
accidentally made symmetric, a union actual becomes acceptable to a concrete parameter —
and the first casualty is a **close op**, which would then be handed a handle whose real
type it cannot know. That is a use-after-free class bug, not a type-checker inconvenience.
The `-invalid` fixture is the guard and is not optional.

**Rejected alternative:** *an exemption table listing the ops that must keep concrete
parameters* (close ops, `thread::transfer`, `thread::accept`). Rejected: `compatible()`
already enforces the direction, so a table would be a second source of truth that can
drift. If the direction is right, no table is needed — and if a table seems necessary, the
direction is wrong.

**Rejected alternative:** *materialize a tagged temporary per call.* Rejected as pure
waste — the handle already carries its kind, so widening changes nothing at runtime.

**Rejected alternative:** *fold this into 13-C.* Rejected: a language spec change inside a
GUI package's commit series is unreviewable, and it would make `app::` the reason the
language changed rather than a consumer of a change that stands on its own.

## Compatibility / Format Impact

- **Changed:** `15_resource-management.md` gains the amendment; three checkers accept
  variant→union widening in non-owning-parameter position.
- **Unchanged:** every existing program's behavior (this only *accepts* more); binding,
  returning and consuming a union; close ops, `thread::transfer`/`accept`; the emitted
  representation of any resource argument.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — the spec amendment

Land the specified rule before the code, so the code has something to be checked against.

- [x] Amend `src/docs/spec/language/15_resource-management.md:34` (was `:30` — citation
      rotted, see Corrections): a `RES` parameter may name a resource union; widening is
      variant→union only, non-owning-position only, and representation-neutral. States
      explicitly that the reverse stays rejected (`TYPE_CALL_ARGUMENT_MISMATCH`) and that
      close ops / `thread::transfer` / `thread::accept` are therefore unaffected. A pointer
      sentence was also added to the **Resource unions** paragraph (`:125`).

Acceptance: the spec states the rule and its directionality; `mfb spec language
resource-management` renders it (verified against the rebuilt release binary).
Commit: 5564c75b6

### Phase 2 — the three checkers

- [x] ~~Checker 1 (`src/syntaxcheck/builtins.rs`): per-overload `param_types` table,
      selected by `expression_compatible()`, with `ExprMode::Use` for resource args.~~ —
      moot in 13-A: this is the **builtin** `term::`-family arg path
      (`check_term_builtin_call`, `builtins.rs:257`). It has no resource-union consumer
      until an `app::` builtin declares a `RES w AS app::Widget` parameter, so it is
      untestable here and belongs to 13-C. The **testable** parameter position for a
      user-declared union — a user function's `RES s AS Stream` param — is checked at
      `syntaxcheck/inference.rs:1265` via `expression_compatible → compatible`, which
      already subsumes variant→union and strips `RES`. **Measured**: `mfb build -ast -ir`
      on a `File → RES s AS Stream` call → exit 0 (accepted). Pinned by
      `resource_union_variant_widens_into_union_param` + the two directional-reject tests
      (`syntaxcheck/resources.rs`).
- [x] ~~Checker 2: the generic variant-predicate mechanism a package's `resolve_call` uses
      (the `app::` table itself lands in 13-C).~~ — moot in 13-A: a package `resolve_call`
      is a **builtin-only** seam; a user function has none, so a user-declared union never
      traverses it. Building a generic variant-predicate with zero consumer and zero test
      here would be exactly the "consumed by a later phase" dead code AGENTS.md forbids. It
      lands with `app::`'s variant table in 13-C.
- [x] Checker 3 (`src/ir/verify/compat.rs:324`, was `ir/verify/mod.rs:4343` — the file was
      split, see Corrections): its `compatible()` already accepts the same widening (union
      arm at `:355`, `RES` stripped at `:328`) and is reached for user-function call args
      at `calls.rs:129`. **Measured**: the valid case passes the `-ir` verify pass. Pinned
      by `resource_union_parameter_widening_is_directional` (`ir/verify/tests.rs`).

Acceptance: every checker path a **user-declared** `UNION Stream { File Socket }` in a
parameter position can traverse (syntaxcheck `inference.rs` + `ir::verify` `calls.rs`)
accepts the variant; both reverse directions are rejected — no `app::` code involved. The
builtin-only paths (checkers 1–2) carry no resource-union consumer until 13-C and are
deferred there.
Commit: 5564c75b6

### Phase 3 — directionality proof (the load-bearing one)

- [x] `tests/syntax/resources/resource-union-param-valid` — a `File` into a
      `RES s AS Stream` user-function parameter; builds clean (`-ast -ir`, exit 0), so
      accepted by both the syntaxcheck and `ir::verify` paths. `.ast` + `.ir` goldens
      captured.
- [x] `tests/syntax/resources/resource-union-param-invalid` — a `Stream` actual into a
      concrete `RES f AS File` parameter, **rejected** (`TYPE_CALL_ARGUMENT_MISMATCH`); and
      `fs::close(s AS Stream)` handed a union, **rejected**. Both fire in one build.log.
- [x] Confirmed the existing resource suite is unchanged: `scripts/test-accept.sh` over
      `syntax/resources/*` → 45 tests pass (43 pre-existing + 2 new).

Acceptance: the valid fixture passes on every applicable checker path **and** both invalid
cases are rejected. A passing positive test alone proves nothing here — symmetric widening
also passes it; the two `-invalid` cases are the guard. (The third "checker path", a
package `resolve_call`, is builtin-only and not reachable from a user-declared union — see
Phase 2 corrections; it is 13-C's to prove for `app::`.)
Commit: 5564c75b6

## Validation Plan

- Tests: the valid/invalid pair above. The invalid half is the real test; it is what
  distinguishes a directional widening from a broken one.
- Coverage check: `tests/syntax/resources/` is golden-backed and in the gate's
  denominator. `tests/acceptance/` has **no** `golden/` dir by design — do not put the
  proof there.
- Runtime proof: none applicable — this is a type-checker capability with no runtime
  behavior. Its proof is the accept/reject matrix.
- Doc sync: `15_resource-management.md` (Phase 1, in the same change as the code).
- Acceptance: the project's full suite, with the existing resource fixtures unchanged.

## Open Decisions

1. **Whether the amendment should also permit union *returns* by widening.** Recommended
   **no** — returns already work by explicit declaration, and widening a return would let
   a function's concrete type be erased at the boundary, which is the direction that
   breaks close ops. Keep the change to the parameter position only.
2. **Whether checker 2's variant table should be generated or hand-written.** Recommended
   hand-written per package with a `#[test]` pinning it against the registered union — a
   generated table needs registry access at a point that deliberately has none.

## Corrections

<!-- Filled in during execution. -->

- 2026-08-02 — **The checker code already accepted the widening; the plan's premise that
  the parameter position needs a code change was false for user functions.** §3 claimed the
  parameter position "today short-circuits on an exact-name check." Measured otherwise: a
  user-function call's arguments are checked at `syntaxcheck/inference.rs:1265` and
  `ir/verify/calls.rs:129`, both via `expression_compatible → compatible`, and **both
  `compatible()` implementations already strip the `RES` marker and subsume a variant into
  its union** (`syntaxcheck/types.rs:197`, `ir/verify/compat.rs:355`). So `mfb build -ast
  -ir` on a `File → RES s AS Stream` parameter already exits 0, and `Stream → RES f AS
  File` / `fs::close(Stream)` are already rejected. The code was simply *more permissive
  than the spec*. 13-A's real deltas are therefore (1) the spec amendment (Phase 1) and
  (2) the directionality fixtures + pinning unit tests (Phases 2–3) — not a checker
  rewrite. The **core premise still holds** (the spec forbade it → amended; `app::`'s
  builtin path genuinely can't widen yet → 13-C), so this is a §4 divergence, not a
  falsified premise.
- 2026-08-02 — **Phase 2's checker-1 and checker-2 items are moot in 13-A and deferred to
  13-C.** Checker 1 (`builtins.rs` per-overload `param_types`) is the **builtin** `term::`
  arg path; checker 2 (a package `resolve_call` variant predicate) is a **builtin-only**
  seam. Neither is traversed by a *user-declared* union parameter, and neither has a
  resource-union consumer until an `app::` builtin declares `RES w AS app::Widget`.
  Building generic scaffolding with no consumer and no test here is the "consumed by a
  later phase" dead code AGENTS.md forbids. Marked moot with evidence; the widening for the
  testable (user-function) surface is proven to already work and is pinned by three
  `syntaxcheck` tests + one `ir::verify` test. Checker 3 (`ir::verify`) already accepts and
  is ticked.
- 2026-08-02 — **Citations rotted further since 2026-07-20** (re-derived by symbol, not
  line): `check_term_builtin_call` `:426`→**`builtins.rs:257`**;
  `normalize_builtin_call_arguments` `:864`→**`:699`**; the `15_resource-management.md`
  sentence `:30`→**`:34`**; `compatible()` `types.rs:117`→**`:122`**; and **checker 3's
  `compatible()` moved out of `ir/verify/mod.rs:4343` into its own file
  `ir/verify/compat.rs:324`** (the module was split). `is_resource_type` remains at
  `resources.rs:4`.
- 2026-08-02 — **`src/builtins/app.rs` now exists** (plan-62 landed the `app::` package
  skeleton + `app_package.mfb`). It does not change 13-A's scope — 13-A is only the
  language amendment — but it confirms 13-C's checker-2 work has a real file to land in.
- 2026-07-20 — **Promoted from "plan-13-C Phase 0" to its own gating unit.** A language
  spec change that every other unit depends on is a precondition, not phase zero of an
  x-large document.
- 2026-07-20 — Citations re-derived. The 2026-07-09 draft's numbers had rotted:
  `check_term_builtin_call` `:879`→`:426`, `normalize_builtin_call_arguments`
  `:1701`→`:864`, `ir/verify::compatible` `:3411`→`:4343`, `compatible()`
  `types.rs:145-170`→`types.rs:117`, and `is_resource_type` is in
  **`syntaxcheck/resources.rs:4`**, not `types.rs:328`.

## Summary

The engineering risk is a single bit: direction. Symmetric widening would let a union
actual reach a concrete parameter, and the first thing that breaks is a close op handed a
handle whose type it cannot determine — a use-after-free, not a type error. The
`-invalid` fixture is the only thing that distinguishes correct from broken, which is why
it is its own phase.

Everything else here is small: the subsumption logic already exists and is already tested
for bindings; this widens where it is consulted, in one direction, in one position.

What is left untouched: binding/returning/consuming unions, every close op,
`thread::transfer`/`accept`, the emitted representation of resource arguments, and every
existing program's behavior.
