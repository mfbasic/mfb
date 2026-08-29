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
- [x] Tests: unit test proving the seam types a HIR expression identically to
      lowering's stamped `IrValue` type on a sample; full suite;
      corpus SAME on every fixture (order-neutral).
      (`ir::shape::tests::walker_types_bindings_exactly_as_lowering_does`
      compares every binding's walker type against lowering's `Bind`/`ForEach`
      type across LET/annotated LET/inline-TRAP LET/FOR promotion/FOR EACH/
      MATCH binding/trap binding/lambda; corpus: `diag-set-diff.sh` → `521
      same, 0 reordered, 0 set-diff`. Full suite: run at the letter's end
      (`6ff122464`, `/tmp/p107-suite-E4.log`) — every cargo test green except
      `artifact_gate_all`, which failed on exactly the two false-cascade
      fixtures (`func_net_url_toString_valid`, `types-union`) that
      C-override-typing below repairs; gate green again after it.)

Acceptance: pass wired, zero corpus change.
Commit: `c2e865703`

### Phase 2 — named-argument cluster

- [x] TYPE_UNKNOWN_ARGUMENT_NAME → shape (justification line; every callee
      class: user, imported, builtin, overloaded builtin); list; delete.
      (`Walker::check_named_arguments` + `callee_params`, which resolves the
      call target in the source checker's order — TESTING call / package
      constant / builtin (canonical name, and only the arms whose argument
      list the checker normalized: `syntaxcheck::checks_builtin_call_arguments`)
      / visible declared FUNC / imported `.mfp` FUNC via the file's import
      binding. The imported table is the UNFILTERED `all_external_signatures`
      (lowering's own is resource-returning only), threaded through
      `collect_diagnostics`; `mfb audit` computes it from the manifest.
      Corpus: 503 same, 18 reordered (exactly the 18 fixtures carrying the
      rule — the shape stream now prints first), 0 set-diff; every regenerated
      golden is a pure line move (`/tmp/p107-movecheck.sh`: sorted old ==
      sorted new).)
- [x] TYPE_DUPLICATE_ARGUMENT_NAME → shape; list; delete.
      (Same walker; the overloaded-builtin form reports only the first
      duplicate and ends the check before unknown names, the per-position
      forms report under the parameter's canonical alias — both exactly as
      syntaxcheck did. Corpus: 519 same, 2 reordered (`func_net_connectTcp_
      invalid`, `project-entry-func-named-args-invalid` — the 2 fixtures
      carrying the rule), 0 set-diff; both regenerated goldens pure moves.)
- [x] Tests: corpus + harness per commit (18 + 2 fixtures); unit tests.
      (`ir::shape::tests`: user / builtin / overloaded-builtin / imported-`.mfp`
      / `general`-arm callee classes, positional-after-named slot walk, the
      private-other-file non-target, nested-call ordering, first-duplicate-only;
      the 6 syntaxcheck twins deleted.)

Acceptance: both codes shape-only; corpus set-equal.
Commit: `ecd3601cd` (TYPE_UNKNOWN_ARGUMENT_NAME); `9a5db7992`
(TYPE_DUPLICATE_ARGUMENT_NAME)

### Phase 3 — builtin-call typing family

- [x] TYPE_CALL_ARITY_MISMATCH: verify (user/function-value/builtin incl. the
      four bespoke arms) + shape (omission form); package-path tests per
      form; list; delete.
      (Landed with the split Corrections C-count-erased describes: every
      COUNT form is `ir::shape`'s on the source path (declared FUNC, builtin
      incl. the four bespoke arms and the source-bodied members, function
      value, the named-argument omission forms); `ir::verify`'s count checks
      (`check_call_arity`, `check_function_value_arity`,
      `builtin_arity_errored`) run on the package path only, with
      syntaxcheck's wording. verify's `check_builtin_call_args` is now the
      full transcription of syntaxcheck's five arms (its ARGUMENT forms land
      with the next box) and sees a call lowering rewrote to a source-companion
      body as the builtin it implements (`builtin_call_target` /
      `registry::rewrite_owner`). syntaxcheck's 10 ARITY report sites deleted,
      its argument normalizers reduced to binding. Corpus: 302 same, 219
      reordered, 0 set-diff; all 219 goldens regenerated as pure line moves.)
- [x] TYPE_CALL_ARGUMENT_MISMATCH: verify + shape (named-on-function-value);
      package-path tests; list; delete; remove `syntaxcheck/builtins.rs`'s
      checker and `inference.rs` call checks.
      (Landed with the split Corrections C-args-erased describes: on the
      source path EVERY argument-type form is `ir::shape`'s — the five builtin
      arms transcribed over the HIR argument list with lowering's
      `expression_type` (`Walker::check_builtin_call`, incl. the byte-literal
      retry, the collections predicate form, term's per-position check and
      the `IMPORT astrings` requirement, thread's entry), the declared-FUNC
      per-position form over the SUPPLIED arguments, the function-value
      per-position and named forms — with `syntaxcheck::compatible` /
      `expression_compatible` ported (`Walker::compatible`, union variants and
      same-declaration identity from the HIR + imported type table).
      `ir::verify`'s IR-level forms stay for the package path only
      (`emit_argument_mismatch`). The shape pass's typing seam now uses the
      UNFILTERED imported-signature table (a `pkg::worker` reference types as
      its ISOLATED FUNC, as the checker typed it) and strips a resource
      local's `STATE` before comparing, as the checker did. syntaxcheck's 12
      report sites deleted (the builtin arms keep only their return-type
      inference; `check_function_value_call` / `check_call` infer arguments
      only). Corpus: 281 same, 240 reordered, 0 set-diff; all 240 goldens
      regenerated as pure line moves.)
- [x] Tests: corpus + harness (all 283/273 fixtures classified; every REORDER
      listed in the commit; zero SETDIFF); `artifact-gate all`; full suite.
      (Corpus zero SETDIFF at both landings — 219 then 240 REORDER, all pure
      moves; unit tests: `ir::shape` (58), `syntaxcheck`, `ir::` green.
      `artifact-gate all` 0 diffs at `0b2360912` (the seam-gap landing); full
      suite at the letter's end — see Phase 1's test line.)

Acceptance: both codes verify/shape-only; syntaxcheck's builtin checker gone;
corpus set-equal; gate byte-identical.
Commit: `f70a3919f` (TYPE_CALL_ARITY_MISMATCH); `4199d19e9` +
`0b2360912` (TYPE_CALL_ARGUMENT_MISMATCH and the seam gaps its gate exposed)

### Phase 4 — TYPE_UNKNOWN_VALUE (added: A's verdict table routes it here, "after them")

- [x] TYPE_UNKNOWN_VALUE → shape (the cascade over the checker's `Unknown`
      verdict: initializer / RETURN / parameter-default forms and the two
      not-a-local-binding target forms) + verify (its own operator-poisoning
      cascade, source path narrowed to Binary/Unary nodes); list; delete
      syntaxcheck's 7 sites. Justification: the checker's `Unknown` is a
      verdict the IR does not carry — lowering stamps a lenient type on a
      failed builtin call, a `$`-temp on a trapped one, `Integer` on a
      non-numeric arithmetic, and a typed variant on a MATCH arm the checker
      never bound. See Corrections C-cascade for the verdict reconstruction
      and the census that drove it.
- [x] Tests: corpus + harness (the 301 fixtures: 0 set-diff, 238 reordered,
      goldens regenerated as pure moves); `ir::shape` cascade tests
      (constructor, thread entry, builtin failure); the pipeline oracle's
      syntaxcheck tests green.

Acceptance: shape/verify-only; corpus set-equal.
Commit: `6ff122464`; the two false cascades its gate exposed are repaired in
the commit that opens D's Phase 1 (C-override-typing; hash recorded there).

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

- **C-count-erased (2026-08-29, Phase 3).** A's verdict that the user-FUNC,
  function-value and builtin ARITY forms "survive lowering" is false for the
  COUNT: `normalize_local_call_arguments` drops the positional arguments a
  declared signature cannot take and `lower_local_call_arguments` fills every
  omitted slot from its default (`src/ir/lower.rs`), so a declared-FUNC call's
  lowered `args.len()` is never the source's count except when a required slot
  vanishes; and the builtin Call arm pads optional trailing arguments with
  their defaults ("Pad optional trailing arguments (`tls.connect` defaults)
  with constants so the fixed-ABI runtime helper always receives …"), so a
  too-few builtin call is padded to a valid count (`tls::listen()` lowered
  with 1 argument; the first corpus run lost 59 fixtures' too-few ARITY —
  csv/crypto/encoding/http/json/net/regex/tls/vector). Measured by the
  harness (`/tmp/p107-phase-E3a.log`, first run: 59 SETDIFF). Consequence:
  every count form is a shape rule on the source path; `ir::verify` keeps a
  structural count check for the package path only (`TypeEnv::source_path`),
  since there is no HIR there. The named-argument omission forms were already
  shape's. Only the argument TYPES of a builtin call survive lowering and are
  verify's.
- **C-args-erased (2026-08-29, Phase 3).** A's "argument TYPES survive
  lowering" is false for the diagnostic the checker produced. Listing
  TYPE_CALL_ARGUMENT_MISMATCH with verify's IR-level forms on the source path
  left 24 SETDIFF fixtures (`/tmp/p107-phase-E3b.log`, first run): lowering
  pads a builtin's optional trailing arguments with defaults (`csv.parse(TRUE)`
  reports `(Boolean, String, String)`, `tls.connect()` `(Integer, String)`),
  coerces a literal argument to the parameter's type before the IR exists
  (`fs.appendBytes(1, [1, 2])` reports `List OF Byte`, `term.setForeground("x",
  1, 2)` reports `Byte, Byte`; `fs.pathJoin([1, 2])` becomes a
  TYPE_LIST_ELEMENT_MISMATCH pair with no ARGUMENT at all), fills a declared
  function's defaults (an ill-typed default then reports as a call argument —
  `user-function-default-args-invalid`, `types-default-value-invalid`),
  lowers `error(code, message)` to record constructors and
  `thread::transfer` to the send plane (no call to judge), and drops a
  duplicate-named argument (`net.connectTcp`'s line 12). Consequence: the
  source-path argument-type forms are shape rules over the HIR, and verify's
  are package-path only — the same split the counts took. The plan's
  "verify checks both paths" holds for the package path's ABI defense, not
  for source-path parity.
- **Seam gaps the ARGUMENT landing exposed (2026-08-29, Phase 3).** The
  artifact gate at `4199d19e9` failed 6 valid fixtures (their `.ast`/`.ir`
  missing: the build was rejected), all shape-pass false rejections from two
  gaps between lowering's `expression_type` and the checker's inference:
  (1) `expression_type` typed `e.source` and every `ErrorLoc` member as
  `Unknown` (it knew only `Error.code`/`.message`), so
  `toString(where.line)` resolved no overload — fixed in the seam itself
  (`lower.rs`, the `Error`/`ErrorLoc` member arms), which also stamps the
  lowered `MemberAccess` with the right type; (2) the checker compared a
  resource's type with its ` STATE T` clause stripped on BOTH sides (an
  imported `db::exec(h AS Db STATE DbInfo)` takes a `Db`), which the port
  had done for the actual only — `Walker::compatible` now strips both. The
  seam fix is a lowering-inference correction: any `.ir` golden that
  annotates such a member access re-baselines with it (a typed annotation
  where `Unknown` stood), never a `.ncode`.
- **C-rewritten-targets (2026-08-29, Phase 3).** A source-bodied builtin
  member (`Body::Mfb`/`Body::Rewrite`, the `astrings` Tier-B transforms, the
  `term::drawText(AttributedString)` bridge) lowers to a call whose target is
  the companion body's internal symbol (`#encoding_base32Decode`,
  `#astrings_upper`, `#term_drawTextAttr`) while keeping the BUILTIN argument
  normalization. verify's builtin family therefore resolves the target back
  to the member (`compat::builtin_call_target`: `registry::rewrite_owner` +
  `strings::tier_b_transform_owner` + the term bridge) and reports the
  member's name; the user-FUNC arity/argument checks skip such targets (only
  the body's `STATE` agreement still binds them — the
  `http_async_wrongarg_invalid` golden's `TYPE_STATE_MISMATCH` for
  `#http_pump` stays).
- **C-thread-entry (2026-08-29, Phase 3).** `thread.start`'s entry form of
  TYPE_CALL_ARGUMENT_MISMATCH ("entry point must be an exported ISOLATED FUNC
  from an imported package") is (S): a `self::worker` export and a bare
  `worker` both lower to `FunctionRef{name: "worker"}` (lowering's
  `canonical_import_name` strips `self.`), so the IR cannot tell the accepted
  form from the rejected one. It goes to the shape pass (with the
  count-check gate the checker had — a bad entry ends the call's checks);
  verify keeps an IR-evidence superset check (`FunctionRef` typed `ISOLATED
  FUNC`) on the package path only. `ExternalSignature` gained `sub` (a SUB
  export is not a FUNC) and `ExternalFunctionParam` gained `has_default` (the
  arity rule's optional-parameter fact), both read straight off the `.mfp`
  export table syntaxcheck already consulted.
- **C-alias-spelling (2026-08-29, Phase 3).** syntaxcheck spelled a callee as
  the source wrote it (`sh.area` under `IMPORT shapes AS sh`, `self.f`);
  verify's IR target is canonical (`shapes.area`, `f`). For the verify-side
  forms (argument types) an aliased or `self::` call therefore renders the
  canonical name — the IR's truth. No fixture in the 556-fixture family
  spells a call through an alias (`/tmp/p107-alias-fixtures.sh`: only
  `func_thread_start_self_invalid`, whose messages name no callee), so the
  corpus is unaffected; the shape-side forms keep the source spelling.
- **C-test-oracle (2026-08-29, Phase 3).** `syntaxcheck::testutil::check_src`
  (and the crate-level `testutil::check_src` that four syntaxcheck test
  modules use, now delegating to it) runs the build path in the build's order
  — registry augmentation, monomorphization, then `ir::shape`, syntaxcheck
  (with its late-pass augmentation) and `ir::verify` on the lowered IR — and
  concatenates their codes, so a rule's unit tests keep passing as it
  relocates; D moves the surviving tests out of `syntaxcheck` with the
  module. Three tests asserted syntaxcheck-only properties and switched to
  the new `syntaxcheck_codes` oracle (the bug-43 "must not be rejected by
  syntaxcheck" guards, and a monomorph-rejected overload program that only
  exercised the checker's fallback branch); one test's source (`Socket` for
  `net::Socket`) never resolved in a real build and was corrected.
  syntaxcheck's builtin arms keep their return-TYPE inference (the rest of
  syntaxcheck still types expressions until `TYPE_UNKNOWN_VALUE` moves); only
  their reports were deleted.
- **Bug found and fixed on the way (2026-08-29, Phase 3).** The pipeline
  oracle exposed `syntaxcheck::types::types_tests::default_list_of_fixed_literal`
  as asserting something the real compiler rejected: `FUNC g(a AS List OF
  Fixed = [1, 2])` failed the build with `TYPE_DEFAULT_VALUE_MISMATCH`
  (reproduced on the pre-plan baseline binary,
  `/tmp/p107-baseline/target/debug/mfb`, so pre-existing). Root cause:
  `lower_param` lowered a default WITHOUT the parameter's expected type
  (`lower_expression`), so the literal list stayed `List OF Integer`, while
  the call site's default fill (`lower_local_call_arguments`) and every other
  typed slot use `lower_expression_with_expected`. Fixed in `lower_param`;
  the artifact gate classifies any codegen delta as this fix's.
- **C-cascade (2026-08-29, Phase 4).** TYPE_UNKNOWN_VALUE is the checker's
  CASCADE: "the initializer/RETURN/default typed `Unknown`". A census of the
  corpus's 839 cascades (`/tmp/p107-unknown-triggers2.sh`: the other rules
  at the same line) found 730 following a builtin count/type failure, ~35
  following an operator/constructor rule of `ir::verify`'s, 13 with no
  co-rule at all (member reads the checker could not type). The verdict is
  therefore split by who can see it: `ir::shape` reconstructs the checker's
  `Unknown` from lowering's `expression_type` PLUS the verdicts the seam does
  not carry — a call its own rules typed Unknown (tracked per call node), a
  package constant in call form (typed), a constructor/`WITH` the checker
  would not construct (`Ok`/`Result`, a compiler-owned record, a union/enum/
  unknown name — the read-only nominals stay typed), an arithmetic on Money
  the lattice rejects or on a `Nothing`/untyped operand, `.state` on a plain
  `LET` of a stateful resource (the checker kept STATE on the `RES` axis
  only — bug-376's displaced error, pinned by its fixture), a MATCH arm the
  checker never bound (`Ok`/`Error`, a non-union scrutinee, an unknown
  variant), and a bare built-in predicate typed from a FUNC expectation;
  `ir::verify` keeps the cascade for the operator nodes its own rules poison
  (Binary/Unary — on the source path only those, so the two never double a
  line) and skips `$`-temp binds (B's note). Two seam-side bugs surfaced and
  were fixed on the way: `AttributedString` was comparable to `ir::verify`
  (an `=` on two attributed strings reached codegen: "native comparable
  comparison does not support type 'List OF AttrSpan'" — syntaxcheck's
  cascade had been the only rejecter), and the seam test's program used an
  unresolvable `collections::map` and constructor-call syntax.
- **Note from B (2026-08-29), for `TYPE_UNKNOWN_VALUE`'s relocation here:**
  lowering now binds a stray `RECOVER`'s value to a `$recover_stray` temp
  (B Corrections). verify's initializer cascade ("Initializer for binding
  `{name}` does not have a known type") must skip `$`-temps — syntaxcheck never
  emitted that cascade for a RECOVER value (nor for `$trap_res`/`$trap_val`,
  whose cascades it reports against the user's binding instead).

- **C-override-typing (2026-08-29, after Phase 4).** `artifact-gate all` at
  `6ff122464` failed on two fixtures with a FALSE TYPE_UNKNOWN_VALUE cascade
  (`rt-behavior/net/func_net_url_toString_valid`: "Initializer for binding
  `rendered` does not have a known type"; `rt-error/types/types-union`:
  "RETURN value does not have a known type"). Root causes, both in the typing
  seam the pass borrows: (1) `lower::expression_type`'s general arm typed a
  call to a builtin OVERRIDE (`toString(Url)`, a user FUNC shadowing a builtin
  spelling) as `None` because it only consulted the builtin registry — the
  checker fell back to the visible declared FUNC; the arm now falls back to
  the declared override's return type. (2) the shape pass's union `variants`
  came from the declaration's own variant list, so a variant reached through
  `INCLUDES` was "unknown" to `constructor_typed`/`compatible`; `TypeShape`
  now carries the INCLUDES-expanded variant set (`ir::shape::tests::
  cascade_spares_typed_seam_cases` pins both). Corpus and gate re-run green
  with D's first landing. Lesson: the gate is the only check that runs the
  cascade over the rt-* fixtures' typed programs — a corpus of `syntax/*`
  goldens cannot see a false cascade on a program that compiles.

## Summary

The letter A's audit created: the shape pass is born with its typing seam and
its first two rules, and the plan's heaviest family — the registry-driven
builtin-call checks behind 556 fixtures — moves to verify where a hostile
package's calls are finally checked too.
