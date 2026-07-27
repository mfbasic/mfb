# plan-63-A: Set type shape (front-end recognition)

Last updated: 2026-07-25
Overall Effort (AI): large (3h–1d)   <!-- whole plan-63 feature -->
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–2h)
Depends on: nothing
Produces: the `Set OF T` type shape recognized by every front-end stage —
parser, resolver, monomorphizer, syntax checker `Type` enum, IR semantic
verifier — plus the comparability constraint on `T` and the defaultability rule
(`Set OF T` defaultable ⇔ `T` defaultable). No literal, no operations, no
codegen yet: after A you can write `MUT s AS Set OF Integer` (empty default) and
pass/return/store a `Set OF T`, and the verifier rejects a non-comparable
element type — but there is nothing to put in it.

This sub-plan adds a new built-in collection template `Set OF T` to the flat
type-name encoding and threads it through the same 25 front-end sites that
already special-case `Map OF `. A Set is, at the type level, "a Map whose keys
are the elements and whose value is absent": it carries the Map's
element-comparability constraint but has one type parameter, not two.

References (read first):

- `mfb spec architecture type-name-encoding` — the flat type-string contract; a
  new `OF`-bearing shape must be added in lockstep to parse/resolve/monomorph/
  syntaxcheck/IR-verify (that doc's closing paragraph is the checklist).
- `mfb spec language types` §4.7 (collections), §4.10 (defaults), §4.11
  (comparable/orderable) — the rules `Set` must obey.
- `mfb spec language collections` — the surface model Set joins.
- `src/docs/spec/language/04_types.md`, `.../12_collections.md`,
  `.../19_grammar.md` — the source files behind those specs (updated in D, but
  read now).

## Prerequisites

These are a precondition on the whole plan-63 feature, not a dependency to
negotiate. Stated once here; sub-plans B/C/D point back to this section.

| Must be true | Command | Status |
|---|---|---|
| Collection kind tag `3` is free (0=List, 1=Map, 2=ListFixed) | `grep -n 'COLLECTION_KIND_' src/target/shared/code/error_constants.rs` → highest is `_LIST_FIXED = 2` | MET (verified 2026-07-25) |
| An "absent value" value-type tag exists | `grep -n 'COLLECTION_TYPE_NONE' src/target/shared/code/error_constants.rs` → `= 0` | MET (verified 2026-07-25) |
| Map probe/bucket machinery keys on key bytes only (no value dependency) | `grep -n 'fn map_key_probe_eligible\|_mfb_rt_map_probe\|_mfb_rt_map_build_buckets' src/target/shared/code/builder_collection_query.rs src/target/shared/code/mod.rs` | MET (verified 2026-07-25 — probe compares `keyLength` bytes; values never read) |
| `cargo test` is green at HEAD | `cargo test` (full suite, never one module) | MET (verified 2026-07-26 — exit 0, 33 `test result: ok`, 0 FAILED) |

Everything below is written against the world where these hold. There are no
hedges for a world where the collection block lacks a bucket index or a free
kind tag.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before continuing, and again
> before deciding to stop. If you stop, report the status of *all* prerequisites.

## Dependency graph

```
A ← nothing        (this sub-plan: the Set type shape)
B ← A              (runtime block, literal, native add/remove/contains/toList/len, iteration)
C ← B              (set-algebra source generics: union/intersection/difference/subset/toSet)
D ← C              (docs, spec, man pages, examples, goldens, acceptance)
```

Execution is topological: A → B → C → D, re-checking each letter's stated
preconditions. The line is straight — Set has no fan-out — because every stage
produces exactly what the next consumes: A makes the type nameable, B makes it
constructible and mutable, C makes it algebraic, D makes it documented and
proven.

## 1. Goal

- A program can declare, default, pass, return, and store a `Set OF T` for any
  comparable `T`, and the compiler rejects `Set OF T` for a non-comparable `T`
  (`Set OF File`, `Set OF FUNC() AS Integer`, `Set OF List OF Integer`) with the
  same diagnostic family the Map-key check uses.
- `MUT s AS Set OF Integer` compiles with no initializer (empty-set default);
  `MUT s AS Set OF File` is rejected as non-defaultable.
- The type string `Set OF Integer` round-trips byte-identically through
  `parse_type_name → resolve_type_name → concrete_type_name` (monomorph
  substitution), proven by a parser + monomorph unit test.

### Non-goals (explicit constraints)

- **No new syntax beyond the type shape.** The `Set OF T { … }` literal is B, not
  A. A must not add a literal, a native member, or any codegen.
- **No change to `List`/`Map` behavior or encoding.** The 25 threaded sites gain
  a `Set OF ` arm *alongside* the `Map OF ` arm; no existing arm changes meaning.
- **No `RES` on Set.** A resource handle is not comparable, so `Set OF RES File`
  is a parse error, exactly as it is for a non-collection. Do not add a
  `RES`-transfer path for Set (unlike `List OF RES` / `Map OF K TO RES`).
- **Set is not comparable.** Like `List` and `Map`, a `Set` may not be a `Map`
  key, a `Set` element, or a `collections::contains` needle-holder.

## 2. Current State

The flat type-string encoding is the single carrier of type structure between
stages (`mfb spec architecture type-name-encoding`). Every stage re-derives
structure by `strip_prefix`/`split_once` on the same literals. The built-in
`OF`-bearing shapes today are `List OF`, `Map OF`, `MapEntry OF`, `Result OF`,
`Thread OF`, `ThreadWorker OF`, plus the `FUNC(` / `ISOLATED FUNC(` prefixes.

Key precedents the Set arm mirrors (all `Map OF `, the closest analogue — one
`OF`, an element-comparability constraint):

- **Parser:** `src/ast/expr.rs:588` `parse_type_name_inner` builds `Map OF K TO V`
  (`expr.rs:610-627`) and `List OF [RES] T` (`expr.rs:637-645`). Set adds a
  `Set OF T` arm (single element type, no `TO`, no `RES`).
- **Resolver:** `src/resolver/resolution.rs:1284` dispatches on
  `strip_prefix("List OF ")`, `:1301` on `strip_prefix("Map OF ")`; the built-in
  template exclusion list is `resolution.rs:1504`
  (`["MapEntry OF ", "ThreadWorker OF ", "Map OF ", "Thread OF "]`).
- **Syntax checker `Type` enum:** `src/syntaxcheck/mod.rs:38-45` has
  `List(Box<Type>)`, `Map(Box<Type>, Box<Type>)`, `Res(Box<Type>)`. Set adds
  `Set(Box<Type>)`.
- **Comparability predicate:** `src/syntaxcheck/types.rs` `is_comparable`; the IR
  mirror is `check_map_key_comparable` / `is_comparable_seen` in
  `src/ir/verify/values.rs`.
- **Defaultability:** `is_defaultable` in `src/ir/verify/resources.rs` (`List OF T`
  defaultable ⇔ `T`; `Map OF K TO V` ⇔ both) and the syntaxcheck twin.
- **Monomorph substitution + user-template exclusion:** `src/monomorph/lower.rs`,
  `src/monomorph/helpers.rs:291`
  (`["MapEntry OF ", "ThreadWorker OF ", "Map OF ", "Thread OF "]`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Non-test source sites keying on the `Map OF ` shape (`strip_prefix`/`starts_with`/string-literal) — the threading surface A must extend | 25 | `grep -rnE 'strip_prefix\("Map OF "\)\|starts_with\("Map OF "\)\|"Map OF "' src/ast src/resolver src/monomorph src/syntaxcheck src/ir src/target \| grep -v 'tests\|test\.rs' \| wc -l → 25` |
| Non-test source *files* referencing the `Map OF ` shape | 15 | `grep -rln 'Map OF ' src/ast src/resolver src/monomorph src/syntaxcheck src/ir \| grep -v tests \| wc -l → 15` |
| Built-in-`OF` exclusion arrays that enumerate `Map OF ` (each must gain `Set OF `) | 4 | `grep -rnE '"(List\|Map\|MapEntry\|Thread\|ThreadWorker) OF ",' src/ \| grep -v tests \| wc -l → 4` |

The 25 sites are the scope ceiling, not the guaranteed edit count: many are
`Map`-specific (map-key checks, `MapEntry` iteration) with no Set analogue, and
some are `List`-only. **Task A0 is to walk all 25 and classify each** as
needs-Set-arm / Map-only / List-only, turning the ceiling into the real edit
list before any code changes.

### Verified properties

- **The Map probe/bucket/equality machinery never reads a value.** Read
  `builder_collection_query.rs:60-129` (`map_key_probe_eligible`,
  `emit_map_query_key`, `emit_map_probe`) and the three `_mfb_rt_map_*` helpers
  (`src/target/shared/code/mod.rs:2042-2357`, per the Map research pass): every
  one operates on `(keyOffset, keyLength)` and FNV-1a over key bytes. This is the
  premise the *whole feature* rests on (Set = Map with a zero-width value), so it
  is verified now, in A, even though A emits no codegen. If it were false, B's
  design collapses.
- **Kind is metadata, dispatch is static.** `error_constants.rs:789-793` states
  `kind` "is written for self-description only — dispatch is static, and no
  generated code loads this field to branch on." So giving Set `kind = 3` (B)
  cannot break any runtime branch.
- **`Set` element ⇒ always a flat block.** A Set element must be comparable
  (§4.11); the only pointer-payload collection element types are resources,
  function values, and non-flat nested collections (`is_pointer_collection_payload_type`,
  `builder_collection_layout.rs:39-51`) — none of which are comparable. Therefore
  a `Set OF T` can never contain a pointer payload, so its scope-drop is always a
  single `arena_free` with no per-element walk (relevant to B, verified here).

## 3. Design Overview

`Set OF T` is a new built-in template with one type parameter. Canonical string:
`Set OF T` (no `TO`, no `RES`). It slots into the type-name grammar exactly where
`List OF T` does structurally (one element type), and inherits `Map`'s *semantic*
constraint (element must be comparable) rather than `List`'s (element merely
storable).

The design is deliberately additive: at every one of the ~needs-arm sites, the
Set arm sits beside the existing `List OF `/`Map OF ` arms and delegates to the
same element-type recursion. Nothing existing changes behavior.

**Where design uncertainty concentrated (now resolved, see Verified properties):**
whether Set could reuse Map's storage. It can. So A carries no spike — the
premise is proven — and A's risk is purely *breadth*: missing one of the 25
sites yields a type that works in most stages and mis-behaves in one. Task A0
(the census) exists to make that breadth auditable.

**Rejected alternatives:**

- *Set as sugar for `Map OF T TO Nothing`.* Rejected: `MapEntry`-based `FOR EACH`
  would yield `MapEntry OF T TO Nothing` instead of `T`, and every operation
  name would be a Map operation. Set needs its own iteration element type and its
  own operation surface, so it is a first-class shape, not sugar.
- *Set as a `List` variant with a dedup flag.* Rejected: dedup requires the hash
  index, which only the Map block carries. A Set is a Map-shaped block, not a
  List-shaped one.

## 4. Detailed Design

### 4.1 Canonical encoding

- Grammar addition (D updates the spec text; A implements it):
  `Args := … | SetArg`, `SetArg := Type` under a `Set OF ` base. `parse_type_name`
  accepts `Set OF ` then one recursive `Type`; it rejects `RES` after `Set OF `
  (unlike `List OF`/`Map OF ... TO`).
- Round-trip: `concrete_type_name` gains
  `strip_prefix("Set OF ") -> "Set OF " + recurse(element)`, mirroring the
  `List OF ` arm.

### 4.2 Type enum + constraints

- `src/syntaxcheck/mod.rs:38`: add `Set(Box<Type>)` to the checker `Type` enum;
  update every exhaustive `match` on `Type` (the compiler enumerates these once
  the variant is added — let it).
- Element comparability: `Set OF T` requires `T` comparable. Enforce in the
  syntaxcheck comparability path and mirror on the IR in `src/ir/verify/values.rs`
  with a check parallel to `check_map_key_comparable` (a distinct diagnostic
  message string, e.g. "Set element type", reusing the map-key error *code*).
- `Set` itself is **not** comparable: add `Set` to the not-comparable arms
  alongside `List`/`Map`.

### 4.3 Defaultability

- `Set OF T` defaultable ⇔ `T` defaultable (empty set), mirroring the
  `List OF T` rule. Add to `is_defaultable` (`src/ir/verify/resources.rs`) and its
  syntaxcheck twin, with the same recursion guard the List/Map arms use.

### 4.4 Iteration element type (declared here, consumed in B)

- `collection_iteration_type` (`src/ir/lower.rs:1312-1324`) currently maps
  `List OF T → T` and `Map OF K TO V → MapEntry OF K TO V`. A adds the arm
  `Set OF T → T`. (B is what actually lowers the loop; A only makes the type
  correct so B has something to consume.)

## Compatibility / Format Impact

- **New type string `Set OF T`.** It appears in `.mfp` package wire output only
  once B/C emit Sets; A alone changes no on-disk format. The `.mfp` type-table id
  for `Set` is assigned in B (package wire id space, distinct from the runtime
  `COLLECTION_TYPE_*` space — see `mfb spec package type-table`). A must **not**
  guess or reserve a wire id; that is B's job with the encoder in front of it.
- No change to any existing type's spelling, id, or layout.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — Census the 25 sites (design uncertainty first: breadth)

One line: turn the 25-site ceiling into the exact edit list before touching code,
so a missed site is impossible to hide.

- [x] A0: enumerate the sites from the population command and classify each
      (needs-Set-arm / Map-only / B-codegen), with `file:line` and a one-phrase
      reason. This is the phase's deliverable. **The grep counts 26 sites (not the
      plan's 25); see Corrections.** The grep also *undercounts*: two needs-arm
      front-end sites route through helper fns (`parse_map`) and so do not match
      the `"Map OF "` literal — they are added below explicitly.
- [x] Confirm the 4 built-in-`OF` exclusion arrays — **DISPROVEN, see
      Corrections.** All 4 (`resolution.rs:1504`, `monomorph/helpers.rs:291`,
      `syntaxcheck/types.rs:481`, `syntaxcheck/inference.rs:1543`) are the
      `type_owns_a_to_separator` helper — a detector for `OF`-constructs owning a
      top-level ` TO `. `List OF`/`Result OF` are deliberately absent because they
      own no ` TO ` (types.rs:478 documents this). Set owns no ` TO ` either, so
      **Set must NOT be added to any of the 4.** The real built-in-template guard
      that lists `List OF`/`Result OF` and DOES need `Set OF ` is
      `user_template_parts` (`monomorph/helpers.rs:149`, a `starts_with` chain, not
      an array) — captured in the needs-arm table below.

#### A0 classification (26 grep sites + 4 helper-routed needs-arm sites)

Front-end **needs-Set-arm** (Set = single-element, List-shaped structurally):

| file:line | site | Set treatment |
|---|---|---|
| `src/ast/expr.rs:589` | `parse_type_name_inner` (not in grep — inline) | new `Set OF T` arm, reject `RES`/`TO` |
| `src/resolver/resolution.rs:1301` | `resolve_type_name` Map arm | Set arm: resolve element |
| `src/monomorph/helpers.rs:67` | `unify_type` Map arm | Set arm: unify element (like List:55) |
| `src/monomorph/helpers.rs:150` | `user_template_parts` exclusion (`starts_with`) | add `"Set OF "` |
| `src/monomorph/helpers.rs:180` | `substitute_type_params` Map arm | Set arm (like List:171) |
| `src/monomorph/helpers.rs:291` | built-in exclusion array | add `"Set OF "` |
| `src/monomorph/lower.rs:1075` | FOR EACH loop-type | Set arm: `Set OF T → T` |
| `src/monomorph/lower.rs:1531` | `concrete_type_name` Map arm | Set arm (like List:1519) |
| `src/monomorph/lower.rs:1595` | `template_view_type` Map arm | Set arm (like List:1589) |
| `src/syntaxcheck/types.rs:70` | `parse_type` Map arm | Set arm → `Type::Set(Box)` |
| `src/syntaxcheck/types.rs:481` | built-in exclusion array | add `"Set OF "` |
| `src/syntaxcheck/inference.rs:1543` | built-in exclusion array | add `"Set OF "` |
| `src/ir/lower.rs:1318` | `collection_iteration_type` (grep hit 1351 helper) | Set arm: `Set OF T → T` |
| `src/ir/verify/values.rs:744` | `is_comparable_seen` not-comparable list | add `"Set OF "` (Set not comparable) |
| `src/ir/verify/values.rs:707` | `check_map_key_comparable` (helper-routed, not in grep) | Set arm: require element comparable |
| `src/ir/verify/resources.rs:302` | `is_defaultable` (helper-routed via `parse_map`, not in grep) | Set arm: defaultable ⇔ element |
| `src/ir/verify/ops.rs:708` | FOR EACH-requires-collection guard | allow `Set OF ` |
| `src/ir/verify/link.rs:650` | `provably_data_type` | allow `Set OF ` (no resources) |
| syntaxcheck `Type` enum + exhaustive matches | `mod.rs:38` | add `Set(Box<Type>)`; compiler enumerates the rest |

**Map-only** (no Set arm — Set is single-element and cannot carry `RES`/`TO`):

| file:line | reason |
|---|---|
| `resolution.rs:1504`, `monomorph/helpers.rs:291`, `syntaxcheck/types.rs:481`, `syntaxcheck/inference.rs:1543` | the 4 `type_owns_a_to_separator` arrays — Set owns no ` TO ` (like the already-excluded List/Result) |
| `src/ir/resource_escape.rs:492` | `is_res_marked_resource_collection`; a Set element can never be `RES` (non-comparable) |
| `src/ir/lower.rs:1351`, `src/ir/verify/mod.rs:1192` | `parse_map`/`parse_map_type` helpers — `Map OF K TO V` split, no Set analogue |
| `src/ir/lower.rs:2554` | crypto/http builtin default-arg padding; no builtin defaults an empty Set |

**B-codegen** (deferred to plan-63-B — runtime/native layer, not front-end type recognition):

`src/target/shared/validate/mod.rs:126`, `src/target/shared/plan/lower.rs:197`,
`src/target/shared/code/type_utils.rs:{96,275,286}`,
`src/target/shared/code/builder_owned_cleanup.rs:45`,
`src/target/shared/code/builder_control.rs:1240` (7 sites).

**Scope of Phase 2: 16 needs-Set-arm front-end edits** (the needs-arm table
above), plus the `Type::Set(Box<Type>)` enum variant and its compiler-enumerated
exhaustive-match arms (across `syntaxcheck/**`). The 4 `type_owns_a_to_separator`
arrays are Map-only (Set owns no ` TO `), correcting the plan's original
"add to 4 exclusion arrays" instruction.

Acceptance: this plan file contains the classification table; the count of
"needs-Set-arm" front-end sites is 16 (stated above with per-row reason and the
reason each excluded row is excluded).
Commit: 5b692d9de

### Phase 2 — Thread the Set arm through the front end

One line: add the `Set OF ` arm at every needs-arm site; no literal, no ops.

- [x] Parser: `Set OF T` in `parse_type_name_inner` (`src/ast/expr.rs:605`),
      rejecting `RES` after `Set OF `. Parser round-trip + `RES`-reject unit tests
      added in `src/ast/tests.rs` (`parses_set_type_variants`,
      `set_type_rejects_res_element`).
- [x] Resolver: `strip_prefix("Set OF ")` arm in `resolution.rs` beside the
      `Map OF ` arm at `:1301`. **The `:1504` exclusion array is Map-only (Set owns
      no ` TO `) — see Corrections; not edited.**
- [x] Monomorph: `Set OF ` arms in `substitute_type_params`/`unify_type`
      (`helpers.rs`), `concrete_type_name`/`template_view_type`/FOR-EACH-loop-type
      (`lower.rs`), and `"Set OF "` added to the `user_template_parts` guard
      (`helpers.rs:150`). (The `:291` array is Map-only — not edited.)
- [x] Syntaxcheck: `Set(Box<Type>)` in `mod.rs`; all 5 non-exhaustive `match`
      errors resolved (`resources.rs` copyable+sendable, `mod.rs`
      package-metadata + `check_type_reference` + `type_name`, `types.rs`
      `is_comparable_with_seen`+`compatible`+`parse_type`).
- [x] IR verify: element-comparability check in `check_map_key_comparable`
      (`values.rs`), Set-not-comparable arm in `is_comparable_seen`,
      defaultability arm in `resources.rs::is_defaultable`, `Set OF T → T`
      iteration-type arm (`lower.rs::collection_iteration_type`), FOR-EACH
      collection guard (`ops.rs`), `provably_data_type` (`link.rs`).
- [x] Tests: parser round-trip (`ast/tests.rs`); monomorph round-trip
      (`unify_recurses_into_all_container_shapes`,
      `substitute_type_params_rewrites_every_shape`,
      `user_template_parts_excludes_builtin_shapes`); end-to-end front-end
      round-trip (`ir/tests.rs::set_type_round_trips_through_front_end`); verifier
      rejections of `Set OF File`/`Set OF FUNC() AS Integer`/`Set OF List OF Integer`
      + accept of `Set OF Integer` (`ir/verify/tests.rs`); defaultability
      accept/reject (`mut_set_is_defaultable`, `rejects_mut_set_of_resource_not_defaultable`).

Acceptance: `cargo test` green with the new tests; a fixture declaring
`MUT s AS Set OF Integer` (empty), passing it to a `FUNC(Set OF Integer)` and
returning it, type-checks and lowers to IR without error; `Set OF File` fails
with the comparability diagnostic. (No runtime execution yet — that is B.)
Commit: d101f726a

## Validation Plan

- Tests: `src/ast/tests.rs` (parse/round-trip), the resolver/monomorph
  round-trip test, `src/ir/verify/*` negative tests (non-comparable element,
  non-defaultable `MUT`).
- Coverage check: confirm the new verifier arms are exercised — a green suite
  that never constructs a `Set OF File` proves nothing; the negative test must
  assert the specific diagnostic.
- Runtime proof: N/A for A (no codegen). The runtime proof lives in B.
- Doc sync: none in A — the spec/grammar text is D. (A implements the grammar; D
  writes it down. Flag in D that A already shipped the parser behavior.)
- Acceptance: `cargo test` full suite. No acceptance-golden run needed for A
  (A emits no machine code, so no `.ncode`/`.run` goldens change).

## Open Decisions

- **Operation namespace** (decided at feature level, restated here because A's
  `is_collections_call` routing may touch it): fold Set operations into the
  existing `collections::` package (recommended — `List` and `Map` already share
  one package) vs. a new `sets::` package. See plan-63-B §Open Decisions; A does
  not depend on the outcome (A adds no operations).
  Decision: Add to `collections::`

## Corrections

- **Site count is 26, not 25.** Re-running the plan's own population command at
  HEAD (`grep -rnE 'strip_prefix\("Map OF "\)|starts_with\("Map OF "\)|"Map OF "'
  src/ast src/resolver src/monomorph src/syntaxcheck src/ir src/target | grep -v
  'tests\|test\.rs' | wc -l`) returns **26**. The extra row vs. the plan's snapshot
  is immaterial to scope: 7 of the 26 are `src/target/**` codegen sites that belong
  to plan-63-B, not A.
- **File count is 17, not 15** (`grep -rln 'Map OF ' src/ast src/resolver
  src/monomorph src/syntaxcheck src/ir | grep -v tests | wc -l` → 17). Same cause;
  no scope impact.
- **A third front-end needs-arm site was missed by the census entirely
  (found in C).** `src/syntaxcheck/checking.rs:549` types a `FOR EACH` loop
  variable by matching the iterable's `Type` enum (`Type::List`/`Type::Map`), not
  the `"Map OF "` string, so no grep in A0 could have found it. Without a
  `Type::Set(element) => *element` arm the loop variable over a Set typed as
  `Unknown`, which made `collections::add(result, x)` inside every C generic fail
  `TYPE_CALL_ARGUMENT_MISMATCH` (the resolver needs the exact element type). Added
  the arm; this is properly an A-scope site (front-end loop-var typing) surfaced
  while wiring C. Lesson: the "grep undercounts" caveat applies to every `Type`-enum
  match, not only the helper-routed string sites below.
- **The grep undercounts front-end needs-arm sites.** Two front-end
  comparability/defaultability sites route through helper fns (`parse_map`) and so
  never match the `"Map OF "` literal: `ir/verify/values.rs:707`
  (`check_map_key_comparable`) and `ir/verify/resources.rs:302` (`is_defaultable`).
  Both are needs-Set-arm and are in the A0 table above. A pure grep census would
  have missed them.
- **The "4 exclusion arrays" do NOT need `Set OF `.** Plan Phase 1 said to add
  `"Set OF "` to the 4 built-in-`OF` exclusion arrays. All 4 are copies of
  `type_owns_a_to_separator` (`resolution.rs:1509`, `monomorph/helpers.rs:291`,
  `syntaxcheck/types.rs:481`, `syntaxcheck/inference.rs:1543`) — a detector for
  `OF`-constructs owning a top-level ` TO `. `List OF`/`Result OF` are already
  excluded there because they own no ` TO `; `syntaxcheck/types.rs:478` documents
  exactly this. Set owns no ` TO `, so adding it would misclassify `Set OF A TO B`-
  style parsing — Set must stay out of all 4. The real built-in-template guard that
  lists `List OF`/`Result OF` (and needs `Set OF `) is `user_template_parts`
  (`monomorph/helpers.rs:149`), a `starts_with` chain, not one of the 4 arrays.
- **No syntaxcheck defaultability twin exists.** Plan §4.3 says to add the Set
  defaultability arm to `is_defaultable` "and its syntaxcheck twin." There is no
  twin: `src/syntaxcheck/checking.rs:110` records that non-defaultable-`MUT`
  rejection "live in `ir::verify` now (plan-20-Z)." The single defaultability site
  is `ir/verify/resources.rs:302`.
- **Comparability enforcement is IR-side, not syntaxcheck-side.** Plan §4.2 says to
  "enforce in the syntaxcheck comparability path and mirror on the IR." The map-key
  comparability *rejecter* is `ir/verify/values.rs::check_map_key_comparable`
  (syntaxcheck's `require_comparable_type` is a relocated no-op, types.rs:357). The
  syntaxcheck `is_comparable(&Type)` (types.rs:296) is used only for `=`-operand and
  map-key-literal checks; the `Type::Set` arm there returns `false` (Set not
  comparable), which is correct and sufficient — the *element* comparability
  rejection lives in the IR check.

## Summary

A makes `Set OF T` a real type everywhere the front end reasons about types,
carrying Map's comparability constraint and List's single-parameter shape. The
engineering risk is breadth (25 candidate sites), not depth — the storage-reuse
premise is already proven — so the census (Phase 1) is the load-bearing step.
Nothing runtime, nothing user-visible beyond a declarable-but-empty type.
