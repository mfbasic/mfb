# plan-107-E: hir::shape scaffold + typing seam; named-argument cluster; builtin-call typing family

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-107-B (the recipe is battle-tested; verify's user-FUNC call
rules are the base the builtin-call family extends). Independent of C.
D depends on this letter (the shape pass it completes is created here).

Added by plan-107-A's audit (Corrections C-census / C-shape-typing). Three
deliverables, in order:

1. **The shape pass scaffold + typing seam.** `ir::shape` (module location per
   A Open Decisions) — one walk over the concrete `HirProject` with the
   function/trap context the (S) rules need, plus a `pub(crate)` seam over
   lowering's `LowerContext` construction and `expression_type` so the pass
   types expressions with lowering's own inference (never a third copy).
   Wired at both seams (build + audit) in the first-stream position; its
   diagnostics merge with syntaxcheck's and verify's exactly as today's two.
2. **The named-argument (S) cluster** — `TYPE_UNKNOWN_ARGUMENT_NAME` (18
   fixtures) and `TYPE_DUPLICATE_ARGUMENT_NAME` (2): the first rules in the
   shape pass, exercising the callee parameter-name tables
   (`LowerContext.function_params` for user/imported functions,
   `builtins::call_param_names`/`call_param_name_overloads` for builtins).
3. **The builtin-call typing family** — `TYPE_CALL_ARITY_MISMATCH` (273
   fixtures), `TYPE_CALL_ARGUMENT_MISMATCH` (283): the largest relocation in
   plan-107. Their surviving forms (user FUNC, function-value call, builtin
   call) go to `ir::verify`; their erased forms (the named-argument omission
   form of ARITY, the "cannot use named arguments" function-value form of
   ARGUMENT) go to the shape pass — landed atomically per code (A §3 "Split
   rules").

See plan-107-A for the shared prerequisites, gate policy, and recipe.

References:

- `src/ir/lower.rs:18-55` — `LowerContext` (the tables the seam exposes);
  `lower.rs:1987` — `expression_type`; `lower.rs:2367-2510` — the named-argument
  normalizers whose silent drops are the (S) evidence.
- `src/syntaxcheck/builtins.rs:267-900` — `check_builtin_call` and the four
  bespoke arms (`general`, `collections`, `term`, `thread`) + the table body
  `check_table_builtin_call` (its comment: "Ordering is load-bearing … inferring
  every argument *before* the arity check — and reporting an arity mismatch
  before a resolve failure — is what keeps diagnostic output byte-identical").
- `src/syntaxcheck/inference.rs:1293-1400` — the user-FUNC and function-value
  call checks (`check_call`, `check_function_value_call`).
- `src/ir/verify/calls.rs:57-160` — verify's user-FUNC arity/argument rules
  (the base; note the arity WORDING differs from syntaxcheck's and must take
  syntaxcheck's — the goldens' — form).
- `src/ir/verify/mod.rs:614-626` — `POISONING_RULES` (the arity/argument codes
  are already poisoners, so `TYPE_UNKNOWN_VALUE` cascades keep working once
  verify emits them on the source path).

## Prerequisites

See plan-107-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-107-B complete | B's boxes ticked | NOT MET until B lands |

## 1. Goal

- `ir::shape` exists, wired at both seams, with its typing seam; every rule
  in it carries a one-line justification naming the erased evidence.
- `TYPE_UNKNOWN_ARGUMENT_NAME`, `TYPE_DUPLICATE_ARGUMENT_NAME` fire from the
  shape pass; syntaxcheck's copies deleted; corpus set-equal.
- `TYPE_CALL_ARITY_MISMATCH`, `TYPE_CALL_ARGUMENT_MISMATCH` verify-only for
  every surviving form (user FUNC, function-value, builtin — including the
  four bespoke package arms), shape-only for the named-argument forms; listed;
  syntaxcheck's `builtins.rs` checker + `inference.rs` call checks deleted;
  corpus set-equal across all 283+273 fixtures.
- Package-path proof: a verify unit test per surviving form — a hostile
  `.mfp` calling a builtin with the wrong arity/types is exactly the class
  PKG-02 named (codegen marshals by declared parameter type).

### Non-goals (explicit constraints)

- Per plan-107-A (set equality, byte-identical codegen, no wording changes).
- The shape pass gains ONLY the named-argument cluster here; the rest of the
  (S) residue is D's.
- No registry changes: verify consumes `builtins::arity`, `resolve_call_return_type`,
  `expected_arguments`, `call_param_names` exactly as syntaxcheck does.

## 2. Current State

| What | Count | Command |
|---|---|---|
| `TYPE_CALL_ARITY_MISMATCH` emission sites in syntaxcheck | 10 | `/tmp/p107-emitted.sh` (A) — `builtins.rs:404,456,624,703,864,1040,1159,1170,1259`, `inference.rs:1366` |
| `TYPE_CALL_ARGUMENT_MISMATCH` sites | 12 | `builtins.rs:422,497,538,607,642,737,797,814,882`, `inference.rs:1320,1355,1391` |
| named-argument sites | 5 + 4 | `builtins.rs:969,1006,1116,1210` (unknown) / `1018,1100,1221` + `inference.rs`… (duplicate) |
| fixtures (ARITY / ARGUMENT / UNKNOWN_NAME / DUP_NAME) | 273 / 283 / 18 / 2 | `grep -rl --include=build.log " $CODE\]" tests` |
| `syntaxcheck/builtins.rs` size | 2473 lines (incl. tests) | `wc -l` |

### Verified properties

- **Every builtin call's target, args and arg types survive lowering** —
  `IrValue::Call{target: canonical "pkg.member", args, type_}`; the message
  spelling syntaxcheck uses IS the canonical name (A §2 "Message spelling").
  VERIFIED (goldens: `` Call to `math.pow` has 1 argument(s), expected 2. ``).
- **Byte-literal coercion**: lowering coerces an unsuffixed literal to the
  expected parameter type when it knows it (`lower_expression_with_expected`);
  for builtins `call_argument_expected_type` may not — so verify needs the
  same `resolve_table_call_with_byte_literals` retry over `Const{Integer}`
  literal args. UNVERIFIED which builtin params get an expected type; measured
  in Phase 3 (the harness will show any `astrings::foreground(255,0,0)`-style
  fixture drifting).
- **Diagnostic ORDER inside one call**: syntaxcheck infers every argument
  (reporting nested errors) BEFORE the arity check, and arity before argument
  mismatch. verify's `check_value` walks args first too (`calls.rs` order);
  UNVERIFIED that the interleaving matches on every fixture — the harness
  reports REORDER vs SETDIFF per fixture, and an in-fixture reorder that is
  not attributable to the stream move is a bug to fix, not a re-pin.

## 3. Design Overview

**Seam.** `ir::lower` exposes `pub(crate) struct LowerFacts<'a>` (or the
`LowerContext` itself) built by a `pub(crate) fn lower_facts(project,
external_signatures, imported_type_defs)` — the prologue of
`lower_augmented_project` — and `pub(crate) fn expression_type(...)`. The
shape pass keeps a `locals: HashMap<String, ParameterType>` per function the
way `lower_statement_block` does (LET/MUT/RES bind, FOR/FOR EACH vars,
trap bindings, lambda params) and calls `expression_type` where a rule needs a
type. Total lowering already tolerates ill-typed input (plan-20-D), so the
oracle never panics on the erroneous programs the shape rules exist for.

**Shape walk.** `ir::shape::check(hir, &facts) -> Vec<rules::PendingDiagnostic>`:
per file/item/function, statement recursion with a context {function kind,
inline-trap success-type stack, loop stack}, expression recursion for the
named-argument rules (every `HirExpression::Call` with a `HirCallArg::Named`).
Diagnostic locations: the statement/argument `line`s the HIR carries
(`HirCallArg::Named { line }`), matching syntaxcheck's.

**Builtin-call family in verify.** A `check_builtin_call` on `IrValue::Call`
whose target the registry owns: dispatch in syntaxcheck's order
(`general` → `collections` → `term` → `thread` → table), transcribing each
arm's rules over `infer_type(arg)` names; arity via `builtins::arity`;
resolution via `resolve_call_return_type` + the byte-literal retry;
`expected_arguments` for the message. The user-FUNC arity wording switches to
syntaxcheck's. Function-value calls: the `Func`-typed local's param list
gives arity + per-position compatibility.

**Landing order (each its own commit, corpus after each):**
1. seam + empty `ir::shape` wired at both seams (order-neutral: emits nothing).
2. `TYPE_UNKNOWN_ARGUMENT_NAME` → shape; list; delete.
3. `TYPE_DUPLICATE_ARGUMENT_NAME` → shape; list; delete.
4. `TYPE_CALL_ARITY_MISMATCH`: verify forms + shape (omission) form; list;
   delete syntaxcheck's 10 sites.
5. `TYPE_CALL_ARGUMENT_MISMATCH`: verify forms + shape (named-on-function-value)
   form; list; delete the 12 sites; then delete the now-unreferenced
   `check_builtin_call` machinery (and its tests move to verify).

### Rejected alternatives

- **A third inference for the shape pass.** Rejected (A §3): plan-106 spent
  five letters deleting duplicate type vocabularies.
- **Relocate the builtin-call family before the shape pass exists.**
  Impossible without dropping the named-argument forms (the list silences by
  code) — which is why the scaffold is step 1 of this letter.

## Compatibility / Format Impact

None to codegen/wire. Diagnostic order re-pins on the 283/273-fixture family
(listed per commit); sets unchanged.

## Phases

### Phase 1 — scaffold + seam

- [x] Expose lowering's facts/typing seam; create `ir::shape` with the walk
      and context, wired at build + audit seams (first stream); emits nothing.
      (`lower::LowerFacts` + `lower_facts()` — the prologue of
      `lower_augmented_project`, which now builds its context from them —
      and `pub(super)` `expression_type`/`match_expression_type`/
      `collection_iteration_type`/`match_case_binding`/`function_return_type`;
      `src/ir/shape.rs` walks every scope form lowering binds.)
- [~] Tests: unit test proving the seam types a HIR expression identically to
      lowering's stamped `IrValue` type on a sample; full suite;
      corpus SAME on every fixture (order-neutral).
      (`ir::shape::tests::walker_types_bindings_exactly_as_lowering_does`
      compares every binding's walker type against lowering's `Bind`/`ForEach`
      type across LET/annotated LET/inline-TRAP LET/FOR promotion/FOR EACH/
      MATCH binding/trap binding/lambda; corpus: `diag-set-diff.sh` → `521
      same, 0 reordered, 0 set-diff`. Remaining: the full suite at this commit,
      queued on the baseline checkout behind C's.)

Acceptance: pass wired, zero corpus change.
Commit: —

### Phase 2 — named-argument cluster

- [ ] TYPE_UNKNOWN_ARGUMENT_NAME → shape (justification line; every callee
      class: user, imported, builtin, overloaded builtin); list; delete.
- [ ] TYPE_DUPLICATE_ARGUMENT_NAME → shape; list; delete.
- [ ] Tests: corpus + harness per commit (18 + 2 fixtures); unit tests.

Acceptance: both codes shape-only; corpus set-equal.
Commit: — (per rule)

### Phase 3 — builtin-call typing family

- [ ] TYPE_CALL_ARITY_MISMATCH: verify (user/function-value/builtin incl. the
      four bespoke arms) + shape (omission form); package-path tests per
      form; list; delete.
- [ ] TYPE_CALL_ARGUMENT_MISMATCH: verify + shape (named-on-function-value);
      package-path tests; list; delete; remove `syntaxcheck/builtins.rs`'s
      checker and `inference.rs` call checks.
- [ ] Tests: corpus + harness (all 283/273 fixtures classified; every REORDER
      listed in the commit; zero SETDIFF); `artifact-gate all`; full suite.

Acceptance: both codes verify/shape-only; syntaxcheck's builtin checker gone;
corpus set-equal; gate byte-identical.
Commit: — (per rule)

## Validation Plan

- Tests: harness per commit; package-path tests per surviving form; seam unit
  test; full suite at letter end.
- Coverage check: A's fixture counts (18/2/273/283) — every form exercised.
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (D owns it).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Module path** — `src/ir/shape.rs` (decided in Phase 1): the pass borrows
  lowering's `pub(super)` items (`LowerFacts`, `LowerContext`,
  `expression_type`, …), which only a sibling of `ir::lower` can reach without
  widening them to `pub(crate)`.

## Corrections

- **Note from B (2026-08-29), for `TYPE_UNKNOWN_VALUE`'s relocation here:**
  lowering now binds a stray `RECOVER`'s value to a `$recover_stray` temp
  (B Corrections). verify's initializer cascade ("Initializer for binding
  `{name}` does not have a known type") must skip `$`-temps — syntaxcheck never
  emitted that cascade for a RECOVER value (nor for `$trap_res`/`$trap_val`,
  whose cascades it reports against the user's binding instead).

## Summary

The letter A's audit created: the shape pass is born with its typing seam and
its first two rules, and the plan's heaviest family — the registry-driven
builtin-call checks behind 556 fixtures — moves to verify where a hostile
package's calls are finally checked too.
