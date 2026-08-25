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
| private parser to delete | plan-writing "1,077 lines"; **actually ~110** — see Correction 3 | the grammar was `parse_type` + `parse_function_type` + `parse_collection_element_type`; the file's other ~920 lines are the compatibility algebra, the argument-mode helpers and 500 lines of tests, none of which is a parser |
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
Commit: `38c3522d1`

### Phase 2 — the swap, module by module

Sequenced as a ladder so each rung is independently gated (see Correction 3 for
why this order, not the plan's original one):

- [x] **2a — delete the private GRAMMAR** (this rung). `parse_type` is now
      `ParameterType::parse` + a conversion; `parse_function_type` and the
      `strip_prefix` cascade are gone, and `parse_collection_element_type` is a
      one-line alias (`RES` is a variant of the canonical grammar).
      `rg -n 'strip_prefix\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF |FUNC\()' src/syntaxcheck/`
      → **0**. The conversion, `type_from_parameter` + `leaf_type_from_name`,
      *is* the Phase-1 mapping table made executable, and it applies the four
      real non-grammar steps at every level exactly as the recursive private
      parser did.
- [x] Deleted with their last callers: `syntaxcheck::types::split_map_body` (a
      bare delegate to `crate::types::split_top_level_to` — its 6 grammar
      assertions follow the splitter to its one home) and
      `builtins::is_builtin_type` (whose only production caller was the parser's
      inert tail arm — its 3 assertions follow to `registry().is_builtin_type`).
      `cargo build --bin mfb` → **0 warnings**.
- [x] ~~**2b — fold the `res_state` slot** into `res` using `with_state`/`state`,
      so `Type::Thread` drops to 3 slots.~~ — **moot as a standalone rung, and
      folded into 2e instead: doing it while `Type` is still private would
      regress plan-52-D.** See Correction 6.
- [x] **2c — rename the container variants** to `ParameterType`'s
      (`List`→`ListOf`, `Set`→`SetOf`, `Map`→`MapOf`, `Result`→`ResultOf`,
      `Function{..}`→`Func(..)`), and merge `Thread`/`ThreadWorker` into one
      `worker: bool` variant. **84** references measured
      (`rg -c 'Type::List\(|Type::Set\(|Type::Map\(|Type::Result\(|Type::Function' src/syntaxcheck/*.rs`).
      The two thread variants had byte-identical bodies at every one of their
      paired match arms, so the merge collapses each pair into one
      `ThreadHandle { .. }` pattern; `compatible` gains an explicit
      `expected_worker == actual_worker` guard, which is what previously came
      from their being separate variants. Verified: `cargo test --bin mfb` 3650
      passed / 0 failed, `artifact-gate all` 0 diffs, `test-accept` 1271 ran /
      0 mismatches, 0 warnings.
- [x] **2d — convert the four nominal variants** (`Error`, `ErrorLoc`, `Scalar`,
      `AttributedString` — **42** refs) onto `Type::User`, which is what they
      are and what `ParameterType` models them as. `User(String)` itself needs
      no change until 2e (it becomes `Named(Symbol)` there).
      Two predicates carry the cases where "is this a KNOWN type?" or "is this
      primitive-like?" mattered: `is_builtin_nominal` and
      `is_comparable_builtin_nominal` — the same shape `ir::verify` already uses
      in `is_comparable_defaultable_primitive`. Four `Type::error()` /
      `error_loc()` / `scalar()` / `attributed_string()` constructors keep the
      call sites reading as before. Verified: `cargo test --bin mfb` 3650
      passed / 0 failed, `artifact-gate all` 0 diffs, `test-accept` 1271 ran /
      0 mismatches, 0 warnings. See Correction 7 for the one place this went
      wrong first.
- [x] **2e — replace `enum Type` with `ParameterType`**; delete the conversion,
      and fold the thread plane's `res_state` into `res` at the same moment
      (Correction 6). `type Type = crate::types::ParameterType;` —
      `rg -n 'enum Type' src/syntaxcheck/` → 0 (the one hit is the doc comment
      recording the removal). The `type_from_parameter` conversion became
      `normalize`, which is only syntaxcheck's three real normalizations; the
      50-line `type_name` match plus `format_thread_type_name` and
      `thread_type_argument_name` collapsed into `ParameterType::name`;
      `compatible_optional` went with the folded plane. Two latent
      `named()`-on-a-structured-spelling bugs surfaced and were fixed
      (Correction 8).
- [ ] `helpers.rs` promotion copy → the numeric.rs typed source.
- [x] Tests: the full `*-invalid` diagnostic corpus after EVERY rung.

Acceptance: `cargo test --no-fail-fast` green; diagnostic corpus
byte-identical; `artifact-gate all` no NEW diff; `rg -n 'enum Type' src/syntaxcheck/`
→ 0; `wc -l src/syntaxcheck/types.rs` → file deleted or reduced to
scope-resolution only (record which).

Rung 2a verified: `cargo test --bin mfb` → 3650 passed, 0 failed;
`artifact-gate all` → `1255 tests, 1402 build(s), 1730 golden(s) checked,
0 diff(s)`; `test-accept` → 1271 ran, 0 mismatches — the whole `*-invalid`
diagnostic corpus byte-identical across a parser swap.
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

### 8. `named()` on a structured spelling, twice more — and the gate caught both

Rung 2e turned two long-standing `format!`-a-spelling-into-a-nominal sites into
live defects the moment the nominal fold went away. Both are the class letter A
found in the `fs::pathJoin` descriptor and letter B found in
`is_defaultable`; this is its third and fourth appearance, which is why the
registry now has a permanent guard for its half.

1. **`checking.rs`'s FOR EACH element type.** Iterating a `Map` built its element
   type as `Type::named(&format!("MapEntry OF {k} TO {v}"))`. While `Type` was
   private that was *correct*, because syntaxcheck's parser had no `MapEntry`
   arm and the member-access path re-parsed the spelling to recover `.key`/
   `.value`. Once `Type` was `ParameterType`, `named()` produced a nominal that
   matched nothing and `entry.value` degraded to `Unknown`:

   ```
   rt-behavior/types/types-behavior  =>  MISSING .ast/.ir
     error[TYPE_CALL_ARGUMENT_MISMATCH]: Call to `toString` has argument
     type(s) (Unknown), expected Integer, Float[, Byte], …
   ```

   Fixed by building it structurally (`Type::map_entry_of(k, v)`) and matching
   the variant in `infer_member` instead of re-parsing — which also deletes the
   parse-of-render plan-105-B had left there.

2. **The `WITH` read-only check.** `read_only_record_type` recognizes a
   `MapEntry` by `starts_with("MapEntry OF ")`, and its caller binds
   `Type::Named(name)` first. A `MapEntryOf` variant falls straight past that
   bind, silently dropping the rule. Given its own arm before the bind.

The pattern to carry forward: **a `format!`ed type spelling is a bug waiting for
its reader to become typed.** It survives exactly as long as every consumer
re-parses it.

### 7. Deleting a variant flips the default answer — `AttributedString`

Rung 2d reddened `attributed_string_not_comparable`. Removing
`Type::AttributedString` from `is_comparable_with_seen`'s `false` group was not
neutral: the general `Type::User(name)` arm below it answers **`true`** for any
name it cannot resolve ("unknown user type — permissive, no false rejection"),
so dropping the explicit arm silently flipped `AttributedString` from
not-comparable to comparable, and `a = b` on two attributed strings would have
started type-checking.

The comment I wrote while making the change said it "falls through to the
general `User` arm" as if that were harmless. It was the test that knew better.

Fixed with its own guarded arm, and the comment now records *why* the arm has to
exist rather than asserting the fall-through is fine.

The general lesson for the rest of this ladder: when a variant is folded into a
catch-all, the catch-all's DEFAULT becomes that type's answer — so every
predicate the variant appeared in has to be re-checked, not just the ones where
it appeared in a `true` list. `is_copyable_type_with_seen` and
`is_thread_sendable_type_with_seen` were audited the same way and their
catch-alls do agree (a builtin nominal is not a resource and not in
`type_infos`, so they answer `true`, which is correct) — they still got explicit
arms so the primitive set stays readable and independent of that arm's shape.

### 6. Rung 2b cannot stand alone — the STATE fold must land WITH the swap

The ladder first listed "fold `res_state` into `res`" as its own rung. It is not
one, and the reason is the invariant syntaxcheck is built on: `Type` deliberately
carries a resource's ` STATE T` **beside** the type, never inside it
(`parse_type` step 1, `LocalInfo::state_type`, `ParamSig`). Folding the clause
into a `Type::User("File STATE Cursor")` while the private enum still exists
would make every ordinary nominal comparison against it fail — which is exactly
the bug plan-52-D §4 fixed ("`fs::close(h)` on an imported stateful handle
reported argument type(s) (fs::File STATE Cursor), expected File").

The fold is only correct once `Type` **is** `ParameterType`, because only then
does the spelling legitimately carry the clause and only then do
`split_state`/`state`/`without_state` apply to it. And even then it is a
*localized* exception: the leaf peel stays, so STATE remains folded only inside
the thread plane's `res` slot — which is precisely where the canonical grammar
already puts it (`parse("Thread OF Nothing RES fs.File STATE Cursor TO Nothing")`
→ `ThreadHandle { res: Named("fs.File STATE Cursor") }`, which the IR and codegen
have always consumed). The two rules that need the two planes apart
(`resources.rs` `thread.transfer`/`accept`, `types.rs` `compatible`) call
`res.split_state()`.

Moved into 2e.

### 3. The "1,077-line parser" is ~110 lines of grammar

§2 Measured populations lists "private parser to delete — 1,077 lines,
`wc -l src/syntaxcheck/types.rs`", and §1's Goal says the file's parser is
replaced "by the canonical `parse`". The file is 1,031 lines at kickoff, but the
*parser* is only `parse_type` (~95 lines), `parse_function_type` (~18) and
`parse_collection_element_type` (~7). The rest is:

- `compatible` / `compatible_optional` / `expression_compatible` — the
  compatibility algebra (~130 lines), which is not a parser and survives the
  swap (it moves onto `ParameterType` at rung 2e);
- `is_numeric` / `is_comparable` / `is_orderable_*` / `require_comparable_type`
  and the `call_argument_mode` family (~150 lines);
- ~500 lines of tests.

`wc -l` on a file is not a measurement of the thing inside it. The real
deliverable — "syntaxcheck holds no copy of the type grammar" — is checkable
and now **met** at rung 2a:

```
$ rg -n 'strip_prefix\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF |FUNC\()' src/syntaxcheck/
$ echo $?
1        # no matches
```

The file is 1,026 lines and will not shrink much further; that is fine, because
line count was never the goal.

### 4. Phase 2 is a ladder, not a module walk

The plan sequences Phase 2 by MODULE (`types.rs` callers, then `inference.rs`,
then the rule modules). That order does not actually decompose: `enum Type`'s
variants are referenced from all eight modules (606 references), so changing a
variant's *shape* breaks every module at once no matter which one you start in —
there is no module-sized commit.

What does decompose is the **enum's shape**, one property at a time, with the
private enum kept until the end. Re-sequenced as rungs 2a–2e above. Rung 2a
(delete the grammar) is deliberately first because it is the rung that removes
the actual defect class — a duplicate parser that can drift from the canonical
one — and it is independent of every variant change.

### 5. A malformed `FUNC(` must stay `Unknown`

Rung 2a reddened `parse_function_type_malformed_yields_unknown`:
`parse_type("FUNC(Integer")` (no `) AS ` clause) answered `Type::Unknown` under
the private parser, and `Type::User("FUNC(Integer")` under the canonical one,
which leaves an unsplittable `FUNC(` as a nominal.

The test is RIGHT and was kept: `Unknown` is syntaxcheck's permissive skip, so a
*parse* failure degrades to "cannot say" rather than to a nominal that matches
nothing and rejects the program. `leaf_type_from_name` restores it explicitly,
with the reasoning at the site.

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
