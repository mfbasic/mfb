# bug-342: `src/ir/` cleanup cluster — a drifted hand-rolled dispatch chain, a dead duplicated block, three `is_c_abi_type` copies (one contradicting the spec), and a semantic verifier living inside the binary codec

Last updated: 2026-07-18
Effort: medium (1h–2h per item; large for the cluster)
Severity: LOW
Class: Other (cleanup)

Status: Open
Regression Test: existing acceptance goldens (no new expected output); new unit
tests only where an item collapses two implementations into one (A1, A2, A5).

A cluster of duplication, placement, and documentation residue in the IR layer
(`src/ir/`), found during the cleanup review of the whole tree. Every item is
verified against the current worktree with a measured citation. None of them
changes what the compiler emits — but three of them are duplications that have
**already drifted**, which is the reason this is filed rather than left alone:

- the hand-rolled builtin-package dispatch chain in `expression_type` has fallen
  three packages behind the shared helper it duplicates (A1);
- `is_c_abi_type` exists in three copies and the `ir::verify` copy contradicts
  the spec paragraph that anchors a different copy (A2);
- a 9-line `CSTRUCT` early-`continue` block is written twice back-to-back, and
  the second copy is unreachable (A4).

The single correct outcome of a fix is that each duplicated rule has exactly one
implementation, each file's contents match its name and module doc, and the
generated artifacts (`-ast`, `-ir`, `-br`, `-nir`, `-nplan`, `-nobj`, `-ncode`,
`-mir`) are **byte-identical** to today's committed goldens.

References:

- Found during the tree-wide cleanup review (Agent 12 — IR layer), base `25c38ba1`.
- `src/docs/spec/language/17_native-libraries.md:92` — the C-ABI type allow-list
  that A2 contradicts.
- `src/docs/spec/architecture/02_frontend.md` — the syntaxcheck / `ir::verify`
  rule split.
- Related memory note: plan-20 IR semantic verification (`ir::verify_semantics`
  is the sole rejecter on both paths).

### Covered by sibling bugs — NOT in scope here

- `StructSlotView` half-vestigial + dead `allow_cstring` param → **bug-326**
  (repo-wide dead-code sweep).
- Dead `display` local at `src/ir/verify/mod.rs:2966` kept alive by
  `let _ = display;` at `:3211` → **bug-326-A9** (already filed with the same
  citations; verified still present).
- Three near-identical recursive `IrValue` walkers plus the `package.rs` fourth
  → **bug-328** (traversal duplication; adds `visit_value` next to `IrValue`).
- `src/ir/verify/mod.rs` (5,268 lines, one 4,248-line `impl TypeEnv` with zero
  section banners) and `src/ir/lower.rs` (4,036 lines, 8 pipeline stages) file
  splits → **bug-327-T2-1** and **bug-327-T1-1**.

## Current State

Measured line counts for the layer (worktree `cleanup-review`, base `25c38ba1`):

| File | Lines |
| --- | --- |
| `src/ir/tests.rs` | 5,784 |
| `src/ir/verify/mod.rs` | 5,268 |
| `src/ir/verify/tests.rs` | 5,298 |
| `src/ir/lower.rs` | 4,036 |
| `src/ir/coverage_tests.rs` | 1,596 |
| `src/ir/binary.rs` | 1,557 |
| `src/ir/json.rs` | 932 |
| `src/ir/link.rs` | 719 |
| `src/ir/package.rs` | 365 |
| `src/ir/mod.rs` | 177 |
| `src/ir/value.rs` | 164 |
| `src/ir/op.rs` | 129 |
| `src/ir/types.rs` | 85 |

Nothing here is a runtime failure. The "current state" of each item is the
measured evidence recorded per item below.

## Items

### Theme A — duplicated logic, some of it already drifted

#### A1 — 19 copy-pasted builtin-package dispatch blocks in `expression_type` duplicate `builtins::resolve_call_return_type`, and have drifted three packages behind it

- Hand chain: `src/ir/lower.rs:2296-2510` inside `fn expression_type` — **19**
  consecutive `is_<pkg>_call(...) → <pkg>::resolve_call(...)` blocks, ~215 lines.
  In source order: general `:2296`, collections `:2327`, strings `:2358`, math
  `:2367`, vector `:2376`, bits `:2385`, fs `:2394`, io `:2403`, net `:2412`, os
  `:2421`, tls `:2430`, audio `:2439`, http `:2448`, json `:2457`, csv `:2466`,
  regex `:2475`, datetime `:2484`, crypto `:2493`, thread `:2502`.
- The helper already exists: `src/builtins/mod.rs:297-330`
  (`resolve_call_return_type`) covers **22** packages — the same 19 **plus
  `encoding`, `money`, and `term`**.
- The helper is already used elsewhere for exactly this job:
  `src/ir/verify/mod.rs:4052`.
- **This is the drift**: the review predicted `encoding` and `money` were
  missing; `term` is missing too. A call whose return type comes from one of
  those three packages resolves through `resolve_call_return_type` in
  `ir::verify` but falls off the end of the hand chain in `ir::lower`.
- Fix: delete the 19 blocks; call `builtins::resolve_call_return_type`. Verify
  the artifact gate is byte-identical afterwards — if it is not, the three
  missing packages were load-bearing and the delta must be explained before the
  change lands.

#### A2 — `is_c_abi_type` has three copies; the `ir::verify` copy contradicts the spec paragraph that anchors a different copy

- `src/ir/verify/mod.rs:2945` — **13** type names, **includes `CVoid`**.
- `src/syntaxcheck/helpers.rs:204` — 12 type names, no `CVoid`.
- `src/resolver/mod.rs:134` — 12 type names, **byte-identical** to the
  syntaxcheck copy.
- `src/docs/spec/language/17_native-libraries.md:92` states the narrower
  allow-list "does **not** include `CBool`, `CByte`, or `CVoid`" and anchors
  `[[src/syntaxcheck/helpers.rs:is_c_abi_type]]`.
- So: two copies are redundant-but-consistent, and the third disagrees with the
  spec. Correction to the original review note — it is *not* three mutually
  divergent copies; it is 2 + 1.
- Fix: one shared predicate on the AST types (converges with bug-324's
  `call_arg_value`/`constructor_arg_value` accessor work). Whether `CVoid`
  belongs is a **semantic decision, not a mechanical merge** — resolve it
  against the spec before collapsing, and note the answer in the spec.

#### A3 — `verify_package`, a semantic verifier, lives inside the binary codec and duplicates an `ir::verify` rule verbatim

- `src/ir/binary.rs:1485-1517` (`verify_package`) and `:1519-1557`
  (`verify_ops`).
- The empty-`MATCH` check at `binary.rs:1545-1547` emits
  `PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH: MATCH has no cases (not
  exhaustive)`.
- `src/ir/verify/mod.rs:154` defines
  `const VERIFY_MATCH: &str = "PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH"` and
  emits the message `MATCH has no cases (not exhaustive)` at `:1190-1191` —
  **same rule id, same message text**.
- The depth cap 256 is also declared twice under two names:
  `src/ir/binary.rs:102` (`MAX_DECODE_DEPTH`) and `src/ir/verify/mod.rs:411`
  (`MAX_DEPTH`).
- Fix: move the semantic half of `verify_package` to `ir::verify` (the sole
  rejecter per plan-20) and leave `binary.rs` owning only decode-structural
  limits; hoist one depth constant.

#### A4 — a 9-line `CSTRUCT` early-`continue` block is written twice back-to-back; the second copy is dead

- `src/ir/verify/mod.rs:3007-3015` and `:3016-3024`. `diff` of the two spans
  reports them **byte-identical, comment included**.
- The second is unreachable: the first block's `continue` fires on exactly the
  same predicate.
- Fix: delete `:3016-3024`. Smallest item in this bug.

#### A5 — `match_covers_all` and `check_match_exhaustive` duplicate one coverage computation

- `src/ir/verify/mod.rs:2655-2698` (`match_covers_all`) and `:3383-3447`
  (`check_match_exhaustive`). (The original review note had these two line
  ranges swapped.)
- Both run the same pipeline: `resource_base_type` → `union_variants`/`enums` →
  skip guarded cases → fold `Else`/`Value`/`OneOf` into `covered` →
  `all.difference(&covered).next().is_none()`.
- The inner closures are **not** byte-identical, only equivalent: `name_of`
  (`:2668-2672`) is `let name_of = |v: &IrValue| match v { … };` while
  `pattern_name` (`:3417-3423`) is
  `let pattern_name = |v: &IrValue| -> Option<String> { match v { … } };`. The
  arms are identical; the closure form and name differ.
- Fix: one `fn match_coverage(...) -> (all, covered)`; both callers consume it.

#### A6 — filter predicate-type resolution written three times

- `src/ir/lower.rs:2300-2318` (general `filter`), `:2329-2349`
  (`collections.filter`, in `expression_type`), `:3016-3027`
  (`collections.filter`, in lowering). Each does
  `strip_prefix("List OF ")` → `builtins::general::filter_predicate_type`.
- Fix: extract `filter_predicate_arg_type`.

#### A7 — `loop_kind_name` exists in three modules with byte-identical bodies

- `src/ir/json.rs:901`, `src/target/shared/nir/json.rs:1006`,
  `src/ast/serialize.rs:1506` — all three bodies identical
  (`For→"for"`, `Do→"do"`, `While→"while"`); only visibility differs
  (`pub(super)` on the `ir/json.rs` copy).
- A fourth spelling, `loop_kind_keyword` (uppercase keywords), at
  `src/ir/verify/mod.rs:4844`.
- Fix: put both spellings on `LoopKind` in `src/ast/`.

#### A8 — `merge_package`'s six "push if absent" O(n²) loops

- `src/ir/package.rs:115-184`, six `if !…iter().any(…) { push }` blocks: types
  `:116-123`, bindings `:124-132`, functions `:133-141`, link_functions
  `:149-157`, link_cstructs `:162-171`, link_aliases `:175-184` — **~70 lines**
  (the review note said ~45).
- Fix: one generic `push_unique` helper. Note the O(n²) is on package-merge
  cardinality, not per-op, so this is a readability fix, not a perf fix.

#### A9 — four separately-maintained primitive-type lists; the differences are real but undocumented

- `src/ir/verify/mod.rs:158-160` `PRIMITIVE_TYPES` — 9: Integer, Float, String,
  Boolean, Byte, Fixed, Nothing, Money, Scalar.
- `:2140-2141` (`is_comparable_seen`) — 12: the 9 above **plus** Error, ErrorLoc,
  Unknown.
- `:2545-2546` (`is_defaultable`) — 12, **byte-identical to the previous list**.
- `:3283-3292` (`provably_data_type`) — 11: the 12 minus `Unknown`.
- Each membership is defensible on its own (member-access target vs.
  comparability vs. default-value vs. provably-data), but nothing records *why*
  they differ — and `is_comparable_seen` and `is_defaultable` have no reason to
  differ from each other at all. A new primitive is four places to remember.
- Fix: one base list plus explicitly-named deltas, each carrying a one-line
  rationale comment.

#### A10 — repeated block-recursion, range-error, and arity boilerplate in `ir::verify`

- Block recursion, 8 sites, each preceded by the identical
  `let mut branch = locals.clone(); let mut branch_muts = muts.clone();`
  prologue: `src/ir/verify/mod.rs:1175`, `:1184` (If then/else), `:1233` (Match
  case), `:1255` (While), `:1330`, `:1345` (For/ForEach), `:1391`, `:1405`.
- "Did the range error fire?" idiom, exactly 4 sites: `:361`, `:821`, `:982`,
  `:1000` — all
  `let before = …len(); check_literal_range(…); let range_errored = …len() > before;`.
- `Assign` (`:980-986`) vs `AssignGlobal` (`:988-1005`) differ **only** in
  `locals.get(name)` / `self.globals.get(name)` and `muts` /
  `self.global_muts`.
- Arity checks, 3 sites with the same min/max + `format!("{min} to {max}")` +
  `TYPE_CALL_ARITY_MISMATCH` + early return: `:4130-4145` (term), `:4168-4184`
  (collections), `:4199-4215` (general).
- A 9-way `is_X_call → resolve_call` fan-out at `:4236-4255` (math, bits, vector,
  strings, encoding, io, fs, net, os) has the same shape as the A1 chain — fold
  both into the same helper.
- Fix: `check_block_in_branch(...)`, `with_range_check(...)`, one arity helper,
  and a table for the fan-out.

### Theme B — structure and placement

#### B1 — the IR data model is cut four ways with no stated principle

- `src/ir/mod.rs:13-140` declares ~128 lines of types: `IrProject` `:15`,
  `ProjectDocs` `:70`, `IrPackageDoc` `:76`, `IrDocKind` `:85`, `IrDocDecl`
  `:94`, `EntryPoint` `:113`, `IrFunction` `:121`.
- `src/ir/types.rs` (85 lines) holds `IrType` `:5`, `IrBinding` `:21`, `IrField`
  `:38`, `IrVariant` `:47`, `IrEnumMember` `:55`, `IrParam` `:61`,
  `IrSourceLoc` `:70`, `IrRecordUpdate` `:76`, `ExternalFunctionParam` `:82`.
- `src/ir/op.rs` (129 lines) = `IrOp` only. `src/ir/value.rs` (164) =
  `IrMatchCase` / `IrMatchPattern` / `IrValue`.
- `IrFunction` lives in `mod.rs` while `IrParam` — its own parameter type —
  lives in `types.rs`. A reader opens four files to see one data model.
- Fix: move the type declarations out of `mod.rs` (leaving it a module root plus
  re-exports), and state the split rule in the module doc.

#### B2 — `binary.rs`'s module doc is orphaned at the END of `json.rs`

- `src/ir/json.rs:917-932` is a 16-line "Binary Representation (structured)
  encode/decode" banner describing `IrProject`/`IrFunction`/`IrOp` binary
  serialization — stranded after `visibility_name`, describing a different file.
- `src/ir/binary.rs:1` is `use super::*;`; the file has **no** `//!` doc
  anywhere.
- Left behind by a past file split. Fix: move the banner to `binary.rs:1` as a
  `//!` doc.

#### B3 — `LowerContext` is declared 668 lines after its first use

- `src/ir/lower.rs:894-931` declares `struct LowerContext<'a>`; the first use is
  `let mut context = LowerContext {` at `:226`. That is **668** lines, not the
  ~180 the review note estimated.
- `write_ir` at `:653-658` also sits mid-file in a 4,036-line file.
- Fix: fold into the bug-327-T1-1 split of `lower.rs`; if that split is deferred,
  hoisting the struct to the top of the file is a standalone 5-minute win.

### Theme C — test organization

#### C1 — two parallel lowering-test modules with a duplicated harness whose copies differ

- Harness A: `src/ir/tests.rs:7-108` (`unique_dir` `:20`, `lower_src` `:48`,
  `try_lower_src` `:71`, `function` `:96`).
- Harness B: `src/ir/tests.rs:3539` `mod lower_pipeline_tests` (`temp_dir`
  `:3546`, `lower_src` `:3565`, `try_lower_src` `:3571`, `function` `:3600`).
- ~70 duplicated lines, and the copies **differ in their temp-directory key**:
  - A (`:26-29`): `"mfb_ir_test_{tag}_{}_{stamp}_{n}"` with `process::id()`, a
    nanosecond stamp, and an atomic counter — unique per *call*.
  - B (`:3547-3551`): `"mfb_ir_lower_{name}_{}_{}"` with `process::id()` and
    `thread::current().name()`.
  - Correction to the review note: B *does* include a per-test `name`, so it does
    not collide across arbitrary tests on one thread. The real (narrower) hazard
    is that B's key is not unique per **call**, and `temp_dir` begins with
    `remove_dir_all` — so two calls sharing a `name` wipe each other's directory.
- The module's doc comment at `:3537` is truncated mid-sentence: it begins
  `/// exercise the AST->IR lowering paths (\`lower.rs\`) directly.` — starting
  on a lowercase verb with no subject.
- Fix: one harness, one keying scheme (A's, which is per-call unique); merge the
  overlapping section taxonomies; repair the doc comment.

#### C2 — two parallel "every op and value" corpora

- `src/ir/coverage_tests.rs` (1,596 lines), `full_project()` at `:371-513`
  (143 lines).
- `src/ir/tests.rs` `mod binary_repr_tests` at `:340-1015`, `corpus_project()` at
  `:362-682` (321 lines), explicitly commented at `:361` "Build a project
  exercising every IrType, IrOp, IrValue, and IrMatchPattern kind".
- Variant mentions: `tests.rs` has 129 `IrOp::` / 151 `IrValue::`;
  `coverage_tests.rs` has 59 / 96. Both must be updated for every new variant.
- `coverage_tests` also names a *tooling motivation* (line coverage) rather than
  a subject.
- Fix: one corpus builder consumed by both suites; rename `coverage_tests` after
  what it actually verifies.

#### C3 — three hand-written `IrProject` literals that disagree on `IrFunction::kind`

- `src/ir/verify/tests.rs:8` (`fn project`), `src/ir/coverage_tests.rs:32`
  (`fn empty_project`), `src/ir/tests.rs:752` (`fn project_named`).
- The disagreement is real: `kind: "function"` at `src/ir/tests.rs:650` and
  `:741`, versus `kind: "func"` at `src/ir/coverage_tests.rs:52` and `:460` and
  `src/ir/verify/tests.rs:177`.
- The contract is stated at `src/ir/verify/mod.rs:431`:
  ``/// `func` or `sub` — a SUB call produces no value (TYPE_SUB_HAS_NO_VALUE).``
- So `"function"` is **off-contract**, and those fixtures silently escape the
  `TYPE_SUB_HAS_NO_VALUE` check rather than exercising it. This is the one item
  in this bug with a (test-only) correctness edge — the tests are weaker than
  they read.
- Fix: one shared fixture builder; normalize to `"func"`/`"sub"`; confirm the
  affected assertions still pass (if one now fires, that is a real rule finding
  and gets its own bug).

### Theme D — comment and doc hygiene

#### D1 — `usable_type`'s doc comment is glued onto `derived_binary_type`

- `src/ir/verify/mod.rs:4782-4785` is `usable_type`'s doc ("A node's annotated
  result type, or `None` when it is absent, empty, or the explicit `"Unknown"`
  marker…"), followed with **no blank line or separator** by
  `derived_binary_type`'s doc at `:4786-4792`, both landing on
  `fn derived_binary_type` at `:4793`.
- `fn usable_type` at `:4822` therefore has no doc at all.
- Fix: move `:4782-4785` above `:4822`.

#### D2 — stale intra-file line reference in a comment

- `src/ir/verify/mod.rs:1982-1986` ends `…(module "Unknown stays permissive"
  contract, :1834).`
- The actual contract is the module header at `:26-31` ("whenever a type cannot
  be resolved with certainty … the node is skipped rather than rejected").
- Line `:1834` is inside the **`"Float" =>`** arm of the literal-underflow match
  (opens `:1829`, emits `TYPE_FLOAT_LITERAL_UNDERFLOW`) — unrelated. (Correction:
  the review note called it a Fixed arm.)
- Fix: cite the symbol/section, not a line number.

#### D3 — `#[derive(Clone)]` separated from its item by a blank line, 8 times, inconsistently within the same files

- `src/ir/mod.rs:13-15`, `:119-121`; `src/ir/op.rs:3-5`;
  `src/ir/value.rs:3-5`, `:12-14`; `src/ir/types.rs:3-5`, `:53-55`, `:59-61`.
- The same files do it the other way elsewhere: `src/ir/mod.rs:69-70`, `:75-76`,
  `:84-85`, `:93-94`, `:112-113`; `src/ir/types.rs:20-21`, `:37-38`, `:46-47`,
  `:69-70`, `:75-76`, `:81-82`; `src/ir/value.rs:20-21`.
- Fix: `cargo fmt` does not close this; do it by hand in one commit.

## Goal

- Each duplicated rule in `src/ir/` has exactly one implementation, and A1's
  drift (3 missing packages) is resolved by construction rather than by hand-sync.
- `is_c_abi_type` has one definition, and its `CVoid` membership agrees with
  `src/docs/spec/language/17_native-libraries.md:92`.
- The dead second `CSTRUCT` block is gone.
- Semantic verification lives in `ir::verify`; `ir/binary.rs` owns only decode
  structure.
- Every module doc sits in the file it describes.
- The IR test suites share one harness and one corpus, and every fixture's
  `IrFunction::kind` is on-contract.
- **All artifact goldens are byte-identical before and after.**

### Non-goals (must NOT change)

- Any emitted artifact: `-ast`, `-ir`, `-br`/`.hex`, `-nir`, `-nplan`, `-nobj`,
  `-ncode`, `-mir`, or any linked binary. This bug is output-preserving by
  construction.
- The `.mfp` wire format, section ids, or the binary-representation layout.
- Any diagnostic **rule id or message text**. A2, A3, A5, A9 and A10 all touch
  code that emits diagnostics; collapsing duplicates must not renumber, reword,
  or reorder a single one.
- The set of programs accepted or rejected. If A1's collapse changes acceptance
  (because `encoding`/`money`/`term` start resolving in `ir::lower`), that is a
  **separate finding** and must be filed and landed on its own, not folded in
  here.
- The `ir::verify` / `syntaxcheck` split established by plan-20 — A2 and A3 move
  code *toward* that split, never away.
- Tempting wrong fix, forbidden: "fixing" C3 by changing the assertion instead of
  the `kind` value, which would preserve the fixtures' current failure to
  exercise `TYPE_SUB_HAS_NO_VALUE`.

## Blast Radius

Searched, not recalled. Every site below was confirmed by reading it.

- `src/ir/lower.rs:2296-2510` (A1), `:2300-2318`/`:2329-2349`/`:3016-3027` (A6),
  `:894-931`/`:226`/`:653-658` (B3) — fixed by this bug.
- `src/builtins/mod.rs:297-330` — the A1 target helper; **read only**, not
  modified.
- `src/ir/verify/mod.rs:4052` — already calls the A1 helper; unaffected, and is
  the proof the collapse is safe.
- `src/ir/verify/mod.rs` `:2945` (A2), `:3007-3024` (A4), `:2655-2698` +
  `:3383-3447` (A5), `:158-160`/`:2140-2141`/`:2545-2546`/`:3283-3292` (A9),
  `:1175`–`:1405` + `:361`/`:821`/`:982`/`:1000` + `:4130-4215` + `:4236-4255`
  (A10), `:4782-4793`/`:4822` (D1), `:1982-1986` (D2) — fixed by this bug.
- `src/syntaxcheck/helpers.rs:204` and `src/resolver/mod.rs:134` (A2) — in scope
  for the merge; these two are already byte-identical to each other.
- `src/ir/binary.rs:102`, `:1485-1557` and `src/ir/verify/mod.rs:154`, `:411`,
  `:1190-1191` (A3) — fixed by this bug.
- `src/ir/package.rs:115-184` (A8) — fixed by this bug.
- `src/ir/json.rs:901` + `src/target/shared/nir/json.rs:1006` +
  `src/ast/serialize.rs:1506` + `src/ir/verify/mod.rs:4844` (A7) — fixed by this
  bug; the `ast/serialize.rs` site overlaps **bug-343**, so land A7 in whichever
  bug reaches it first and drop it from the other.
- `src/ir/json.rs:917-932` → `src/ir/binary.rs:1` (B2) — fixed by this bug.
- `src/ir/mod.rs`, `types.rs`, `op.rs`, `value.rs` (B1, D3) — fixed by this bug.
- `src/ir/tests.rs`, `src/ir/coverage_tests.rs`, `src/ir/verify/tests.rs`
  (C1–C3) — fixed by this bug; test-only.
- `src/ir/verify/mod.rs:2966`/`:3211` (dead `display`) — **out of scope**,
  already owned by bug-326-A9.
- The four `IrValue` walkers and the `package.rs` fifth — **out of scope**,
  owned by bug-328; B1 must not pre-empt that bug's `visit_value` placement in
  `src/ir/value.rs`.
- `src/ir/verify/mod.rs` and `src/ir/lower.rs` file splits — **out of scope**,
  owned by bug-327. Ordering matters: see Fix Design.

## Fix Design

The risk in this bug is **not** the edits; each is small. The risk is that a
"pure cleanup" silently shifts an artifact. So the gate comes first and runs on
every commit, not at the end.

Every commit in this bug must pass, on an unmodified fixture tree:

```
cargo build --release
scripts/artifact-gate.sh target/release/mfb     # execution-free, ~5 min
```

`scripts/artifact-gate.sh` regenerates `-ast -ir -br -nir -nplan -nobj -ncode
-mir` for every fixture carrying the matching golden and `cmp`s each against the
committed file (`scripts/artifact-gate.sh:29-38`). A non-zero `diffs` count on
any item in this bug means the item is not output-preserving and must be
re-examined, not re-goldened. **No item in this bug may run
`scripts/sync-goldens.sh`.** Before merge, the full harness
(`scripts/test-accept.sh <mfb-exe> <actual-dir>`) must be green as well, since
the artifact gate does not link or run.

Ordering against sibling bugs:

- Land **A4** (delete 9 dead lines) first — it is the cheapest possible proof
  that the gate is wired and green.
- Land **A1** early: it is the one item with a real drift and the one most
  likely to produce a gate diff. If it does, stop and file the acceptance
  finding separately.
- Land **B1/B3** *after* bug-327's splits, or explicitly coordinate — otherwise
  the two bugs move the same declarations in opposite directions.
- Land **B1** *after* bug-328 places `visit_value` in `src/ir/value.rs`, so the
  visitor lands next to the enum rather than being moved twice.
- **A7** overlaps bug-343 at `src/ast/serialize.rs:1506`; pick one owner.

Rejected alternatives, so they are not re-litigated:

- *Keep the A1 hand chain and just add the three missing packages.* Rejected: it
  restores the invariant for exactly as long as nobody adds a 23rd package. The
  duplication is the defect; the missing rows are the symptom.
- *Delete the `ir::verify` `is_c_abi_type` copy and point it at the syntaxcheck
  one without deciding `CVoid`.* Rejected: that silently changes behavior in
  whichever direction the surviving list happens to point. Decide `CVoid` first.
- *Add a `#[allow]` or a "keep in sync" comment over A9's four lists.* Rejected:
  comments do not survive; the deltas must be expressed in code.
- *Fix C3 by relaxing the assertions.* Explicitly forbidden (see Non-goals).

## Phases

### Phase 1 — gate + audit (no behavior change)

- [ ] Confirm `scripts/artifact-gate.sh target/release/mfb` reports `diffs=0` on
      a clean tree; record `checked`/`ran` counts as the baseline.
- [ ] Land A4 (delete `src/ir/verify/mod.rs:3016-3024`) as the gate smoke test.
- [ ] Decide the A2 `CVoid` question against
      `src/docs/spec/language/17_native-libraries.md:92`; write the answer into
      this file and into the spec.

Acceptance: gate green at `diffs=0` after A4; the `CVoid` decision is recorded.
Commit: —

### Phase 2 — collapse the drifted duplicates

- [ ] A1: replace `src/ir/lower.rs:2296-2510` with
      `builtins::resolve_call_return_type`. Gate must stay `diffs=0`.
- [ ] A2: one `is_c_abi_type`; delete the two redundant copies; align `CVoid`
      with the Phase 1 decision.
- [ ] A3: move the semantic half of `src/ir/binary.rs:1485-1557` into
      `ir::verify`; hoist one depth constant over `binary.rs:102` /
      `verify/mod.rs:411`.
- [ ] A5, A6, A7, A8, A9, A10: extract the shared helpers named per item.

Acceptance: gate `diffs=0` after each commit; `cargo test` green; no rule id or
message text changed (`git diff` over `src/rules/` empty).
Commit: —

### Phase 3 — structure, tests, and docs

- [ ] B1, B2, B3 (coordinated with bug-327 / bug-328 per Fix Design).
- [ ] C1: one harness with per-call-unique temp dirs; repair the `:3537` doc.
- [ ] C2: one corpus builder; rename `coverage_tests` after its subject.
- [ ] C3: one `IrProject` fixture builder; normalize `kind` to `"func"`/`"sub"`.
- [ ] D1, D2, D3.
- [ ] Full `scripts/test-accept.sh` run; `cargo fmt` (second pass in
      `repository/`, which is not a workspace member).

Acceptance: full acceptance suite green; artifact gate `diffs=0`; zero golden
files modified in the diff.
Commit: —

## Validation Plan

- Regression tests: no new fixture. New unit tests only for the merged helpers
  (A2's single `is_c_abi_type`, A5's `match_coverage`, A1's dispatch table)
  asserting the full package/type sets, so a future omission fails a test rather
  than drifting silently.
- Runtime proof: `scripts/artifact-gate.sh` at `diffs=0` on every commit — this
  *is* the proof for an output-preserving bug — plus one full
  `scripts/test-accept.sh` run before merge.
- Byte-identity guard: `git status` must show **zero** modified files under any
  `tests/**/golden/` directory. If a golden moves, the change is out of scope.
- Doc sync: `src/docs/spec/language/17_native-libraries.md:92` if the `CVoid`
  decision changes the list; otherwise none expected.
- Full suite: `cargo test`, `scripts/test-accept.sh`, `cargo clippy`.

## Open Decisions

- **A2 — does the C-ABI allow-list include `CVoid`?** Recommended: follow the
  spec (`17_native-libraries.md:92`, list excludes `CVoid`) and correct the
  `ir::verify` copy; alternative is to amend the spec if `ir::verify`'s inclusion
  is deliberate. Must be settled before the merge, not during.
- **A7 ownership** — recommended: land in bug-343 (which already owns
  `src/ast/serialize.rs`); alternative is here. Either way, one owner.
- **B1 sequencing vs. bug-327** — recommended: let bug-327 split first and do B1
  as a follow-up; alternative is to do B1 first and hand bug-327 a smaller
  `mod.rs`.
- **C2 naming** — recommended: `ir_variant_corpus_tests`; alternative is to fold
  the corpus into `binary_repr_tests` outright.

## Summary

Twenty verified cleanup items in `src/ir/`. The engineering risk is concentrated
in exactly two places: **A1**, where collapsing the drifted dispatch chain could
change which calls resolve (`encoding`, `money`, `term`) and must be gated
before it lands, and **A2**, where the merge requires a real semantic decision
about `CVoid` rather than a mechanical dedup. Everything else — the dead
duplicated block, the orphaned module doc, the four primitive-type lists, the
duplicated test harnesses and corpora, the glued doc comments — is mechanical,
provable by `scripts/artifact-gate.sh` staying at `diffs=0`, and touches no
shipped output. Left untouched: the file splits (bug-327), the `IrValue`
traversal duplication (bug-328), the dead-code sweep (bug-326), and every
diagnostic rule id and message in the layer.
