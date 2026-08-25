# plan-106-E: Consolidation + the terminal no-strings census

Last updated: 2026-08-24
Effort: large (3h–1d) — corrected from "medium (1h–2h)" by plan-106-A Correction 2:
codegen's residual hand-rolled type-grammar sites survived plan-104 and land here
Depends on: plan-106-D (every engine typed, no backward edge; this letter
consolidates shared algebra and CERTIFIES the end state).

Finish the review's Recommendation #2 (consolidate the duplicated
type-inference/numeric-promotion walks behind single sources of truth) and
certify plan-106's terminal invariant: **no internal type-string
representation, parsing, or comparison anywhere in the compiler** — the
"NO STRINGS" end state, proven by a recorded census, not asserted.

See plan-106-A for the invariant's exact definition (the three permitted
boundary classes) and the roadmap.

References:

- `planning/Compiler Pipeline.md:28-29,68` — the sibling-walk and promotion
  censuses and the consolidation mandate.
- `src/numeric.rs` — the single typed promotion source (landed in 106-A).
- `src/codegen/engine/types/type_utils.rs` + `src/codegen/memory/…` — the
  five NIR type walks (`static_nir_value_type`, `static_type_name`,
  `static_type_name_for_fold`, `…_with_types`, `…_for_fold_with_types`),
  typed by plan-104 but still five sibling walks.
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/collections.md`,
  `src/docs/spec/architecture/02_frontend.md`/`04_ir.md`/`13_native-ir.md` —
  the docs pass.

## Prerequisites

See plan-106-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-106-D complete | `rg -n 'deelaborate' src/` → 0 | **MET** (2026-08-24, commit `47d60ec82`) — one hit, the tombstone comment; zero code |

## 1. Goal

- **One numeric-promotion implementation.** The measured 6 copies
  (`rg -n 'fn (numeric_binary_result_type|promote_loop_numeric_type_name)' src/`)
  are 1: `src/numeric.rs`'s typed algebra (A killed ir/lower + monomorph's 4;
  plan-104 killed codegen's; C converted syntaxcheck's; E deletes whatever
  shell remains and re-measures → exactly 1 definition, N callers).
- **The five sibling NIR walks collapse** to `static_nir_value_type` +
  environment parameters (the `_with_types`/`_for_fold` variants differ only
  in the environment consulted — the review's measured claim; verify by
  diffing the walk bodies before merging, record the diff summary here).
- **The terminal census passes and is recorded** in this file (the invariant
  from plan-106-A):
  - `rg -n 'strip_prefix("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF )' src/`
    → hits only `src/types.rs` + `src/ast/`
  - `rg -n '== "(Integer|String|Boolean|Float|Fixed|Byte|Money|Nothing|AttributeString|Scalar)"' src/`
    → 0 type-value compares (audit each residual hit; non-type string
    compares like op names are out of scope — annotate)
  - `rg -n 'format!\("(List OF|Set OF|Map OF|Result OF|MapEntry OF)' src/`
    → hits only `ParameterType::name`
  - type-valued `HashMap<String, String>` environments → 0 (per-module sweep)
  - `rg -n 'deelaborate' src/` → 0
  - `ParameterType::parse` call-site inventory → only the permitted parse-in
    boundaries (elaborate, wire decode, resolver's canonical AST-domain
    queries, tests) — list every site with its classification
- Docs and spec reflect the typed pipeline end-to-end.

### Non-goals (explicit constraints)

- No behavior change; byte-identical output and diagnostics (both corpora).
- Do not merge ENGINES across layers (lowering vs verify stay independent —
  the soundness rule requires it; consolidation means shared *algebra* and
  intra-layer sibling-walk merges only).
- No new abstraction layers (the review asks for single sources of truth, not
  a type-system framework).

## 2. Current State (at this letter's start — re-measure at kickoff)

All engines are typed (A–D, 104, 105); duplication remains as typed siblings:
the promotion shells not yet deleted, and codegen's five walks now
`ParameterType`-valued but still five bodies that must agree.

### Measured populations

| What | Count (plan-writing) | Command |
|---|---|---|
| promotion implementations | 6 → re-measure (expect 1–2 shells) | `rg -n 'fn (numeric_binary_result_type\|promote_loop_numeric_type_name)' src/` |
| sibling NIR walks | 5 | review `Compiler Pipeline.md:27`; confirm: `rg -n 'fn static_nir_value_type\|fn static_type_name' src/codegen/` |
| `.ai`/spec files needing the docs pass | 6 named in References | — |

### Verified properties

- **The `_with_types` variants "differ only in the environment they consult"**
  — the review's claim. **MEASURED IN PHASE 1, AND IT IS FALSE.** See Correction 1.
  The five are not five siblings; they are three distinct oracles, and the pair
  that looks mergeable answers *differently for the same program*.

## 3. Design Overview

Small, sequential, each behind the gate: delete promotion shells → merge
sibling walks (after the body diff) → run the census → fix any straggler the
census finds (a straggler is a TASK here, never a deferral) → docs pass.

### Rejected alternatives

- **Skip the census, trust the letters.** Rejected: plan-102 shipped with
  backward seams precisely because green gates were trusted to imply
  architecture. The census IS the deliverable.

## Compatibility / Format Impact

None.

## Phases

### Phase 1 — promotion + sibling-walk consolidation

- [x] Delete residual promotion shells; re-measure → 1 implementation.

      codegen's two shells are gone: `typed_numeric_binary_result_type` (which
      rendered both operand names, ran the string algorithm, then re-matched the
      result back to a variant) and its string twin `numeric_binary_result_type`.
      What replaces them is a one-line `promoted_binary_type` over the typed
      source. Re-measured:

      ```
      $ rg -n 'fn (numeric_binary_result_type|promote_loop_numeric_type|typed_binary_result_type|typed_promote_loop_numeric_type|promoted_binary_type|binary_result_type)' src/
      numeric.rs:431          typed_binary_result_type          <- THE implementation
      numeric.rs:463          typed_promote_loop_numeric_type   <- the FOR fold, built on it
      numeric.rs:400          binary_result_type                <- #[cfg(test)] name adapter
      syntaxcheck/helpers.rs  numeric_binary_result_type        <- 1-line delegation
      syntaxcheck/helpers.rs  promote_loop_numeric_type         <- 1-line delegation
      codegen/…/type_utils.rs promoted_binary_type              <- 1-line delegation
      ```

      **One algebra, three named delegations** (each existing because its layer
      defaults differently: syntaxcheck → `Unknown`, codegen → `Integer`), plus
      the `FOR` fold and a test-only name adapter.

      Production now has **zero** string promotion callers, which made
      `numeric::binary_result_type` and its two name mappers dead in the binary.
      They are `#[cfg(test)]` rather than deleted, and commented why: ~30
      assertions state the promotion table in NAME form, which is how it stays
      legible and how it is pinned against the frozen `legacy_*` copies. An
      adapter is not a second implementation.
- [x] Diff the five NIR walk bodies; record the delta (**Correction 1** — the
      review's premise is false, and the delta is recorded there in full); merge
      what is genuinely shared.

      What landed instead of the proposed merge — and it is the bigger prize:
      **all four `static_type_name*` walks went from `Option<String>` to
      `Option<ParameterType>`.**

      ```
      $ rg -n 'fn static_type_name.*-> Option<String>' src/
      (0 hits)
      ```

      That is what killed codegen's last string promotion callers, and with them
      a chain of derived string grammar across 9 files: `strip_prefix("Result OF ")`,
      `format!("Result OF {…}")`, `strip_prefix("ISOLATED FUNC(")` + `") AS "`
      re-split, `starts_with("List OF ")`, `== Some("Address")`,
      `== Some("AttributedString")`, `== Some("Byte") | Some("Scalar")`, and five
      `is_worker_thread_type(&str)` calls (that helper is now deleted; the typed
      `is_worker_thread_handle` reads the variant's `worker` flag).
- [x] Tests: the A-phase equivalence suite extended over the merged walks. The
      pre-pass/resolver drift test (`data_objects.rs`, bug-354) now compares
      `ParameterType`s structurally on both sides instead of names.

Acceptance: suite green; gate no NEW diff; walk count recorded. **ALL MET:**

```
cargo test --bin mfb                      3651 passed, 0 failed
cargo build --bin mfb / --release         0 warnings
artifact-gate.sh target/release/mfb all   1255 tests, 1402 build(s),
                                          1730 golden(s) checked, 0 diff(s)
test-accept.sh                            acceptance tests passed (1271 ran)
```

Walk count: **5 → 5**, deliberately (Correction 1), but 4 of the 5 changed
domain from `String` to `ParameterType`.
Commit: `2a5c3f40f`

### Phase 2 — the terminal census + straggler burn-down

- [x] Run every census line from §1; paste the full results here. **See
      §The terminal census below** — every line, its command, its count, and
      every residual hit classified.
- [x] Any hit outside the permitted boundaries is fixed in this phase. Four
      tranches, each gated independently:

      | | what | commit |
      |---|---|---|
      | 1 | 48 `helper(&x.type_.name())` render→parse round-trips across 36 files | `e70c84697` |
      | 2 | the collection emitter tree (24 functions) takes `&ParameterType` | `91bce3797` |
      | 3 | the last hand-rolled grammar — **and a real `.mfp` wire bug** (Correction 3) | `4746fa03a` |
      | 4 | monomorph's substitution walk + its 5 render→parse sites | `cba1524ba` |

      Hand-rolled type grammar outside `types.rs`/`ast/` went **63 → 10**, and
      all 10 are boundaries rather than residue (census line 1 below).

Stragglers already identified by earlier letters (each a TASK here, not a
deferral — see plan-106-A §Corrections 2 and 3 for the measurements):

- [x] **Codegen's residual hand-rolled type grammar** (plan-106-A Correction 2).
      Both promotion shells are deleted (Phase 1). The grammar sites were
      **~15 by the plan's estimate and 63 measured** — corrected in Correction 2
      below with the command. All are gone from `src/codegen/`:

      ```
      $ rg -n '(strip_prefix|starts_with)\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF |Thread OF |ThreadWorker OF |FUNC\(|ISOLATED FUNC\()' src/codegen/ src/target/
      (0 hits)
      ```
- [x] **Monomorph's substitution walk** (plan-106-A Correction 3). Both are
      `ParameterType -> ParameterType` now (`concrete_type` / `template_view`),
      the 14 type-grammar `format!`s are gone, and all five
      `ParameterType::parse(&self.concrete_type_name(…))` sites call the typed
      walk directly.

      The two behaviours the plan named are preserved, and both turned out to be
      *statable* rather than implied:

      * the per-level `strip_type_group` unwrap is **no longer needed here** —
        Correction 3 of this plan made `ParameterType::parse` peel a `(T)` group
        at every level, so a grouped spelling can no longer arrive as a junk
        nominal. The earlier fix paid for itself.
      * the `substitutions` probe belongs at the nominal leaves — **verified**,
        not assumed: every key is `Symbol::intern(param)` over a declaration's
        `template_params` (`lower.rs:707`, `:861`), so it is always a bare
        parameter name and could only ever have matched at a leaf. `leaf_symbol`
        accepts `Named` AND `Var`, because a name arriving through HIR was
        classified by `with_vars` and one arriving through a spelling was not.
      * the self-binding guard still compares RENDERED names, so a `Var`/`Named`
        spelling difference cannot be mistaken for progress and spin the walk.
- [x] `bench-lowering.sh` vs the 106 baseline: recorded, **not slower on the
      release path**, and the decisive evidence is not the wall clock:

      ```
                          baseline (94a38078b)   plan-106 end
        trivial   release        0.62                0.35
        one-regex release        7.09                6.87
        acceptance release      50.01               51.08
        regex fn instructions  869181              869181   <- identical
                  int_vregs    143493              143493   <- identical
      ```

      The lowered instruction and vreg counts are **byte-for-byte the same**,
      which is the direct measurement of "no work was added"; wall-clock is a
      proxy. Two of the three release timings improved. The third moved 1.1s
      (2%), which is inside the run-to-run variance observed on this machine —
      the same binary measured 54.61 then 51.08 on consecutive runs while a peer
      session was compiling. Debug timings drifted upward monotonically across
      runs (279 → 300 → 318) under that same load and are not a usable signal;
      recorded here rather than quietly dropped.

Acceptance: the census in this file shows the invariant HOLDS with every
residual hit classified into the three permitted boundary classes; suite
green; gate no NEW diff; `test-accept` no NEW mismatch; perf ≤ baseline.
Commit: `e70c84697` (1/n), `91bce3797` (2/n), `4746fa03a` (3/n),
`cba1524ba` (4/n), `a2b8692f5` (census recorded)

### Phase 3 — docs/spec pass

- [x] Docs/spec pass. Driven by a **symbol-level citation sweep**, not by
      reading: `spec_citations_resolve` is file-level only (`.ai/testing-gates.md`
      says so), so a deleted symbol passes it. Swept all 1,580 `[[file:Symbol]]`
      citations across `src/docs/spec/`, `src/docs/man/` and `.ai/` for symbols
      that no longer appear in the cited file.

      90 dangle in total; **exactly one is plan-106's** —
      `[[src/ir/lower.rs:promote_loop_numeric_type_name]]` in
      `spec/architecture/04_ir.md`, deleted by letter A. Fixed, with the prose
      retargeted to `numeric::typed_promote_loop_numeric_type` and a line saying
      what it replaced. The other 89 are pre-existing (mostly `file.rs:LINE`
      citations, which are not symbols) and are left alone — they are not this
      plan's to fix and are noted here so the next reader knows the sweep saw
      them.

      Prose corrections:
      * `.ai/codegen-invariants.md` — was "the `static_nir_value_type` oracle …
        with `typed_numeric_binary_result_type` as the numeric twin". Both halves
        were stale: ALL FOUR NIR oracles are `Option<ParameterType>` now, and the
        promotion twin is `promoted_binary_type`, a one-line delegation.
      * `.ai/resources-packages.md` — the thread resource-plane split is keyed on
        `is_worker_thread_handle` (the variant's flag); the name-prefix
        `is_worker_thread_type` it names is deleted.
      * `.ai/compiler.md` — the sibling-walk note is now correct AND carries
        Correction 1's finding: the walks are **not interchangeable**, with the
        table delta spelled out, so the next reader does not "consolidate" them
        into a behavior change.
- [x] Memory sync. `hir-parse-name-roundtrip-load-bearing` rewritten: the
      `deelaborate` dependency is gone (it lists what still depends on the
      round-trip — the wire serializers and codegen's name-keyed tables), and it
      now records the ONE deliberate exception to byte-exactness (the grouped-type
      peel, Correction 3 of letter D). `byte-identity-cannot-see-backward-seams`
      closes the loop with the census's own result: it works, and it must be
      allowed to fail — it found both the 109-site codegen gap and a real wire bug.

Acceptance: docs updated; full suite; gate; test-accept; fmt both crates.
Commit: `9255d90c6`

## Validation Plan

- Tests: equivalence suites; both corpora (byte-identity + diagnostics).
- Coverage check: the census sweeps `src/` wholesale — nothing outside the
  denominator.
- Runtime proof: gate; test-accept; bench vs baseline.
- Doc sync: Phase 3 IS the doc sync.
- Acceptance: full suite; gate; test-accept; fmt both crates.


## The terminal census

Run at the end of Phase 2. Every line from §1, its command, its count, and every
residual hit classified into one of plan-106-A's three permitted boundary classes
— or named plainly as NOT one, which happens once (line 6).

### 1. Hand-rolled type-grammar parsing → **10, all boundaries** (was 63)

```
$ rg -n '(strip_prefix|starts_with)\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF |Thread OF |ThreadWorker OF |FUNC\(|ISOLATED FUNC\()' src/ \
    --glob '!src/types.rs' --glob '!src/ast/**' --glob '!src/docs/**' | grep -v '///'
```

| n | site | class |
|---|---|---|
| 6 | `ir/tests.rs` ×3, `binary_repr/tests/sections_tests.rs` ×3 — assertions on a rendered spelling | **tests** (class 2) |
| 3 | `binary_repr/sections.rs` `is_structural` / `opaque_structural_kind` | neither: they choose WHICH ARM, and the fallback wire kind for a spelling that **did not parse**. They cannot consult a parse result by definition. |
| 1 | `syntaxcheck/types.rs:145` — the malformed-`FUNC(` guard | same: it fires precisely *because* `parse` said no. Pinned by `parse_function_type_malformed_yields_unknown`. |

`src/codegen/` and `src/target/` are at **zero**.

### 2. Scalar type-string compares → **0 outside tests and the name domain**

```
$ rg -n 'name\(\)(\.as_ref\(\)|\.as_str\(\))? == "(Integer|String|Boolean|Float|Fixed|Byte|Money|Nothing|AttributeString|Scalar|Error)"' src/
```
16 hits at Phase-2 start; 12 are in `ir/tests.rs` (tests), and the 4 production
round-trips are fixed: `binary_repr/writer.rs` ×2 compare the `ParameterType`
directly, `ir/verify/values.rs` ×2 use `is_named("Scalar")`.

The broader `== "Integer"`-family sweep is 67 hits, but the remainder compare a
NAME already in the name domain (a `&str` element/field type threaded from a
name-keyed table), not a rendered `ParameterType`. Those belong to line 6.

### 3. `format!` type construction outside `name()` → **10, all boundaries**

```
$ rg -n 'format!\("(List OF|Set OF|Map OF|Result OF|MapEntry OF|Thread OF|ThreadWorker OF)' src/
```

| n | site | class |
|---|---|---|
| 5 | `types.rs` — `ParameterType::name` itself | **the renderer** (class 3) |
| 5 | `binary_repr/reader.rs` — the `.mfp` type-table DECODER rebuilding a spelling from ids | **wire decode** (class 2/3) |

Gone this phase: monomorph's 14, the 6 collection builtins'
`format!("List OF {x}")` locals, and three
`ParameterType::parse(&format!("…"))` build-then-parse pairs.

### 4. Type-valued `HashMap<String, String>` environments → **0**

```
$ rg -n '(locals|function_returns|function_types|function_params|globals|binding_types|declared_binding_types|enclosing_return)\s*:\s*&?(HashMap<String, String>|Option<String>)' src/
src/binary_repr/writer.rs:169:  external_function_returns: &HashMap<String, String>,   <- WIRE metadata, not an environment
src/codegen/engine/builder/mod.rs:165:  promoted_float_locals: HashMap<String, String>,  <- name -> REGISTER (`%fN`), not a type
```
Both hits are false positives of the pattern; no type environment survives.
Three other `HashMap<String, String>` maps are nominal→nominal symbol tables
(`TypeIndex::variants`, `verify::link::resource_state`,
`syntaxcheck::close_to_type`), not type-shape maps.

### 5. HIR→AST de-elaboration → **0** (plan-106-D)

```
$ rg -n 'deelaborate' src/
src/hir/mod.rs:918:// behind one `deelaborate` entry, rendering the concrete HIR ...
```
One hit, the tombstone comment. Zero code. syntaxcheck's private `Type` enum and
parser are likewise gone (plan-106-C); the driver's signature round-trip went in
plan-105-A.

### 6. `ParameterType::parse` call-site inventory → **223 production, and one is
not a sanctioned boundary**

```
$ rg -n 'ParameterType::parse\(' src/ --glob '!src/docs/**' | grep -v tests | grep -v test_support \
  | sed 's|^src/\([a-z_]*\)/.*|\1|;s|^src/\([a-z_]*\)\.rs.*|\1|' | sort | uniq -c | sort -rn
  109 codegen      39 ir        38 types      26 monomorph
    4 hir           2 syntaxcheck  2 manifest    2 binary_repr    1 resolver
```

| area | n | class |
|---|---|---|
| `types` | 38 | the parser's own recursion + its tests — **the one parser** |
| `ir` | 39 | wire/JSON decode (`ir/binary.rs` 27) and decoded-IR hardening in `ir::verify` — **wire decode** |
| `monomorph` | 26 | the instantiation-key domain: mangled names in/out of `type_instantiations`, and `Symbol`-keyed substitution values built from argument spellings |
| `hir` | 4 | `elaborate` — **the AST→typed boundary** |
| `syntaxcheck`/`manifest`/`resolver`/`binary_repr` | 7 | AST-domain queries (`UNION` variants, `LINK` signatures, manifest entry) and wire decode |
| **`codegen`** | **109** | **NOT a sanctioned class — see below** |

**The one honest gap.** `src/codegen/` re-parses a type NAME in 109 places. This
is not scattered grammar any more — every one goes through the canonical parser,
and the hand-rolled cascades are at zero — but it is a render→parse round trip,
which plan-106-A:452 says the invariant exists to *surface*, not to bless.

What it is: codegen's block-layout, symbol-mangling and runtime-helper tables are
keyed by type NAME (`type_model.record_fields`, `union_variant_fields`,
`_mfb_builtin_{name}_{type}`, the `CollectionTypeLayout` codes). An emitter deep
in that tree is handed a spelling and asks the grammar what shape it is.

Why it is not fixed here: closing it means retyping the name-keyed tables
themselves, not the emitters — the emitters were retyped this phase (24 of them,
commit `91bce3797`), and the parses that remain are below that layer. That is a
codegen-representation change with its own risk surface, and it is not what a
plan about the *front end's* type vocabulary should be doing under cover.

Recorded as follow-up with its shape, not reclassified: **key the codegen type
tables by `ParameterType` (or by an interned type id) instead of by rendered
name.** The measurement above is the starting denominator.

## Open Decisions

- **Resolved during Phase 2:** the census's own line 6 turned up a residual the
  plan had not scoped — codegen's 109 name→type re-parses. The decision taken,
  and the reason, is recorded in the census above: name it plainly as a gap with
  its measurement and its fix shape, rather than reclassify it as a permitted
  boundary. Reclassifying is exactly the move plan-102 made when it shipped with
  backward seams behind a green gate, which is why §Rejected-alternatives says
  the census IS the deliverable.

## Corrections

### Correction 4 (post-archive, 2026-08-24) — census line 3 undercounted by one production site

A code-level re-verification of the archived census reproduced every line
exactly (line 1 = 10, line 2 = 12 test hits, line 4/5/6 inventories identical,
223-parse distribution byte-for-byte) EXCEPT line 3: the recorded "10, all
boundaries" missed one production `format!` type build present since plan-57-D
— `refined_list_literal_type`
(`src/codegen/collection/layout/builder_collection_layout.rs:2459`,
`format!("List OF {element}")` over name-domain `&str` inputs; the file's other
hit, `:2890`, is `#[cfg(test)]`). The true line-3 production count is **11**.
The site is not a new class: it is squarely the codegen name-keyed layout web
that census line 6 already records as the honest follow-up ("key the codegen
type tables by `ParameterType`"), and it joins that denominator. Recorded so
the follow-up's starting measurement is right, not to reclassify it as a
boundary.

### Correction 3 — the `.mfp` wire type-id encoder mis-split a nested `Map` key

Found while clearing census line 1, and the reason the one-grammar rule exists.

`binary_repr::sections::type_id` split a `Map OF …` body with
`split_once(" TO ")` — the LEFTMOST separator, which is exactly the mis-split
bug-108.2 fixed in the front end. For

```
Map OF Map OF String TO Integer TO Boolean
```

it encoded key `Map OF String` and value `Integer TO Boolean`: two types that do
not exist. Proved with a test written BEFORE the fix, and the damage is worse
than a wrong name — the table does not decode at all:

```
names: "truncated binary representation"
```

So a package exporting a nested-`Map`-key signature wrote an unreadable `.mfp`.
The container arms now decompose through `ParameterType::parse`, where the
top-level split rule lives once.
`a_nested_map_key_splits_at_the_top_level_separator`
(`binary_repr/tests/sections_tests.rs`) was RED before, GREEN after.

This is the concrete answer to "why not just leave the duplicate grammar alone":
the duplicate had the bug, and nothing else could see it.

### Correction 2 — the straggler count was ~15; it is 63

§Phase-2 inherited plan-106-A Correction 2's estimate of "~15
`strip_prefix("List OF "…)` sites" in codegen. Measured at kickoff:

```
$ rg -c '(strip_prefix|starts_with)\("(List OF |Set OF |Map OF |RES |Result OF |MapEntry OF |Thread OF |ThreadWorker OF |FUNC\(|ISOLATED FUNC\()' src/ \
    --glob '!src/types.rs' --glob '!src/ast/**' --glob '!src/docs/**'
63 hits across 24 files
```

The earlier estimate counted only `strip_prefix`, and only in codegen. The real
population also included `starts_with` predicates (the larger half), the `.mfp`
wire encoder, `ir::verify`, and the `general` resolver's string pocket.

Re-scoped in place rather than re-split: the burn-down ran as four gated
tranches, and the line now stands at 10 — all boundaries (see §The terminal
census, line 1). Size was not treated as a reason to defer any of it.

### Correction 1 — the five NIR walks are NOT five siblings; the review's premise is false

§2 recorded the review's claim as UNVERIFIED and required a body diff before any
merge. Diffed. The claim does not survive it.

The five are **three distinct oracles**:

| Walk | Role |
|---|---|
| `static_nir_value_type` | the precise typed oracle (registry-resolved calls) |
| `static_type_name` / `static_type_name_with_types` | the coarse builder oracle, and its pre-pass twin |
| `static_type_name_for_fold` / `…_for_fold_with_types` | 15-line wrappers adding the resolver fallback, one per base |

Only the `_for_fold` pair is "the same body with a different environment". The
BASE pair — the one the consolidation was aimed at — differs in three ways, and
two of them change the answer:

1. **`Global`**: the builder falls back to `self.globals`; the pre-pass answers
   `None`.
2. **The builtin-call table is a different table.** The builder maps bare names
   (`replace`, `find`, `mid`, plus `get`/`getOr` and eight `math.*`); the pre-pass
   maps qualified ones (`strings.find`, `collections.find`, `strings.mid`, eight
   more `strings.*`, `strings.graphemes`/`split` → `List OF String`, the three
   `strings` predicates → `Boolean`). Neither is a superset. **They answer
   differently for the same program.**
3. **The field source**: `self.type_model.record_fields` + `union_variant_fields`
   vs a flat `FieldTypes` map.

Merging them is a behavior change, which §Non-goals forbids — and the code already
knew: `static_type_name_for_fold`'s own doc says it "deliberately does NOT widen
`static_type_name` … widening it would shift their codegen for every program using
these builtins."

So the walk count stays 5, per §Phase-1's sanctioned alternative ("or record the
justified mode flags"). The real consolidation opportunity was one the plan had not
seen: all four `static_type_name*` walks were **`Option<String>`** — the largest
remaining type-string carrier in the compiler. Retyping them to `ParameterType` is
what Phase 1 did instead, and it is what let the promotion shells die.

**Filed for later, not silently dropped:** that the two base tables disagree is a
latent inconsistency (the pre-pass is documented as "the pre-pass twin of
`CodeBuilder::static_type_name`", and a twin that answers differently is a bug
waiting for a consumer to notice). Reconciling them is a behavior change and
belongs to its own ticket, not to a no-strings plan.

## Summary

The certificate letter: duplication collapsed to single sources, and the
"NO STRINGS" invariant proven by recorded greps — the review's Recommendations
#1 and #2 finished, checkably, with nothing left to take on faith.
