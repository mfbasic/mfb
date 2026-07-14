# MIGRATION COMPLETE — 2026-07-04

**Post-completion (69db1207):** `src/typecheck` is RENAMED `src/syntaxcheck` —
it holds only the source-syntax rules (constructs total lowering erases, which
therefore cannot exist in IR or `.mfp` packages). All semantics live in
`ir::verify`. The relocated-rule emission sites were DELETED (c9610448) and the
zombie ownership dataflow stripped (9ffc123d): 8,050 → 6,909 lines.


Every rule below is dispositioned. Final state:
- **65 rule ids RELOCATED** (`RELOCATED_TO_IR_VERIFY`): ir::verify is the sole
  rejecter on BOTH the source-lowered IR and decoded package IR.
- **Partial ports** (package-path defense; rule id stays with typecheck for the
  erased source-syntactic arm): CALL_ARITY/CALL_ARGUMENT (named-arg arms),
  UNKNOWN_VALUE (inference-depth arm), DUPLICATE_FIELD (constructor named-arg),
  READ_ONLY_RECORD_CONSTRUCTOR (Error arm), SUB_RETURN_FORBIDDEN (bare/NOTHING),
  UNREACHABLE_AFTER_EXIT (EXIT SUB/FUNC), TRAP_FALLTHROUGH (trap-tree shapes),
  RECOVER_TYPE_MISMATCH (bare arms), COLLECTION_OWNERSHIP (thread-element arms),
  NATIVE_FREE_INVALID (deallocator ctypes), all 8 NATIVE_* (slot-level spans).
- **Cannot relocate** (construct erased by total lowering, no IR analogue):
  EXIT_FUNC_FORBIDDEN, EXIT_SUB_IN_FUNC, SUB_CANNOT_RETURN_VALUE,
  DUPLICATE/UNKNOWN_ARGUMENT_NAME, INLINE_TRAP_FALLS_THROUGH/REQUIRES_FALLIBLE/
  ON_INLINED_BUILTIN, RECOVER_OUTSIDE_INLINE_TRAP, LAMBDA_CAPTURE_UNSUPPORTED,
  THREAD_NOT_SENDABLE, RESULT_NOT_USER_VISIBLE (resolver-owned),
  PACKAGE_INVALID (package-decode layer, audit-1 hardened).
- Verification: census (verify_vs_typecheck_diagnostic_parity, CENSUS_MISSING/
  CENSUS_EXTRA modes) shows every remaining MISSING is a documented erased arm;
  artifact gate byte-identical (1020 tests, 0 diffs); all 79 *-invalid fixtures
  green; ir unit tests pass.

## Ported to `ir::verify`, NOT yet relocated (5) — checks exist and run on the package path; blocked from removal because the port is partial or the rule id is shared with a source typecheck still owns

[x] `TYPE_CALL_ARGUMENT_MISMATCH` — FINAL (partial): user calls + ALL builtin families (term/math/bits/vector/strings/encoding/io/fs/net + collections/general with typecheck-exact messages) checked on IR = full package-path coverage; CANNOT relocate the rule id — the named-argument arm (`f(x := ...)` mis-binding) is source syntax lowering erases (census: the only MISSING/EXTRA occurrences are named-arg fixtures)
[x] `TYPE_CALL_ARITY_MISMATCH` — FINAL (partial): user + all builtin families incl. collections/general checked on IR = full package-path coverage; CANNOT relocate — named-arg normalization failures ("has 0 argument(s)" after dedup) are source-syntactic (3 census-MISSING, all in named-arg fixtures)
[x] `TYPE_REQUIRES_COMPARABLE` — RELOCATED; equality + map-key comparability (nested Map OF scan at Bind/global/param/field/MapLiteral, explicit-annotation gated) + collections contains/replace/find element comparability
[x] `TYPE_MATCH_NOT_EXHAUSTIVE` — RELOCATED; enum/union (typecheck-exact missing lists: union declaration-order join, enum sorted `T.m`) + open-type arm (any known non-union/enum needs unguarded CASE ELSE) + Result-scrutinee cascade suppression
[x] `TYPE_USE_AFTER_MOVE` — RELOCATED; cross-branch MaybeMoved joins (fall-through-only merge, diverging branches excluded), RES-rebind moves (`RES b = a` via resource_owners), user-declared native close ops (resource_closers from IrProject.native_resources); typecheck's literal duplicate emissions collapsed to one (Option-B reviewed)

## Not ported yet (80)

**Type-relational — need the compat/inference algebra (mostly already in `ir::verify`), same pattern as the 11 done (18)**
[x] `TYPE_ASSIGNMENT_MISMATCH` — relocated; Assign/AssignGlobal/StateAssign mismatch via compat algebra; `$`-temp targets skipped (RECOVER slot = RECOVER_TYPE_MISMATCH's rule)
[x] `TYPE_ASSIGN_REQUIRES_MUT` — relocated; muts map threaded through check_ops (params/loop-vars/trap-binding immutable, capture binds unknown)
[x] `TYPE_BINDING_MISMATCH` — relocated; needed IrBinding/Bind `explicit_type` (inferred bindings can't mismatch) + diagnostic merge/collect infra
[x] `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` — relocated; positional args (named already reordered by lowering) + WITH updates
[x] `TYPE_CONSTRUCTOR_ARITY_MISMATCH` — relocated; EXACT arity (records have no field defaults) via new ordered record_field_lists map
[x] `TYPE_CONSTRUCTOR_REQUIRES_RECORD` — relocated; declared union/enum names rejected, unknown names skipped
[x] `TYPE_DEFAULT_VALUE_MISMATCH` — relocated; param-default vs declared type in the collect_diagnostics param loop; new fixture types-default-value-invalid
[x] `TYPE_MATCH_PATTERN_MISMATCH` — relocated; declared-type CASE names checked as union variants (wrong-variant / non-union-scrutinee), literal patterns via compat; new fixture control-flow-match-pattern-invalid
[x] `TYPE_READ_ONLY_RECORD_CONSTRUCTOR` — FINAL (partial): handle types (TermColor/TermSize/Address/MapEntry) checked on IR (package-path defense); Error/ErrorLoc arm CANNOT relocate (lowering synthesizes Constructor{Error} for error()/traps — user construct indistinguishable on IR)
[x] `TYPE_READ_ONLY_RECORD_UPDATE` — relocated; WithUpdate on Error/ErrorLoc/read-only types, with infer_type fallback when lowering stamps Unknown; added Error/ErrorLoc builtin field tables (RESTORED UNKNOWN_FIELD coverage the 415c1574 cutover silently lost)
[x] `TYPE_CONDITION_REQUIRES_BOOLEAN` — relocated; IF/WHILE/LOOP UNTIL/WHEN-guard sites, statement-specific message prefixes
[x] `TYPE_FOR_REQUIRES_NUMERIC` — relocated; bounds resolved through `$for` temp binds; Unknown-typed locals skipped
[x] `TYPE_FOR_EACH_REQUIRES_COLLECTION` — relocated; `List OF `/`Map OF ` prefix check (MapEntry excluded); Unknown skipped (LINK-package locals!)
[x] `TYPE_FOR_STEP_ZERO` — relocated; step resolved through the `$for` temp bind (temp_consts map); new fixture control-flow-for-step-zero-invalid
[x] `TYPE_UNKNOWN_VALUE` — FINAL (partial): the erroneous-value cascades (poisoning-rule mechanism: operator/comparable/call/constructor/read-only failures → UNKNOWN_VALUE at the consuming Bind/Return) checked on IR = 13 of 22 census occurrences; CANNOT relocate the rule id — the pure-inference-unknown arm needs typecheck's full type knowledge (external LINK signatures, generic MapEntry fields) that lowering doesn't thread; emitting on bare lowering-Unknown would false-reject valid LINK-package programs (proven by the sqlite3 fixture)
[x] `TYPE_UNKNOWN_ENUM_MEMBER` — relocated; `Enum.Member` selection where target is the bare unshadowed enum name; new fixture types-enum-member-invalid
[x] `TYPE_MEMBER_NOT_VISIBLE` — RELOCATED; private-field access + private-type/hidden-field constructors cross-file (IrType.file/visibility + IrField.visibility); new fixture types-member-visibility-invalid (rule had NO fixture)
[x] `SYMBOL_NOT_CALLABLE` — relocated; package-constant calls (`math.pi()`) + known non-FUNC local/param calls
[x] `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` — (unlisted in the original census breakdown) CANNOT relocate: the escaping/non-escaping capture proof is front-end escape analysis; the IR's Capture by_ref flag records only its conclusion

**Literal range — check `Const` value against its type (8)** — DONE (relocated, commit)
[x] `TYPE_BYTE_LITERAL_OVERFLOW`
[x] `TYPE_BYTE_LITERAL_UNDERFLOW`
[x] `TYPE_INTEGER_LITERAL_OVERFLOW`
[x] `TYPE_FLOAT_LITERAL_OVERFLOW`
[x] `TYPE_FLOAT_LITERAL_UNDERFLOW`
[x] `TYPE_FIXED_LITERAL_OVERFLOW`
[x] `TYPE_FIXED_LITERAL_UNDERFLOW`
[x] `TYPE_UNARY_OPERATOR_UNKNOWN`

**Declaration well-formedness — check the IR type/param tables (13)**
[x] `TYPE_DUPLICATE_VARIANT` — relocated; ported expanded-union include conflict detection
[x] `TYPE_DUPLICATE_FIELD` — FINAL (partial): WITH-update duplicates checked on IR (package-path defense); the constructor named-argument arm (`Point[x := 1, x := 2]`) is erased by lowering's named→positional normalization, so the rule id stays with typecheck
[x] `TYPE_UNION_INCLUDE_REQUIRES_UNION` — relocated; IrType.file added for source span
[x] `TYPE_UNION_MEMBER_REQUIRES_TYPE` — relocated; reports at variant.loc.line
[x] `TYPE_ENUM_REQUIRES_MEMBER` — relocated
[x] `TYPE_PARAM_REQUIRES_TYPE` — relocated; param.type_=="Unknown" (lowering's missing-AS stamp), lambdas included
[x] `TYPE_DEFAULT_ARG_ORDER` — relocated; defaulted-then-plain param order on the IrParam list
[x] `TYPE_DUPLICATE_ARGUMENT_NAME` — CANNOT relocate (erased): named-argument syntax is normalized away by lowering; stays in typecheck (same class as the CALL arity/argument named-arg arms)
[x] `TYPE_UNKNOWN_ARGUMENT_NAME` — CANNOT relocate (erased): same named-argument class
[x] `TYPE_BINDING_REQUIRES_TYPE_OR_VALUE` — relocated; Bind/global with no annotation and no value (`$`-temps skipped)
[x] `TYPE_LET_REQUIRES_VALUE` — relocated; immutable Bind/global with annotation but no value
[x] `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` — relocated; is_defaultable ported to type strings (primitives/collections/records recurse; FUNC/Result/RES/Thread/union/enum/resource no)
[x] `TYPE_FUNC_REQUIRES_RETURN_TYPE` — relocated; kind==func && returns=="Unknown" (`$lambda` bodies skipped — computed returns)

**Control flow / returns (12)**
[x] `EXIT_FUNC_FORBIDDEN` — CANNOT relocate (erased): `EXIT FUNC` lowers to NOTHING (Vec::new())
[x] `EXIT_SUB_IN_FUNC` — CANNOT relocate (erased): `EXIT SUB` lowers to Return{None}, indistinguishable from a bare RETURN
[x] `EXIT_NO_MATCHING_LOOP` — relocated; loop_stack (RefCell) tracks enclosing kinds; ExitLoop carries LoopKind
[x] `CONTINUE_NO_MATCHING_LOOP` — relocated; same stack
[x] `SUB_RETURN_FORBIDDEN` — PARTIAL: value-carrying `RETURN <v>` in a SUB checked on IR (package-path defense); bare RETURN / RETURN NOTHING lower to Return{None} = EXIT SUB (erased) → rule id stays typecheck
[x] `TYPE_SUB_CANNOT_RETURN_VALUE` — CANNOT relocate (erased): a SUB's `AS T` clause is dropped (returns stamped "Nothing")
[x] `TYPE_SUB_HAS_NO_VALUE` — relocated; sub Call in value position (allow_sub_call flag set at Eval statements and `$`-temp trap binds); residual gap: a sub call under a *value-position* TRAP (`LET x = sub() TRAP`) is not distinguishable from statement-position trap machinery — no fixture covers it
[x] `UNREACHABLE_AFTER_EXIT` — PARTIAL: ops after ExitLoop/ContinueLoop in a block checked on IR (4 of 6 census occurrences); after EXIT SUB/FUNC erased → rule id stays typecheck
[x] `TYPE_FUNC_MISSING_RETURN` — relocated; block_always_returns flow analysis (Return/Fail/ExitProgram, both-branch If, exhaustive MATCH incl. full enum/union coverage without ELSE, TRAP body); `AS Nothing` funcs and `$lambda` bodies exempt
[x] `TYPE_EXIT_PROGRAM_REQUIRES_INTEGER` — relocated; ExitProgram code vs Integer
[x] `EXIT_PROGRAM_CODE_OUT_OF_RANGE` — relocated; constant code 0..=255 (integer_constant_value on IR)

**TRAP / PROPAGATE / RECOVER (8)**
[x] `TYPE_TRAP_FALLTHROUGH` — PARTIAL (package-path defense): function-level Trap-op body must always-return; the inline-trap tree and sub-trap shapes diverge from typecheck's flow view (1 EXTRA + 1 MISSING on those fixtures) → rule id stays typecheck
[x] `TYPE_INLINE_TRAP_FALLS_THROUGH` — CANNOT relocate (erased): the inline handler is treeified into If/Assign/Fail ops with no handler boundary marker
[x] `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` — CANNOT relocate: callable fallibility (does the callee FAIL?) is a front-end property not carried in IR signatures
[x] `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` — CANNOT relocate: same class (inline-lowered builtin identity is erased at the call site)
[x] `TYPE_PROPAGATE_REQUIRES_TRAP` — RELOCATED; outside-trap PROPAGATE lowers to Fail(Local("$error")) with the sentinel unbound — detectable exactly
[x] `TYPE_RECOVER_OUTSIDE_INLINE_TRAP` — CANNOT relocate (erased): the no-target fallback lowers to a plain Eval, indistinguishable
[x] `TYPE_RECOVER_TYPE_MISMATCH` — PARTIAL (package-path defense): the Assign-into-`$trap_val` slot arm checked (1 of 2 fixture shapes); bare/extra-RECOVER arms erased → rule id stays typecheck
[x] `TYPE_FAIL_REQUIRES_ERROR` — RELOCATED; Fail op error value vs Error via compat; new fixture control-flow-fail-propagate-invalid (no fixture existed)

**Resource annotation (7)**
[x] `TYPE_RESOURCE_REQUIRES_RES` — RELOCATED; RES-ness = resource_owners membership; binding + collection-element/value + return-type axes (recursive walker)
[x] `TYPE_RES_REQUIRES_RESOURCE` — RELOCATED; gated on provably_data_type (unknown names may be external-package resources — the sqlite3 fixture caught the naive version)
[x] `TYPE_RESOURCE_BORROW_INVALIDATE` — RELOCATED; borrows = resource params + FOR EACH element vars + Float-owned (collection-floated) RES bindings; close/RETURN of a borrow rejected
[x] `TYPE_RESOURCE_ELEMENT_NOT_OWNER` — RELOCATED; RES-bind of collections.get/getOr borrow + RETURN of a get borrow + temporary (non-Local) element in a `List OF RES` literal
[x] `TYPE_STATE_INVALID` — RELOCATED; declaration arm (STATE payload must be defaultable) + StateAssign-on-stateless arm
[x] `TYPE_UNION_STATE_FORBIDDEN` — RELOCATED; " STATE " on a union-typed RES binding
[x] `TYPE_COLLECTION_OWNERSHIP_VIOLATION` — PARTIAL (package-path defense): map-key resource/thread containment checked on IR (all fixtured occurrences); the element/value THREAD-containment arms have no fixtures and stay with typecheck

**Result-type visibility / threads (5)**
[x] `TYPE_RESULT_IS_IMPLICIT` — RELOCATED; Constructor{Ok|Result} rejected
[x] `TYPE_RESULT_NOT_MATCHABLE` — RELOCATED; CASE Ok/Error/Err arms (unless a real union variant of the scrutinee)
[x] `TYPE_RESULT_NOT_USER_VISIBLE` — resolver-owned: the resolver rejects `Result`/`Ok` in user type positions before lowering (typecheck's arm is a belt-and-braces invariant with no reachable fixture); nothing to relocate
[x] `TYPE_THREAD_NOT_SENDABLE` — CANNOT relocate: sendability (resource sendable flags + copyability over thread channel types) is a front-end registry property; the Thread type-string forms it checks never reach lowering with a fixture (0 census)
[x] `TYPE_THREAD_RESULT_REMOVED` — RELOCATED; `.result` member access on Thread-typed targets

**Native LINK ABI — check the IR link tables (8)** — ALL DONE as package-path defense (check_link_functions over IrProject.link_functions; typecheck keeps the rule ids for slot-level source spans, which the IR does not carry)
[x] `NATIVE_ABI_NO_RESULT` — ported (value-producing wrapper needs exactly one result marker)
[x] `NATIVE_ABI_RESULT_MARKER` — ported (return slot must be OUT; at most one marker)
[x] `NATIVE_ABI_UNBOUND_PARAM` — ported (every wrapper param maps to a slot)
[x] `NATIVE_ABI_UNBOUND_SLOT` — ported (every slot binds to param/CONST/result; census-matched)
[x] `NATIVE_CONST_OUT` — ported (CONST pin cannot be OUT)
[x] `NATIVE_CONST_UNKNOWN_SLOT` — ported (CONST names a real slot)
[x] `NATIVE_CPTR_ESCAPE` — ported (C ABI types only inside ABI slots; census-matched)
[x] `NATIVE_FREE_INVALID` — PARTIAL: empty-symbol arm ported; the deallocator param/return ctype arms are erased (IrFree carries slot+symbol only)

**Package (1)**:
[x] `PACKAGE_INVALID` — NOT an IR rule: it guards raw .mfp metadata ingestion during typecheck's import scan (unreadable type/resource tables). The package-decode layer (audit-1 PKG-01..07 hardening + signature verification) owns that surface; nothing to relocate.
