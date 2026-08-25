# plan-106-C: Replace syntaxcheck's private Type enum with ParameterType

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-106-B (verify is typed; syntaxcheck is the last engine on a
private representation).

Delete the compiler's **sixth type representation**: `syntaxcheck`'s private
`enum Type` (`src/syntaxcheck/mod.rs:27` — scalars, `List`/`Set`/`Map`/`Res`/
`Function`/`Result`/`Thread`/`ThreadWorker`/`User(String)` + `Error`/`ErrorLoc`)
and its private 1,077-line parser (`src/syntaxcheck/types.rs`), replacing both
with `crate::types::ParameterType` and the canonical `parse`. Syntaxcheck is
already structurally typed *internally* — this is an enum **swap** plus a
parser deletion, not a 14k-line stringly rewrite; the discovery that makes
plan-106 tractable.

See plan-106-A for the roadmap, shared prerequisites, and the terminal
no-strings invariant.

References:

- `src/syntaxcheck/mod.rs:27-…` — the private `Type` enum (read the whole
  definition; note `Error`/`ErrorLoc` variants and
  `Thread(msg, res, res_state, out)`'s four-slot shape vs `ParameterType`'s
  `ThreadHandle`).
- `src/syntaxcheck/types.rs` (1,077 lines) — `parse_type`/
  `parse_collection_element_type`/`parse_function_type` + their unit tests: the
  private grammar this letter deletes.
- `src/syntaxcheck/inference.rs` (2,831 lines, 153 `Type` references) — the
  canonical AST checker engine, already enum-driven.
- `src/syntaxcheck/helpers.rs:272` — `numeric_binary_result_type(op, &Type,
  &Type) -> Type` (the sixth promotion copy; E deletes it onto numeric.rs once
  the enum matches).
- `src/types.rs` — `ParameterType` (post-105-B: includes `UserOf`).

## Prerequisites

See plan-106-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-106-B complete | verify typed; B's boxes ticked | **MET** 2026-08-24 — B's boxes all ticked; `TypeEnv` stores + `infer_type` are `ParameterType` |
| plan-105-B's `UserOf` exists | `rg -n 'UserOf' src/types.rs` → hit | **MET** 2026-08-24 — `src/types.rs:82` declares `UserOf(Symbol, Vec<ParameterType>)` |

## 1. Goal

- `enum Type` and `src/syntaxcheck/types.rs`'s parser **do not exist**;
  syntaxcheck's rule modules and `inference.rs` operate on `ParameterType`.
- Diagnostics are byte-identical (codes, wording, order) across the full
  `*-invalid` corpus — the checker's accept/reject set is unchanged.
- Where syntaxcheck's enum is richer or differently shaped than
  `ParameterType`, the mapping is explicit and recorded (see §3): `Error`/
  `ErrorLoc` map to the same `Named` spellings the rest of the compiler uses;
  the thread shape's `res_state` slot maps onto `ThreadHandle`'s existing
  RES/STATE modelling (the plane's STATE rides the res type exactly as
  `parse`/`name` already round-trip it).

### Non-goals (explicit constraints)

- No behavior change: same programs accepted/rejected, same diagnostics.
- Syntaxcheck still consumes the **AST** in this letter (parsing source-string
  type fields via the canonical `ParameterType::parse` where its private parser
  did) — the HIR input switch is D's scope. One representation change at a
  time.
- No rule relocation between syntaxcheck and `ir::verify`
  (`RELOCATED_TO_IR_VERIFY` untouched).

## 2. Current State

Syntaxcheck parses AST type strings ONCE at its own boundary
(`types.rs::parse_type`, scope-aware — it resolves user names against its
symbol tables, which is why it has `User(String)`), then runs its 124-rule
engine over its private enum. The enum is near-isomorphic to `ParameterType`;
the differences are enumerable and small.

### Measured populations

| What | Count | Command |
|---|---|---|
| private parser to delete | 1,077 lines | `wc -l src/syntaxcheck/types.rs` |
| `Type` references in the engine | 153 (inference.rs) | `rg -c '\bType\b' src/syntaxcheck/inference.rs` → 153; whole-module count at kickoff: `rg -c '\bType\b' src/syntaxcheck/` |
| syntaxcheck total | 14,441 lines | `find src/syntaxcheck -name '*.rs' \| xargs wc -l` |
| enum variant delta vs `ParameterType` | to enumerate | Phase 1 task: read both enums side-by-side; record the full mapping table here |
| `HashMap<String, String>` in syntaxcheck | 2 | `rg -c 'HashMap<String, String>' src/syntaxcheck/` → 2 (triage in Phase 2: type-valued vs name-valued) |
| diagnostic goldens guarding this | 124 rules / every `*-invalid` fixture | plan-102-F census |

### Verified properties

- **The enum is near-isomorphic** — read side-by-side at plan-writing:
  scalars match; `List/Set/Map/Res/Function/Result` match
  (`Function{params,return,isolated}` ≡ `Func(Vec,Box,bool)`); `User(String)` ≡
  post-105 `Named`/`UserOf`; `Thread(msg, res, res_state, out)` carries
  `res_state` separately where `ThreadHandle{worker,msg,res,out}` folds STATE
  into the res type's spelling. UNVERIFIED remainder: whether any syntaxcheck
  rule DISTINGUISHES `res` from `res_state` in a way the folded spelling
  cannot — Phase 1 reads every `Thread(` match arm and records the answer
  before the swap. If a genuine expressiveness gap exists, the fix is a
  `ParameterType` accessor (splitting the folded STATE back out
  structurally), never a parallel enum.
- **`User(String)` is scope-resolved at parse time** (its parser consults
  symbol tables). The swap must preserve WHERE resolution happens: syntaxcheck
  keeps resolving names, then constructs `Named`/`UserOf` — the canonical
  parse handles grammar, syntaxcheck handles scope, same as `elaborate`'s
  split. Phase 1 verifies this boundary by reading `parse_type`'s
  symbol-table touches.

## 3. Design Overview

A mechanical enum swap executed like plan-102's ports: alias first, then
migrate, then delete.

1. **Mapping table** (Phase 1): every `Type` variant → its `ParameterType`
   form, with the two known deltas resolved (`Error`/`ErrorLoc` → `Named`;
   thread `res_state` → folded spelling + accessor if any rule needs the
   split).
2. **Swap** (Phase 2): `type Type = ParameterType` is NOT enough (variant
   names/shapes differ) — convert module-by-module (`types.rs` callers first,
   then `inference.rs`, then the rule modules), compile-driven, with the
   private parser reduced to scope-resolution + canonical `parse` and finally
   deleted.
3. `helpers.rs`'s promotion copy converts to the numeric.rs typed source
   (E deletes the last copy once codegen's falls in 104).

**Correctness risk:** the highest of plan-106 — 124 rules' worth of
comparisons changing representation. Held by the strongest corpus in the
repo: every `*-invalid` fixture byte-compares the full diagnostic stream, and
accepted programs byte-compare through the gate. Convert incrementally
(module-per-commit) so a corpus failure localizes.

### Rejected alternatives

- **Keep the private enum, add a converter at the edges.** Rejected: that is a
  SEVENTH representation's worth of conversion code and none of the drift
  protection; the review's complaint is the multiplicity itself.
- **Do the HIR input switch simultaneously.** Rejected: two representation
  changes in one diff makes corpus failures unattributable; D does the input
  switch against an already-`ParameterType` checker.

## Compatibility / Format Impact

None. Diagnostics byte-identical.

## Phases

### Phase 1 — mapping table + thread-shape verification

- [x] Read both enums + every `Thread(`/`ThreadWorker(` match arm in
      syntaxcheck; record the complete variant mapping table here, including
      the `res_state` resolution (folded spelling or new accessor).

**The mapping table** (`src/syntaxcheck/mod.rs:27` vs `src/types.rs:23`):

| syntaxcheck `Type` | `ParameterType` | Note |
|---|---|---|
| `Boolean` `Byte` `Fixed` `Float` `Integer` `Money` `Nothing` `String` | same-named variants | exact |
| `List(T)` | `ListOf(T)` | exact |
| `Set(T)` | `SetOf(T)` | exact |
| `Map(K,V)` | `MapOf(K,V)` | exact |
| `Result(T)` | `ResultOf(T)` | exact |
| `Res(T)` | `Res(T)` | exact |
| `Function{params,return_type,isolated}` | `Func(params, ret, isolated)` | exact, incl. the `isolated` flag |
| `Unknown` | `Unknown` | exact |
| `Error` | `Named("Error")` | no variant; a nominal in the language |
| `ErrorLoc` | `Named("ErrorLoc")` | ditto |
| `Scalar` | `Named("Scalar")` | ditto |
| `AttributedString` | `Named("AttributedString")` | **not** `ParameterType::AttributeString` — that variant renders `"AttributeString"`, no `d`, a spelling the language's attributed-text type never uses (verified in letter A, `ir/lower.rs::attributed_string_type`) |
| `User(String)` | `Named(sym)` or `UserOf(sym, args)` | scope-resolved; `parse` classifies the grammar, syntaxcheck keeps classifying the name |
| `Thread(msg, res, res_state, out)` | `ThreadHandle{worker:false, msg, res, out}` | **4 slots → 3 — see below** |
| `ThreadWorker(...)` | `ThreadHandle{worker:true, …}` | ditto |

- [x] **The `res_state` question is ANSWERED, and the answer is: a genuine
      expressiveness gap exists.** The plan asked whether any rule
      *distinguishes* `res` from `res_state` in a way the folded spelling
      cannot. Two do:

      1. `resources.rs:421-441` (`thread.transfer` / `thread.accept`) checks the
         two planes **separately and differently**, with distinct diagnostics:
         `require_thread_sendable_type(…, "…resource STATE type", resource_state)`
         runs whether or not the `resource` arm then fires (bug-301 G4 — the
         STATE payload is deep-copied across the boundary, so it must be
         sendable in its own right).
      2. `types.rs:146-167` (`compatible`) compares the planes with two
         independent `compatible_optional` calls, so a `Some` state against a
         `None` state is decidable on its own axis.

      `ParameterType::ThreadHandle` folds the STATE into the `res` type's
      NOMINAL spelling (`Named("File STATE Cursor")`), so neither rule can be
      written against it as-is.

      Per this plan's own instruction ("the fix is a `ParameterType` accessor
      … never a parallel enum"), Phase 2 must add that accessor. **Letter B
      independently hit the same wall**: its five surviving production
      `ParameterType::parse` sites all exist to recover a STATE clause from
      inside a nominal (plan-106-B §Phase 2 census). So this is not a
      syntaxcheck-local problem — it is the one hole left in the `ParameterType`
      vocabulary, and closing it serves B's residue, C's swap, and E's census
      at once.

      **Decision, and it is DONE** (landed ahead of the rest of Phase 2, because
      letter B's residue needed it too):

      `ParameterType` gains `split_state` / `state` / `without_state` in
      `src/types.rs` — the structural twins of
      `codegen::resource::{state_type_name, base_resource_name}`, joining
      `with_state` from letter A. No variant was needed: the accessors are
      enough for both the thread plane's `res_state` slot (whose `res` child is
      a leaf) and every STATE read in `ir::lower` / `ir::verify`.

      They are **top-level only**, and that is load-bearing rather than a
      shortcut — see Correction 2. Wired at ten sites across `ir::lower` and
      `ir::verify`, which removes the last `state_type_name` renders from both
      and closes two of letter B's five recorded `parse` sites.

- [x] Read `types.rs::parse_type`'s symbol-table touches; record the
      grammar-vs-scope split.

`parse_type` (`src/syntaxcheck/types.rs:15`) does exactly **five** things that
`ParameterType::parse` does not, and they are the whole of what Phase 2 must
preserve. Everything else in its 1,077 lines is the type grammar, which the
canonical parser already owns.

| # | Non-grammar step | What it is |
|---|---|---|
| 1 | `base_resource_name(name)` at the top — strips a top-level ` STATE T` | **Deliberately lossy.** Its own comment: "`Type` has no STATE concept … a `fs::File STATE Cursor` IS a `File` for every purpose `Type` serves." syntaxcheck carries the clause *beside* the type, in `LocalInfo::state_type` / `ParamSig`. |
| 2 | `builtins::is_qualified_builtin_resource(name)` | Registry lookup: a qualified builtin **resource** (`fs.File`) keeps its qualified identity, because resources are package-scoped (plan-97). |
| 3 | `builtins::qualified_builtin_type(name)` | Registry lookup: a qualified builtin **value** type (`net.Url`) collapses to its bare internal id (plan-03-http §A.1/§B.2). |
| 4 | the thread arm peels `state_type_name(resource)` into the separate `resource_state` slot | The 4th-slot split — see the `res_state` finding above. |
| 5 | `self.user_types.contains(other)` in the tail | **A no-op today.** All three tail arms (`is_builtin_type`, `user_types.contains`, and the bare fallback) produce the identical `Type::User(other.to_string())`. The symbol table is consulted and the answer is discarded. |

**Correction to this plan's premise** (recorded in §Corrections below): §2's
"Verified properties" says `User(String)` "is scope-resolved at parse time (its
parser consults symbol tables)" and that the swap "must preserve WHERE
resolution happens". Row 5 shows the scope consultation is inert — the
discrimination it performs changes nothing. The real non-grammar work is rows
1–4: two **registry** rewrites (not scope), the STATE peel, and the thread-plane
split. That makes Phase 2 simpler than the plan assumed in one respect (no
scope threading to preserve) and harder in another (the STATE peel is load
-bearing and has no `ParameterType` equivalent — see the `res_state` finding).

Acceptance: the mapping table exists in this section with no UNVERIFIED rows.
**MET** — the table above is complete, and both flagged UNVERIFIED items (the
`res_state` distinction, the grammar-vs-scope split) are answered with citations.
Commit: —

### Phase 2 — the swap, module by module

- [ ] Convert `types.rs` callers → canonical parse + scope resolution;
      convert `inference.rs`; convert the rule modules (`checking.rs`,
      `resources.rs`, `builtins.rs`, `link.rs`, `helpers.rs`); delete
      `enum Type` and the private parser (+ its 1,077 lines of tests, ported
      to `ParameterType` where they cover grammar the canonical tests lack).
- [ ] `helpers.rs` promotion copy → the numeric.rs typed source.
- [ ] Tests: the full `*-invalid` diagnostic corpus after EVERY module commit.

Acceptance: `cargo test --no-fail-fast` green; diagnostic corpus
byte-identical; `artifact-gate all` no NEW diff; `rg -n 'enum Type' src/syntaxcheck/`
→ 0; `wc -l src/syntaxcheck/types.rs` → file deleted or reduced to
scope-resolution only (record which).
Commit: —

## Validation Plan

- Tests: full diagnostic corpus per module commit; ported grammar tests.
- Coverage check: 124/124 rules golden-guarded (measured, plan-102-F).
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (E owns docs).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Where scope-resolution lives after the parser dies:** a thin
  `syntaxcheck::resolve_type_name(&str, scope) -> ParameterType` that calls
  canonical `parse` then classifies user names, vs inlining at call sites.
  Recommend the thin fn — one seam, testable.

## Corrections

### 1. `User(String)` is NOT scope-resolved — the symbol-table consultation is inert

§2 Verified properties asserts "`User(String)` is scope-resolved at parse time
(its parser consults symbol tables)" and makes preserving that a Phase-2
requirement. Measured by reading `parse_type`'s tail
(`src/syntaxcheck/types.rs:104-107`):

```rust
other if builtins::is_builtin_type(other) => Type::User(other.to_string()),
other if self.user_types.contains(other)  => Type::User(other.to_string()),
other                                     => Type::User(other.to_string()),
```

All three arms are the same expression. The symbol table is queried and the
answer thrown away, so there is no scope-dependent classification to preserve
and the Open Decision's `resolve_type_name(&str, scope)` seam has nothing to
carry. What *is* scope-ish are two **registry** rewrites (qualified builtin
resource keeps its qualifier; qualified builtin value collapses to a bare id),
which are global lookups, not lexical scope.

Phase 2 is re-scoped accordingly: the seam it needs is a *registry+STATE*
adapter, not a scope resolver.

### 2. The STATE accessors must be TOP-LEVEL only — bug-429 depends on the no-op

`split_state` was first written to descend to the same child `with_state`
attaches to, so the two would be exact inverses for every shape. That is
mathematically tidy and **wrong**, and the gate caught it: 4 diffs, on
`bug427_list_union_state_rt` and `bug429_owned_list_union_drain_rt` — both
MISSING (the fixtures stopped compiling), not byte diffs.

```
$ target/release/mfb build -q -ast -ir <bug427 fixture>
…:29 error[TYPE_ASSIGNMENT_MISMATCH]:
   Assignment to `handles` has type List OF RES Handle STATE Cursor,
   expected List OF RES Handle.
```

The reason is recorded in the code that the descending version broke —
`ir::verify::values.rs::check_result_value_type`, whose comment states it
outright: base-normalizing a composite "is a no-op — `base_resource_name`
declines to split a ` STATE ` whose base contains a space", and that no-op is
what makes **both sides normalize identically**. Peel the element's clause on
one side while the other has none to peel and a correct, STATE-carrying
resource union is rejected asymmetrically. That asymmetry IS bug-429.

So the accessors reproduce the name helpers exactly, guard and all:
`List OF RES File STATE Cursor` splits to nothing. They agree with `with_state`
on leaf bases — every place a resource's STATE is actually read — and
deliberately not on composites. Both properties are pinned:
`split_state_is_top_level_only` (which cites the two fixtures) and
`split_state_matches_the_name_domain_helpers` (which compares against
`state_type_name`/`base_resource_name` on nine spellings, including the thread
planes whose ` STATE ` belongs to the plane).

The lesson generalises past this plan: *an inverse pair is not automatically the
right pair.* `with_state` builds a spelling and so must descend; reading one
back must not, because the callers compare normalized types and normalization
has to be symmetric.

## Summary

The sixth representation dies. The engine is already enum-shaped, so this is a
swap with a 124-rule golden corpus holding every comparison steady; the one
real design question (the thread `res_state` slot) is Phase 1's first read,
recorded before any code moves.
