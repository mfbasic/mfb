# plan-107-A: Single-checker relocation — evidence audit + pilot rules

Last updated: 2026-08-29
Overall Effort: huge (>3d) — the whole plan-107 feature
Effort: large (3h–1d)
Depends on: nothing within 107 (plan-106 complete is the gate)

Finish the codebase's own declared end state — `rules/mod.rs`: "the eventual
single-checker (`ir::verify` traversal) end state" — by relocating **every
remaining semantic rule** out of `src/syntaxcheck/` into `ir::verify`, moving
the small set whose evidence lowering erases into a minimal pre-lowering shape
pass, and then **deleting `src/syntaxcheck/` entirely** along with the
dual-checker split machinery (`RELOCATED_TO_IR_VERIFY`, the skip logic, the
two-stream merge).

This also closes a real security gap class: today a rule still implemented
only in syntaxcheck does **not** guard decoded `.mfp` packages (verify is the
sole checker on that path — `verify/mod.rs` module docs, review finding
PKG-02). Every rule relocated in this plan starts guarding packages too.

This sub-plan is the **lead document for plan-107**. Roadmap (letter order =
implementation order; the alphabet is append-only, so E — added by the audit,
see Corrections — sits between B/C and D in the dependency graph, not at the
end of the work):

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | Per-rule evidence audit of all 49 remaining codes; the relocation recipe priced by 3 pilot rules landed end-to-end | large |
| **B** | The general semantic cluster relocated (pure (V) rules) | large |
| **C** | The `NATIVE_*`/LINK cluster; TESTING-assert family formally handed to D (verdict (S)) | large |
| **E** | `hir::shape` scaffold + typing seam; the named-argument (S) cluster; the builtin-call typing family (`TYPE_CALL_ARITY_MISMATCH` / `TYPE_CALL_ARGUMENT_MISMATCH` / `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`) | large |
| **D** | Remaining erased-evidence residue + split rules → `hir::shape`; **DELETE `src/syntaxcheck/`**; retire the split machinery; single-stream rendering | large |

Dependency graph: A → B → C; B → E; {C, E} → D.

References:

- `src/ir/verify/mod.rs:71` — `RELOCATED_TO_IR_VERIFY` (**71** entries:
  `sed -n 71,160p src/ir/verify/mod.rs | grep -oE '"[A-Z][A-Z_]+"' | sort -u | wc -l`
  → 71) and the sole-rejecter mechanism (`syntaxcheck::report` debug-asserts
  listed codes; `verify::collect_source_diagnostics` filters TO listed codes)
  — the proven incremental relocation machinery this plan drives to completion.
- `src/cli/build/mod.rs:373-414` — the split rationale comment (names the
  erased-construct classes: named arguments, EXIT flavors, inline-trap
  boundaries) and the two-stream concatenation.
- `src/rules/mod.rs:17-22` — the merge-order contract and the declared
  single-checker end state.
- `planning/Compiler Pipeline.md:47-48` — the review findings this plan
  discharges (dual-list `debug_assert!` hazard dies with the list itself).
- The plan-20-E..I history (`planning/completed/`) — the per-rule
  reproduction discipline this plan continues.
- `scripts/diag-set-diff.sh` — the set-equality harness (built in Phase 1).

## Prerequisites

Shared by every plan-107 letter; stated once here.

| Must be true | Command | Status |
|---|---|---|
| plan-106 complete | plan-106-A..E archived; `rg -n 'deelaborate' src/` → 0; `rg -n 'enum Type' src/syntaxcheck/` → 0 | MET 2026-08-29 — plan-106-A..E in `planning/completed/`; both greps hit only one historical doc comment each (`src/hir/mod.rs:918` "the de-elaboration block … is deleted", `src/syntaxcheck/mod.rs:91` "this was a private `enum Type`"), no code |
| Feature worktree + fresh baselines | as plan-104-A §Prerequisites (gate + suite + bench) | MET 2026-08-29 — `worktree-P-107` at `.claude/worktrees/P-107` forked from main `2299b6326`; `scripts/artifact-gate.sh target/release/mfb all` → `1259 tests, 1406 build(s), 1734 golden(s) checked, 0 diff(s)` (recorded in `planning/plan-107-baseline-diffs.txt`); full suite + bench: see the rows below |
| Full suite green at HEAD | `rustup run 1.96.0 cargo test --no-fail-fast` on a pristine detached checkout of main tip (`git worktree add --detach /tmp/p107-baseline 2299b6326`) | MET 2026-08-29 — exit 0; 64 `test result: ok`, 0 `FAILED` suites |
| Lowering perf baseline captured | `scripts/bench-lowering.sh` → `planning/plan-107-bench-baseline.txt` | MET 2026-08-29 — recorded (trivial 0.62/0.35, one-regex 31.80/7.26, acceptance 362.71/65.36 debug/release s) |

plan-106 is a hard gate, not a preference: post-106 the rules are
`ParameterType`-typed on HIR, so each relocation is a **transcription** into
verify's (also typed, post-106-B) environment instead of a string→typed
rewrite; and the erased-evidence residue lands in a shape pass that reads HIR,
which only exists as the checker's input after 106-D.

## 1. Goal

- Every one of the **49** not-yet-relocated codes (§2 — the plan-writing count
  of 46 was a census error, see Corrections) is classified with **evidence**,
  and 3 pilot relocations land end-to-end proving the recipe and its per-rule
  cost.
- The **diagnostic gate policy** for the whole plan is established and
  encoded in a harness (see §3) — because this plan is NOT diagnostics-order
  neutral and pretending otherwise would corrupt every later letter.

### Non-goals (explicit constraints)

- No change to compiled output for accepted programs — `artifact-gate all`
  byte-identical throughout every letter (codegen is untouched by rule
  relocation).
- No rule is weakened, merged, or reworded: same code, same message text, same
  file:line, per fixture — only the **stream a rule renders in** (hence
  relative order in multi-error fixtures) changes, and only when that rule
  relocates.
- Resolver/monomorph diagnostics (print-and-short-circuit) are NOT restructured
  here (the review's Rec #5 stays separate work).

## 2. Current State

The dual-checker split runs both passes to completion and concatenates
streams — syntaxcheck's traversal-ordered stream first, then verify's
(`build/mod.rs:446-449`; measured: `tests/syntax/control-flow/exit-loop-invalid`
renders syntaxcheck's `UNREACHABLE_AFTER_EXIT` at lines 3,4 BEFORE verify's
`EXIT_NO_MATCHING_LOOP` at line 2). **71** rules are verify's; **49** codes
still render from syntaxcheck.

### The census (measured 2026-08-29)

```
grep -rhoE '"[A-Z][A-Z_]+_[A-Z_]+"' src/syntaxcheck/ | sort -u | tr -d '"' > /tmp/sc.txt   # 120 strings
sed -n 71,160p src/ir/verify/mod.rs | grep -oE '"[A-Z][A-Z_]+"' | tr -d '"' | sort -u > /tmp/rel.txt   # 71
comm -23 /tmp/sc.txt /tmp/rel.txt | wc -l   # 49
```

Emission-site census (`/tmp/p107-emitted.sh`: every `self.report(`/
`self.report_warning(` site's code): **109 sites, 44 distinct codes**; the
codes with no emission site are `AUGMENTATION_FAILED`, `CARGO_MANIFEST_DIR`
(not rules — see table), `EXPORT_IN_EXECUTABLE` (its own build-boundary fn,
`mod.rs:253`), `TYPE_INLINE_TRAP_DEAD_HANDLER` (`report_warning`, counted
separately) and `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` (retired). Zero
syntaxcheck emission site names a relocated code — the `debug_assert!` in
`report` holds on the whole corpus (measured with `target/debug/mfb` on
`func_math_abs_invalid`: no panic; CI's `coverage.yml:139` runs the same
debug binary over the corpus).

Per-code golden-fixture counts: `grep -rl --include=build.log " $CODE\]" tests`
(the column below). Corpus for the harness: **518** golden `build.log`s carry
at least one diagnostic (`grep -rlE --include=build.log --include=test.log
' (error|warn|info)\[' tests | grep -c /golden/`); 0 `test.log` goldens do.

### Verdict table

Legend — **(V)** relocatable to `ir::verify` (the facts survive lowering);
**(S)** shape residue (lowering erases the evidence) → `hir::shape` (D, or E
for the named-argument cluster); **(V/S)** a split rule: one sub-shape
survives (verify) and another is erased (shape) — such a rule can only be
listed in `RELOCATED_TO_IR_VERIFY` once BOTH halves have a home, so it lands
in D (or E); **(I)** infra / not a rule. "verify has" = an implementation of
that code already exists in `src/ir/verify/` (package path) — the 20 marked
codes need a fidelity check + list entry + syntaxcheck deletion, not a port.
"Typing" = the shape-pass half needs an expression-type oracle (see §3).

| # | Code | Verdict | Evidence (what lowering keeps / erases) | Inference facts needed | Fixtures | verify has | Letter |
|---|---|---|---|---|---|---|---|
| 1 | AUGMENTATION_FAILED | (I) | not a rule: the test harness sentinel in `syntaxcheck::testutil::check_src` (`mod.rs:1788`); not in `rules/table.rs` | — | 0 | — | dies with syntaxcheck's tests (D) |
| 2 | CARGO_MANIFEST_DIR | (I) | `env!("CARGO_MANIFEST_DIR")` in a unit test (`types.rs:762`); not a rule | — | 0 | — | D |
| 3 | EXIT_FUNC_FORBIDDEN | (S) | `ExitTarget::Func` lowers to NOTHING (`lower.rs:696` `Vec::new()`) | function kind (structural) | 1 | — | D |
| 4 | EXIT_SUB_IN_FUNC | (S) | `ExitTarget::Sub` lowers to `Return{value:None}` (`lower.rs:695`) — identical to a bare `RETURN` | function kind | 1 | — | D |
| 5 | EXPORT_IN_EXECUTABLE | (I)→shape | a build-boundary rule over the ORIGINAL AST (needs manifest `kind`); no checker state; `syntaxcheck::export_in_executable_diagnostics` (`mod.rs:253`) moves verbatim beside the shape pass (the generic HIR mirrors the AST 1:1 with `HirFile.internal` + per-item visibility/line) | — | 1 | — | D |
| 6 | MONEY_INEXACT_FLOAT_LITERAL (Warn) | (S) | `numeric::classify_literal` STRIPS the `f`/`F`/`m`/`M` suffix (`numeric.rs:46-60`) before the `Const` is built (`lower.rs:2630`): `1.08` and `1.08f` are the same `Const{Float,"1.08"}` | typing (left operand `Money`) | 2 | — | D |
| 7 | NATIVE_ABI_NO_RESULT | (V) | LINK tables ride to IR verbatim (`lower_link.rs`); `verify/link.rs:390` | none | 0 → write one | yes | C |
| 8 | NATIVE_ABI_RESULT_MARKER | (V) | `verify/link.rs:399` has the "returns Nothing but declares a RETURN" form; syntaxcheck's struct-slot-is-IN form (`link.rs:243`) must be ported | none | 1 | partial | C |
| 9 | NATIVE_ABI_UNBOUND_PARAM | (V) | `verify/link.rs:430` (wording identical) | none | 0 → write one | yes | C |
| 10 | NATIVE_ABI_UNBOUND_SLOT | (V) | `verify/link.rs:331,465`; wording drift: verify says `SUCCESS_ON/RESULT`, syntaxcheck `SUCCESS_ON/RETURN` (`link.rs:599`) — syntaxcheck's spelling is the golden's | none | 6 | yes (wording) | C |
| 11 | NATIVE_ABI_UNKNOWN_CTYPE | (V) | verify has 2 of syntaxcheck's 3 forms (`link.rs:199` INOUT-non-CSTRUCT form missing) | none | 4 | partial | C |
| 12 | NATIVE_BIND_IN_INVALID | (V) | verify has 5 forms, syntaxcheck 6; three wordings differ (`link.rs:283,319,345` vs `verify/link.rs:121,142,152`) | none | 1 | partial | C |
| 13 | NATIVE_CONST_OUT | (V) | `verify/link.rs:310` (identical) | none | 1 | yes | C |
| 14 | NATIVE_CONST_UNKNOWN_SLOT | (V) | verify has the unknown-slot form (`441`); syntaxcheck's unfoldable-pin form (`link.rs:693`) must be ported | none | 1 | partial | C |
| 15 | NATIVE_CPTR_ESCAPE | (V) | `verify/link.rs:245,255` (identical) | none | 1 | yes | C |
| 16 | NATIVE_CSTRUCT_ESCAPE | (V) | `verify/link.rs:182,192`; wording differs ("only its mapped record type is nameable in a wrapper signature" vs "name its mapped record type instead — a CSTRUCT is nameable only in an ABI slot or SIZEOF") | none | 1 | yes (wording) | C |
| 17 | NATIVE_CSTRUCT_INVALID | (V) | `verify/link.rs:39` (identical) | none | 2 | yes | C |
| 18 | NATIVE_FREE_INVALID | (V) | `verify/link.rs:483,491`; malformed-FREE wording differs | none | 2 | yes (wording) | C |
| 19 | NATIVE_STRUCT_FIELD_MISMATCH | (V) | verify has the maps-to-non-record form (`75`); syntaxcheck's returns-struct-slot form (`link.rs:254`) must be ported | none | 1 | partial | C |
| 20 | PACKAGE_INVALID | (I) | 5 sites (`mod.rs:485,531,643,735,833`) fire while syntaxcheck READS an imported `.mfp`'s metadata — decode-boundary validation, not a program rule; `cli/build/packages.rs:157,164` already owns the code at that boundary; the resolver reads the same exports first (`resolver/packages.rs:82` → `IMPORT_PACKAGE_INVALID`) | — | 0 (unit tests `mod.rs:2343,2537`) | — | D: move the metadata validation (`validate_package_metadata_type` + readers) to the decode boundary; prove which sites the resolver already shadows |
| 21 | RESOURCE_SHADOWS_BUILTIN | (V) | every `RESOURCE` decl lowers into `IrProject.native_resources` (`lower_link.rs:368`) with its name + line; check `builtins::is_resource_type(name)` | none | 0 → write one | no | B |
| 22 | SUB_RETURN_FORBIDDEN | (V/S) | `RETURN <v>` in a SUB survives as `Return{Some}` (`verify/ops.rs:451`, same message); bare `RETURN` lowers to `Return{None}` = `EXIT SUB` → erased | function kind | 3 | valued form | D (both halves) |
| 23 | TESTING_EXPECT_ARITY | (S) | `expand_expect` (`lower.rs:819` → `testing/desugar/expect.rs`) desugars assertions into LET/IF/FAIL before IR exists; a missing operand makes the desugar EMPTY (`expect.rs:64`) | none | 0 → `mfb test` fixture | — | D (C records the hand-off) |
| 24 | TESTING_EXPECT_CODE_TYPE | (S) | as #23; the expected-code operand becomes `LET $expect_want = code` | typing (`Integer`-compatible) | 0 → fixture | — | D |
| 25 | TESTING_EXPECT_INCOMPARABLE | (S) | as #23; the `=` becomes an ordinary comparison (would surface as `TYPE_BINARY_OPERATOR_MISMATCH` — a different code) | typing (`infer_binary "="`) | 0 → fixture | — | D |
| 26 | TESTING_EXPECT_NOT_PRINTABLE | (S) | as #23 | typing (`is_printable`) | 0 → fixture | — | D |
| 27 | TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE | (S) | the guarded expression becomes an inline trap (`expand_trap`) | canonical callee name | 0 → fixture | — | D |
| 28 | TESTING_EXPECT_TYPE_MISMATCH | (S) | as #23; the typed assert's `want` (Float/Integer/…) is not in the desugar | typing | 0 → fixture | — | D |
| 29 | TYPE_COLLECTION_OWNERSHIP_VIOLATION | (V) | type references survive in every IR type position; verify has the Set-element + Map-key arms (`values.rs:900`); syntaxcheck's List-element-contains-thread and Map-value-contains-thread arms (`mod.rs:1552,1574`) must be ported; goldens show the message TWICE per line (declared type + literal type) | `contains_resource_or_thread` (verify has, `link.rs:614`) | 2 | partial | B |
| 30 | TYPE_DUPLICATE_ARGUMENT_NAME | (S) | lowering binds named args by name and the second occurrence silently overwrites the first (`lower.rs:2400-2411`, `2480-2484`) | callee param-name tables (= lowering's `function_params` + `builtins::call_param_names`/overloads) | 2 | — | E |
| 31 | TYPE_DUPLICATE_FIELD | (V/S) | WITH form survives (`verify/values.rs:179`, same message); constructor named-arg duplicate is erased (`lower_constructor_args` keeps the first match per field, `lower.rs:3596-3606`). Resolver's TYPE-decl form (`resolution.rs:701`) is not syntaxcheck's | none | 0 → write one (both forms) | WITH form | D (both halves) |
| 32 | TYPE_INLINE_TRAP_DEAD_HANDLER (Warn) | (V) | an inline trap lowers to `Bind $trap_resN = CallResult{target,…}` (`lower.rs:1166-1176`); the callee's canonical name survives as `target`; check `builtins::inline_builtin_is_infallible(target)`. GUARD: `expectTrap`'s desugar builds the same shape with a `$expect_err` binding — syntaxcheck never saw it as a trap, so verify must skip `$expect_`-bound traps | none | 1 (4 warnings, a `-valid` rt fixture) | no | B |
| 33 | TYPE_INLINE_TRAP_FALLS_THROUGH | (S) | `RECOVER` in a value-less trap lowers to NOTHING (`lower.rs:745`), so "handler ends in RECOVER" and "handler falls through" are the same IR; `treeify_handler` also restructures the handler | flow analysis over HIR (structural) | 1 | — | D |
| 34 | TYPE_INLINE_TRAP_ON_INLINED_BUILTIN | (I) | retired in plan-26-C: no emission site, not in `rules/table.rs` (line 723 comment), one negative test (`inference.rs:1874`) | — | 0 | — | dies with syntaxcheck's tests (D) |
| 35 | TYPE_INLINE_TRAP_REQUIRES_FALLIBLE | (V) | non-call scrutinee: `$trap_res` is bound to a non-`CallResult` value (`lower.rs:1163-1176` `other => other`); package-constant scrutinee: `CallResult{target}` with `builtins::is_package_constant(target)`. Both forms keyed on the `$trap_res` temp — the precedent is `$trap_val` for TYPE_RECOVER_TYPE_MISMATCH (`ops.rs:301`) | none | 2 | no | B |
| 36 | TYPE_ISOLATED_NOT_VISIBLE | (V) | `IrFunction{isolated, kind, visibility, loc}` (`ir/types.rs:198`) carries all three facts | none | 1 | no | **A pilot (decl-level)** |
| 37 | TYPE_LAMBDA_CAPTURE_UNSUPPORTED | (V) | captures survive as `Bind name = Capture{index,type_,by_ref}` in the `$lambdaN` body (`lower.rs:3149-3167`) and `Closure{captures:[Local…]}` at the use site; `by_ref` IS "MUT capture in a licensed non-escaping position" (`by_ref = nonescaping && mutable`); the enclosing function's `muts` map gives mutability; the diagnostic line is the enclosing statement (lambdas carry `context.current_loc`) | `is_copyable_type` (port, ~40 lines from `resources.rs:190`); resource-ness (verify has) | 2 | no | B (gap-bearing) |
| 38 | TYPE_RECOVER_OUTSIDE_INLINE_TRAP | (S) | a stray RECOVER lowers to a discard `Eval` or nothing (`lower.rs:719-748` fallback target) | trap-context stack (structural) | 1 | — | D |
| 39 | TYPE_RECOVER_TYPE_MISMATCH | (V/S) | the value-mismatch form survives (`Assign $trap_val = …`, `verify/ops.rs:324`, same message); the two arity forms ("must supply a value" → lowers to nothing; "must not supply a value" → discard `Eval`) are erased | typing (is the trapped expression's success type `Nothing`) | 1 (both forms) | mismatch form | D (both halves) |
| 40 | TYPE_RESULT_NOT_USER_VISIBLE | (V) | `ParameterType::ResultOf` / `Named("Ok")` survive in every IR type position; verify already walks declared types (`check_map_key_comparable` sites); the resolver ALSO emits this code for its own positions (`resolution.rs:1425`) — which emitter owns each golden line is measured at relocation | none | 2 | no | B |
| 41 | TYPE_SUB_CANNOT_RETURN_VALUE | (S) | `function_return_type` forces a SUB's IR `returns` to `Nothing` (`lower.rs:423`) — the declared annotation is dropped | none (structural) | 0 → write one; if the parser rejects `SUB x AS T` the rule is dead → moot with that evidence | — | D |
| 42 | TYPE_THREAD_NOT_SENDABLE | (V) | `ParameterType::ThreadHandle{msg,res,out}` survives in every type position; the boundary calls survive as `Call{target:"thread.start"/"thread.send"/transfer/accept}` with typed args; 4 message forms (`mod.rs:1612`, `resources.rs:346,437,488,499`) | `is_thread_sendable_type` (port from `resources.rs:186`; needs the imported `RESOURCE_TABLE` sendable bit, which bug-377's `imported_resources` seam does NOT carry — extend the seam) | 2 | no | **A pilot (inference-fact port)** |
| 43 | TYPE_TRAP_FALLTHROUGH | (V) | handler form already in verify (`ops.rs:767`, same message); the "Normal flow in `f` reaches the TRAP handler" form (`mod.rs:1521`) = `!block_always_returns(body before the Trap op)` — verify has `block_always_returns` (`resources.rs:380`) | none | 2 | partial | B |
| 44 | TYPE_UNKNOWN_ARGUMENT_NAME | (S) | an unknown named argument is silently dropped by lowering (`lower.rs:2400-2411`, `2480-2484`) | callee param-name tables (as #30) | 18 | — | E |
| 45 | TYPE_UNKNOWN_VALUE | (V) | initializer + RETURN cascades already in verify (`ops.rs:123,463`, poison-gated, same messages); remaining forms: default-param initializer (`mod.rs:1460`; `IrParam.default` survives), assignment target not a local (`checking.rs:335` → `Assign`/`AssignGlobal`), state-assign target not a local (`checking.rs:358` → `StateAssign`; demangles `#hash$name`), lambda assignment target (`inference.rs:1522`); the EXIT PROGRAM form is parser-unreachable | none | **301** (the reorder-scale probe) | partial | **A pilot (expression-level)** |
| 46 | UNREACHABLE_AFTER_EXIT | (V/S) | after `EXIT FOR/DO/WHILE`/`CONTINUE` survives (`verify/ops.rs:86`, same message) and after `EXIT PROGRAM` (`ExitProgram` op — add to verify's trigger); after `EXIT SUB` (→ `Return{None}`; a bare `RETURN` followed by statements is NOT reported by syntaxcheck) and `EXIT FUNC` (→ nothing) is erased | none | 3 (exit-loop/continue-loop = V forms; exit-sub-invalid = S forms) | loop forms | D (both halves) |
| 47 | TYPE_CALL_ARITY_MISMATCH | (V/S) | NEW to the census. Three surviving call shapes: user FUNC (`verify/calls.rs:96` — wording differs: "expected {required}..={total}" vs syntaxcheck's "expected {n}"/"{min} to {max}"), function-value call (verify SKIPS `Func`-typed locals, `calls.rs:74` — port), builtin call (`builtins::arity` + the registry; `check_table_builtin_call` + the four bespoke `term`/`thread`/`general`/`collections` checkers, `syntaxcheck/builtins.rs:267-900`). The named-argument omission form ("omits parameter `x` before a later supplied argument", `builtins.rs:1040,1259`) is erased with the names → (S) | registry resolution (`builtins::resolve_call_return_type` + byte-literal retry, `builtins.rs:27`) | **273** | user-FUNC form (wording) | E |
| 48 | TYPE_CALL_ARGUMENT_MISMATCH | (V/S) | NEW. user FUNC (`verify/calls.rs:141`, same wording); function-value call (`inference.rs:1391` — verify skips indirect calls; port); builtin call ("Call to `x` has argument type(s) (A, B), expected …" via `resolve_table_call_with_byte_literals` + `builtins::expected_arguments`); the "cannot use named arguments" function-value form (`inference.rs:1355`) is erased → (S) | as #47 + the bespoke checkers' per-position rules | **283** | user-FUNC form | E |
| 49 | TYPE_READ_ONLY_RECORD_CONSTRUCTOR | (V) | NEW. `Constructor{type_}` survives; verify has the compiler-owned form (`values.rs`); syntaxcheck has three forms (`AttributedString` opaque, `Error`/`ErrorLoc`, compiler-owned — `inference.rs:641,663,701`) | none | 4 | partial | B |

Message spelling: syntaxcheck's `display_callee` is already the CANONICAL
dotted name (goldens say `` `math.pow` ``, `` `tls.poll` ``, `` `thread.send` ``),
which is exactly the IR `Call.target` — so verify reproduces call messages
without the source spelling. Only an `IMPORT x AS y` alias would differ
(`y.f` vs `pkg.f`); 21 fixtures use an alias, all of them `-valid`/rt fixtures
whose goldens carry no diagnostic (measured in Phase 2's harness run).

Totals by verdict: (V) 21 pure + 6 split; (S) 13 pure (+ the 6 split halves);
(I) 5 (`AUGMENTATION_FAILED`, `CARGO_MANIFEST_DIR`, `EXPORT_IN_EXECUTABLE`,
`PACKAGE_INVALID`, `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`).

Zero-fixture rules (a fixture is written BEFORE each relocates): 7, 9, 21, 23–28
(as `mfb test` fixtures — TESTING blocks only lower under `mfb test`), 31, 41.

### Verified properties

- **The relocation mechanism works and is golden-guarded** — 71 rules landed
  through it (the list + skip logic + per-rule fixture verification;
  plan-20-E..I history). VERIFIED (landed history).
- **Order changes are confined and mechanical**: moving a rule changes only
  which stream it renders in; a fixture tripping a single rule sees NO golden
  change; multi-error fixtures see a deterministic reorder. VERIFIED by the
  stream model above; Phase 2's pilots measure how many fixtures per rule
  actually reorder (recorded per pilot).
- **Verify's typed inference (post-106-B) covers the moved rules' needs** —
  VERIFIED per rule in the table's "Inference facts needed" column: the only
  ports are `is_thread_sendable_type` (+ the imported sendable bit),
  `is_copyable_type`, and the builtin-call registry resolution (E).
- **The (S) rules' evidence exists in HIR** — HIR mirrors the AST 1:1
  (`HirCallArg::Named`, `ExitTarget` flavours, `HirExpression::Trapped`,
  `HirStatement::Recover`, `Lambda`, `Number(text)` with suffix). VERIFIED by
  reading `src/hir/mod.rs:174-400`. But 7 of the (S) rules need an expression
  TYPE (rows 6, 24–26, 28, 39) — see §3 and Open Decisions.

## 3. Design Overview

**The gate policy (applies to every letter — this is the plan's spine):**
- Accepted programs: `artifact-gate all` byte-identical, always.
- Rejected programs: per-fixture **diagnostic-set equality** — same
  (file, line, code, detail) multiset before and after each relocation;
  ORDER may change only on fixtures that trip the relocated rule, and each
  affected golden is regenerated **deliberately and listed in the commit**.
  `scripts/diag-set-diff.sh <mfb-exe> [-v] [glob…]` re-runs every fixture
  whose golden `build.log`/`test.log` records a diagnostic (518 today) with the
  golden's own echoed command lines (dump flags dropped) and classifies each
  as SAME / REORDER / SETDIFF; it exits non-zero on any SETDIFF, so a wording
  or line drift can never hide inside an expected reorder.
- Package path: for each (V) rule, a verify unit test building the violating
  `IrProject` by hand — the precedent for "package-path proof" throughout
  `verify/tests.rs` (e.g. the plan-58-A CBuffer twins) — proves the rule fires
  on decoded IR (the security payoff made testable).

**Relocation recipe per rule** (the pilots prove it): implement in the right
`verify/` module reading typed IR (+ port any missing inference fact) → add
the code to `RELOCATED_TO_IR_VERIFY` (syntaxcheck's copy goes silent) → run
the corpus with the harness → regenerate the listed goldens
(`scripts/sync-goldens.sh target/release/mfb <name-glob>`) → delete the
syntaxcheck implementation → corpus again (proves the list entry, not luck,
did the silencing).

**Split rules (V/S)** cannot use the list until both halves have a home: the
list silences every syntaxcheck emission of a CODE, so listing a split code
before its (S) half exists in `hir::shape` drops diagnostics. They land in
D (rows 22, 31, 39, 46) and E (47, 48), each commit adding the shape half,
confirming verify's half, listing, and deleting — atomically per rule.

**The shape pass needs a typing oracle.** Seven (S) rules (rows 6, 24, 25,
26, 28, 39 — plus 47/48's named-argument forms need callee parameter tables)
read an expression's TYPE, and syntaxcheck's 2,779-line `inference.rs` is
exactly what is being deleted. The only other HIR typer is lowering's own
`ir::lower::expression_type` (`lower.rs:1987`), driven by a `LowerContext`
whose tables (`function_params`, `function_returns`, `type_index`,
`binding_types`) lowering builds from the same inputs the build seam already
has. E exposes that context + `expression_type` as a `pub(crate)` seam and
the shape pass walks statements tracking locals the way lowering does —
ONE inference, not a third copy. (Rejected: keeping a trimmed `inference.rs`
— it is the duplicate type vocabulary plan-106 spent five letters deleting.)

**Risk concentration:** wording/line fidelity — verify derives locations from
IR spans (plan-20-A infrastructure), and any divergence is a golden diff the
harness pins to the exact fixture. Second risk: the builtin-call family (E)
is 630 lines of registry-driven checking with per-package bespoke arms whose
diagnostic ORDER ("infer every argument before the arity check") is what the
goldens record — E transcribes that order, not just the rules.

### Rejected alternatives

- **Wholesale move (all 49 in one diff).** Rejected: un-reviewable golden
  churn; a single wording slip hides in thousands of reordered lines. The
  71-rule history was incremental for this reason.
- **Annotate IR to carry erased evidence so (S) rules relocate too.**
  Rejected: fattens the IR and the `.mfp` wire surface (`IrValue::Call` is
  serialized) for validation-only data; a small HIR shape pass over the
  existing typed HIR is strictly cheaper (D/E).

## Compatibility / Format Impact

None to codegen or wire formats. Diagnostic ORDER on multi-error fixtures
changes deliberately per relocation (goldens re-pinned); set never changes.

## Phases

### Phase 1 — the audit + the harness

- [x] Build `scripts/diag-set-diff.sh` (set-normalized per-fixture diagnostic
      compare over the diagnostic-bearing golden corpus); prove it flags a
      planted wording change and passes a planted reorder (see Corrections
      §C-harness for the two planted runs).
- [x] Audit all 49 codes: for each, read the syntaxcheck implementation and
      the lowering path of its construct; record verdict (V)/(S)/(I) with the
      evidence (what survives into IR), the inference facts needed, and the
      count of corpus fixtures that trip it. §2's hypothesis table replaced
      with the verdicts; letters B/C/D re-scoped in place and E added.

Acceptance: harness proven; the verdict table complete with no hypothesis
rows.
Commit: 6dd04364c

### Phase 2 — three pilot relocations

- [~] Relocate 3 (V)-verdict rules of different shapes end-to-end via the
      recipe; record per-rule cost and reordered-fixture counts:
  - [x] `TYPE_ISOLATED_NOT_VISIBLE` (decl-level): 12 lines in verify's
        function loop; corpus `518 same, 0 reordered, 0 set-diff` (its one
        fixture trips only this rule, so no reorder — as predicted).
  - [x] `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (expression-level; replaces
        `TYPE_UNKNOWN_VALUE`, see Corrections C-pilot-swap): ~45 lines keyed on
        the lowered `$trap_res` temp at the inline-trap `If`, with the
        `$expect_` desugar guard; corpus `517 same, 1 reordered, 0 set-diff` —
        the reorder is `tests/syntax/trap/inline-trap-infallible-builtin-invalid`
        (verify's `SYMBOL_NOT_CALLABLE` now precedes the rule at line 13, both
        in one stream); golden re-pinned in the commit.
  - [ ] `TYPE_THREAD_NOT_SENDABLE` (inference-fact port: sendability + the
        imported sendable bit).
- [~] Tests: full corpus via the harness; package-path (verify unit) test for
      each pilot — `verify::tests::rejects_private_isolated_func`,
      `rejects_isolated_sub`, `accepts_public_isolated_func` (pilot 1);
      pilot 2/3 twins written alongside (`rejects_inline_trap_on_a_non_call`,
      `…_package_constant`, `skips_the_testing_desugared_trap_guard`,
      `rejects_unsendable_thread_message_in_a_parameter`, `…record_field`,
      `rejects_unsendable_message_sent_across_a_thread`,
      `rejects_transfer_on_a_thread_without_a_resource_plane`).

Acceptance: 3 rules live in verify, syntaxcheck impls deleted, corpus
set-equal, goldens re-pinned and listed, `artifact-gate all` byte-identical.
Commit: —

## Validation Plan

- Tests: the harness across the full diagnostic corpus; pilots' package-path
  tests; full suite.
- Coverage check: the audit records fixture counts per rule — a rule with ZERO
  corpus fixtures gets one written BEFORE its relocation (no unguarded moves).
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none yet (D owns it).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Where the (S) shape pass lives** (E creates it, D completes it): a module
  beside lowering, because its typing oracle IS lowering's `expression_type`
  and its callee tables are `LowerContext`'s. Recommended: `src/ir/shape.rs`
  (`ir::shape::check(&HirProject, &LowerFacts) -> Vec<Diagnostic>`), run at
  both seams right before `lower_augmented_project`, collected into the same
  merge in the first-stream position. `src/hir/shape.rs` (the original
  recommendation) would make `hir` depend on `ir::lower`; the name is
  cosmetic, the dependency direction is not. Decided in E with the seam in
  hand.
- **`TYPE_INLINE_TRAP_DEAD_HANDLER` on `expectTrap`'s desugared trap** —
  verify must skip traps whose error binding is a `$expect_` temp (row 32);
  the alternative (moving the rule to the shape pass) forfeits its
  package-path guard for no gain. Decided: skip by temp-name prefix.

## Corrections

- **C-census (2026-08-29).** The plan-writing census was wrong in both
  numbers. The command it recorded for the relocated list —
  `awk '/RELOCATED_TO_IR_VERIFY/,/^\];/' … | grep -cE '"'` → 74 — re-triggers
  its range on a LATER mention of the name (`verify/mod.rs:470`, a doc
  comment) and runs to the next `];`, sweeping up `POISONING_RULES`
  (`mod.rs:614-626`), which names `TYPE_CALL_ARGUMENT_MISMATCH`,
  `TYPE_CALL_ARITY_MISMATCH` and `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`. Those
  three were therefore counted as relocated; they are not (precise range:
  `sed -n 71,160p src/ir/verify/mod.rs | grep -oE '"[A-Z][A-Z_]+"' | sort -u
  | wc -l` → **71**), and they are still emitted by syntaxcheck on the source
  path (probe: `add(1)` / `f(1, 2)` on a FUNC-typed local / `math::abs()`
  each render `TYPE_CALL_ARITY_MISMATCH` exactly once, from syntaxcheck's
  stream). Remaining set: **49**, not 46. They are the largest relocation of
  the whole plan (273 + 283 + 4 fixtures) and get their own letter, **E**,
  together with the shape-pass scaffold they depend on (their named-argument
  sub-forms are (S)).
- **C-shape-typing (2026-08-29).** The plan assumed the (S) residue is purely
  syntactic ("a 200-line HIR shape pass"). Seven (S) rules need an expression
  type and two need callee parameter tables (§3). The shape pass therefore
  borrows lowering's typer via a seam built in E — recorded as the design in
  §3 and Open Decisions rather than left for D to discover.
- **C-verify-has (2026-08-29).** 20 of the 49 codes already have a verify
  implementation on the package path (all 13 `NATIVE_*` via the parity test
  `native_rule_sets_agree_between_syntaxcheck_and_verify`, plus
  `SUB_RETURN_FORBIDDEN`, `TYPE_COLLECTION_OWNERSHIP_VIOLATION`,
  `TYPE_DUPLICATE_FIELD`, `TYPE_RECOVER_TYPE_MISMATCH`, `TYPE_TRAP_FALLTHROUGH`,
  `TYPE_UNKNOWN_VALUE`, `UNREACHABLE_AFTER_EXIT`). For those the recipe is a
  fidelity diff (sub-forms + wording, listed per row) rather than a port —
  cheaper than the plan priced, except that several verify copies have
  drifted wording the goldens do not pin.
- **C-split (2026-08-29).** Six rules are split (V/S) — one sub-shape survives
  lowering, another is erased. The list mechanism silences by CODE, so a
  split rule can only relocate once its (S) half exists; rows 22, 31, 39, 46
  move to D and 47, 48 to E (see §3 "Split rules").
- **C-TESTING (2026-08-29).** The `TESTING_EXPECT_*` family is (S) as the
  hypothesis said, with the added fact that the assertions only exist under
  `mfb test` (TESTING blocks are dropped by `mfb build`), so their fixtures
  are `mfb test` fixtures with `test.log` goldens; the harness handles both
  logs.
- **C-pilot-swap (2026-08-29).** `TYPE_UNKNOWN_VALUE` cannot be a pilot:
  verify's cascade (`ops.rs:117-126`) is gated on verify ITSELF having emitted
  the poisoning rule for the initializer, and for the 301 fixtures that rule
  is the builtin-call arity/argument check, which only E moves — an
  unlisted-poisoner cascade would go silent (300 SETDIFFs). Probe evidence:
  `LET z AS Integer = math::abs()` gets its `TYPE_UNKNOWN_VALUE` from
  syntaxcheck's stream today. The rule's verdict stands (V); its letter is
  **E**, landed after the builtin-call family, and E inherits the
  reorder-scale probe. The expression-level pilot is
  `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` instead (B loses it). Also measured
  while probing: syntaxcheck's plain-assignment form of `TYPE_UNKNOWN_VALUE`
  (`checking.rs:335`) is shadowed by the resolver (`x = 1` on an undeclared
  `x` → `SYMBOL_UNKNOWN_IDENTIFIER`), while the lambda-target form
  (`inference.rs:1522`) is live for both a global (`LAMBDA(v) -> g = v`) and
  an undeclared name — E's row.
- **C-baseline (2026-08-29).** Bench recorded in
  `planning/plan-107-bench-baseline.txt` (trivial 0.62/0.35, one-regex
  31.80/7.26, acceptance 362.71/65.36 debug/release s — slower than
  plan-104's 276/49.85 on the same probes; a host-load difference, and this
  plan is not perf-motivated: the comparison that matters is D's closing
  number against this file). Full-suite row: see its Status cell.
- **C-harness (2026-08-29).** Two planted runs prove `diag-set-diff.sh`:
  (1) wording plant — `EXIT_SUB_IN_FUNC`'s detail `FUNC.` → `FUNC!`
  (`syntaxcheck/checking.rs:224`), rebuilt, `diag-set-diff.sh target/release/mfb
  'tests/syntax/control-flow/*'` → `11 same, 0 reordered, 1 set-diff`
  (`exit-sub-invalid`, the diff naming the changed detail), exit 1;
  (2) reorder plant — verify's stream concatenated BEFORE syntaxcheck's
  (`build/mod.rs:446`), rebuilt, full run → `463 same, 55 reordered, 0
  set-diff`, exit 0. Both reverted. Unmodified tree: `518 same`, 19 s. The
  first cut of the harness dropped the `-ast -ir` flags on replay and produced
  26 false SETDIFFs: `NATIVE_LIBRARY_TARGET_UNCOVERED` is emitted only by the
  full (link) build, so a dump-only run and a full run are NOT
  diagnostic-equivalent — the harness now replays each command line exactly as
  echoed and removes the artifacts it writes.

## Summary

The audit is the plan: 49 verdicts with evidence, a set-equality harness that
makes reorder churn safe to review, and three priced pilots. B/C are then
production-line work, E carries the shape scaffold and the heaviest family,
and D deletes a 14k-line subsystem plus the `debug_assert!`-guarded split the
review flagged.
