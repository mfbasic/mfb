> **STATUS: COMPLETE (2026-07-04).** `src/typecheck` is now `src/syntaxcheck` (69db1207) — the surviving module is the source-syntax checker only; all semantics are in `ir::verify`. 20-A/B/C/D landed earlier; 20-E..I and the
> per-rule cutover are DONE — see `planning/migrate.md` (the porting ledger) for
> the final per-rule dispositions. 65 rule ids are relocated with `ir::verify`
> as sole rejecter on both paths; the remainder are partial ports
> (package-path defense for arms whose source syntax lowering erases) or
> documented cannot-relocate dispositions. G-gate outcomes: G1 via the census
> test (every remaining typecheck-only emission is a documented erased arm);
> G2 met for every relocated rule — all 101
> emission sites for the 65 relocated ids are DELETED from typecheck
> (grep gate: zero hits; report() carries a debug_assert guarding the
> invariant); typecheck keeps exactly elaboration plus the erased-syntax
> arms, which the original plan text did not anticipate (total lowering
> DELETES constructs like `EXIT FUNC`, named arguments, and inline-trap
> boundaries, so no IR checker can see them); G3 held to skip-if-unknown-with-census-proof (full
> G3 needs external LINK signatures threaded into the checker's lowering —
> follow-on work); G4 held (order-only re-goldens where relocated rules moved
> to the appended stream, plus reviewed Option-B improvements: duplicate
> collapse, restored UNKNOWN_FIELD coverage, span fidelity via
> IrType/IrBinding.file); G5 byte-identical throughout; G6 package battery
> extended per family.

# MFBASIC Typed-IR Single Semantic Checker Plan

Last updated: 2026-07-03
Overall Effort: huge (>3d — realistically 1–2 weeks)
Effort (this master doc): it is the roadmap; execution happens in the lettered
sub-plans enumerated in §7, each sized small/medium.

This plan does what `planning/plan-19-ir-semantic-verification.md` set out to do
and did **not** finish: make the IR a **typed IR** and have **exactly one
semantic checker**, operating on that IR, verify every path that produces IR —
the source front end and the package decoder — before lowering. plan-19 shipped
only a conservative *subset* checker (`src/ir/verify/`, `ir::verify_semantics`)
that runs alongside the AST type checker, so today there are **two** places that
enforce overlapping semantic rules and they can drift. This plan removes that
duplication: the 96 semantic rule ids live in one implementation on the
IR, and `src/typecheck/` is reduced to type *elaboration* (inference that
annotates the program) with **no rejection logic of its own**.

The single behavioral outcome a correct implementation produces: **deleting a
semantic rule from `src/typecheck/` and re-adding it to `src/ir/verify/` leaves
exactly one implementation of that rule; a program is rejected by the IR checker
and by nothing else; valid programs compile to byte-identical native output.**

It complements:

- `./mfb spec language error-model` — typing, `Result`/`PROPAGATE`/effect
  agreement (the rules being relocated; canonical under `src/docs/spec/**`)
- `./mfb spec language resource-management` — resource linearity / drop-once
- `./mfb spec language pattern-matching` — `MATCH` exhaustiveness
- `./mfb spec package verifier-rules` — the merge-time verification plan-19 added
- `./mfb spec diagnostics error-codes` — the `errorCode::` constant registry
  (build input); every relocated rule keeps its existing rule id
- `./mfb spec architecture native-ir` — the IR shape this plan re-types

## Why plan-19 came out a subset (root cause, so this plan can't repeat it)

The IR is a **lowering target, not a typed IR**. It carries a result type on
some value nodes (`Const`, `Constructor`, `UnionWrap`, `UnionExtract`,
`WithUpdate`, `List/MapLiteral`, `Closure`, `Capture`, `LocalRef`,
`FunctionRef`) but **not** on `Call`, `CallResult`, `Binary`, `Unary`,
`MemberAccess`, `ResultIsOk`, `ResultValue`, `ResultError` — nor on the
reference variants `Local`/`Global`, whose types live in the enclosing
`Bind`/param/`IrBinding` declarations (verified: `src/ir/value.rs`). To check any
type-relational rule (operand compatibility, argument/parameter match,
assignment/return match, comparability) you must know the type of every
computed subexpression. On an under-typed IR the only options are (a) re-infer
types bottom-up — i.e. re-implement `typecheck/inference.rs`, the "second
checker" trap — or (b) skip when the type is unknown, which is the conservative
subset plan-19 shipped. **The subset was forced by the representation, not a
scoping whim.** The fix is therefore representational first: give every IR node a
type, then a complete checker is a straightforward validator, not a second
inference engine.

Two further facts from the codebase make the shape concrete:

- **Elaboration and checking are the same code in `typecheck/`.** Inference
  functions report errors as a side effect (`infer_constructor`→arity/arg,
  `check_call`→arg match, `infer_binary`/`infer_unary`→operand, `infer_lambda`
  →captures, `check_statement`→use-after-move; ~8 intertwined points). You cannot
  "delete the checks" without first separating rejection from inference.
- **Lowering hard-depends on inference having run** — 25
  `.expect("typecheck requires …")` sites in `src/ir/lower.rs`
  (511, 573, 581, 642, 737, 755, 822, 831, 940, 990/992, 1064/1066, 1137, 1501,
  1632, 1677, 1685, 1745, 2480/2483, 2764/2806/2829, 2883; grep the message to
  re-census). Lowering also has its *own* partial inference
  (`expression_type`, `src/ir/lower.rs:1791`) — inference is *already* duplicated
  between `typecheck` and `lower`. Consolidating both into one IR elaborator is
  part of the win.

## 1. Goal

- **One typed IR.** Every `IrValue` and `IrOp` node carries (a) its result type
  — annotated on the node, or (for the reference variants `Local`/`Global`) one
  deterministic environment lookup away via the enclosing `Bind`/param/
  `IrBinding` declaration, which is a lookup, never inference — and (b) a source
  span. Serialized in the `.mfp` (format-version bump).
- **One elaborator.** A single pass turns resolved AST into fully-typed IR,
  producing a best-effort type for *every* node even for ill-typed input (an
  explicit `Unknown`/`Error` type marker where inference fails), and emits **no
  diagnostics of its own**. It absorbs both `typecheck/inference.rs` and
  `lower.rs:expression_type`.
- **One checker.** `src/ir/verify/` enforces **all** 96 semantic rule ids (§6
  census) on the typed IR, emitting the existing rule ids/messages/spans. It is
  the sole rejecter. It runs on the source-lowered IR (front-end path) and the
  decoded+merged package IR (package path).
- **`src/typecheck/` becomes elaboration-only** (or is deleted, with elaboration
  folded into lowering): after this plan, `typecheck/` contains zero
  `report_error`/diagnostic-emitting rule logic.
- **Delete the duplication** plan-19 introduced/left: fold `src/ir/verify/`'s
  subset rules into the complete checker; delete `verify_package`/`verify_ops`
  (§4 of plan-19) once the checker subsumes them.

### Non-goals (explicit constraints)

- **No language-surface change.** No new/changed syntax; the set of programs
  accepted/rejected is identical to today's.
- **No changed diagnostics for valid programs**, and **no changed diagnostic
  *identity* (rule id) for invalid programs.** Message text/span for invalid
  programs must match today's byte-for-byte **or** change only where explicitly
  reviewed and re-goldened (see the diagnostic-fidelity gate, §5).
- **No change to value/copy/move/freeze semantics, layout, or ABI.** Native
  output for every valid program stays **byte-identical** (the golden oracle).
- **No second source of truth.** The end state has exactly one implementation of
  each semantic rule. If a rule is expensive to express on the typed IR, that is
  a reason to improve the IR (add the missing type/annotation), never to fork the
  rule or skip it.
- The `.mfp` on-wire format **does** change (typed IR must serialize types +
  spans): version bump is in-scope and expected; silently breaking old packages
  is not — the decoder must reject an unknown version cleanly (it already does).

## 2. Current State

- Pipeline (`src/cli/build.rs:186–194` then lowering at 219/249/306/354):
  parse → resolve → monomorph → resolve → **typecheck** → **lower** → codegen.
  Checking runs on the AST *before* IR exists; lowering assumes well-typed AST.
- `typecheck::check_project` (`src/typecheck/mod.rs:126`) first **augments** the
  AST with the builtin source packages (json/csv/regex/datetime/vector/http/net/
  crypto/encoding, in that dependency order), then collects types/sigs and
  validates. 96 distinct rule ids across `mod.rs`/`checking.rs`/`inference.rs`
  /`resources.rs`/`types.rs`/`builtins.rs` (full census in §6). Inference
  annotates the AST (`type_name` etc.) that lowering consumes. Diagnostics
  **accumulate**: `report` (`mod.rs:2042`) prints every violation and sets
  `had_error`; checking continues, so one invalid golden holds *several*
  diagnostics, including cascades (e.g. `TYPE_UNKNOWN_VALUE` follows a failed
  `TYPE_BINARY_OPERATOR_MISMATCH` at the same site). The augmentation and the
  all-diagnostics-in-traversal-order behavior are both part of the surface the
  IR checker must reproduce.
- IR: `src/ir/value.rs`, `op.rs`, `types.rs`. `IrSourceLoc` on only 5 variants
  (`Call`/`CallResult`/`Binary`/`Unary` values, `For` op). Result type missing on
  the 8 computed-value variants above. Binary encoding `BINARY_REPR_VERSION = 2`
  (`src/ir/binary.rs:8`); `encode_value`/`decode_value`, `encode_op`/`decode_op`,
  `encode_function`.
- plan-19 delivered `src/ir/verify/` (subset: member-on-record, capture bounds,
  call/constructor arity, union-variant, empty-MATCH; conservative skip-if-
  unknown) wired in `merge_packages` (`src/target/shared/nir/lower.rs`), plus
  `tests/security/pkg-02-type-confusion`. `verify_package`/`verify_ops`
  (`src/ir/binary.rs`) still do the structural re-check. `validate_nir`
  (`src/target/shared/validate.rs`) does structural name-resolution + structural
  resource rules on the merged NIR.
- Precedent to mirror: plan-00-A..H re-typed/relocated the whole codegen seam in
  lettered sub-plans with a byte-identical self-diff oracle. This plan uses the
  same discipline: representation change first, behavior-preserving, gated by
  byte-identical goldens; the risky deletion last.

## 3. Design Overview — elaborate, then check

The architecture is **elaborate → typed IR → check**, the standard split:

```
resolve → monomorph → ELABORATE (AST → typed IR, total, no diagnostics)
                        → CHECK (typed IR → Ok | diagnostics)   ← the one checker
                        → lower (typed IR → NIR → native)
package path: decode .mfp → typed IR → (merge) → CHECK → lower
```

- **Elaboration** always yields a fully-typed IR. Where a type cannot be
  determined it stamps the explicit marker type `Unknown` (or `Error` for a
  known-bad node) and records nothing — the checker turns those into the
  diagnostics. Elaboration never panics on ill-typed input (the 25 `.expect`s
  become total code paths) and never rejects.
- **Checking** is a pure validator over a fully-typed IR: for each node it reads
  the annotated types and applies the rule. Because the IR is fully typed, the
  checker has **no legitimate skip-if-unknown path on the source route** — a bare
  `Unknown` where the checker expected a real type is either (a) an elaboration
  bug (debug-assert/panic) or (b) already flagged by the rule that owns
  `Unknown`-producing nodes. This guardrail is what prevents the plan-19 subset
  dodge from reappearing (§5).
- **Package path completeness.** Because types are serialized into the `.mfp`,
  the decoded IR is fully typed too, so the checker is **complete on the package
  path** — the conservative skips of plan-19 disappear, closing the residual
  PKG-02 gap for the type-relational rules (operand/arg/return mismatch on
  crafted package IR), not just the structural ones. This does **not** wait for
  the rule relocation: the moment types are serialized (20-B), the existing
  package checker can drop its skip-if-unknown branches (20-C) — the security
  win lands early, and 20-C is a legitimate pause point if the relocation is
  deferred.

Correctness risk concentrates in two places: the **elaboration/inference
consolidation** (must produce exactly today's types so native output stays
byte-identical) and the **diagnostic fidelity** of the relocated rules (must
reproduce today's rule ids/spans/order; message text per §8's decided
Option B). Both are gated by the existing
golden suite (byte-identical native goldens; the 371 `*-invalid` diagnostic
goldens).

**Open architectural decision (see §8):** keep `typecheck/` as an
elaboration-only module (Option 1, recommended) vs. delete `typecheck/` entirely
and fold elaboration into `lower.rs` (Option 2). Both yield one *checker*; they
differ in whether inference stays a named AST pass or becomes part of lowering.

## 4. Detailed Design

### 4.1 Typed IR (representation)

- Add `type_: String` (the type-name string form already used elsewhere) to the
  8 computed-value variants lacking it: `Call`, `CallResult`, `Binary`, `Unary`,
  `MemberAccess`, `ResultIsOk`, `ResultValue`, `ResultError` (`src/ir/value.rs`).
- `Local`/`Global` stay unannotated **by design**: their types come from the
  enclosing `Bind` op / `IrFunction` param / `IrBinding` global — all already
  typed and serialized — via a one-step environment lookup in the checker (a
  lookup, not inference, so it does not violate "pure validator"). Annotating
  each reference would create a second source of truth the checker would then
  have to cross-validate. A `Local`/`Global` naming an unbound symbol (possible
  only in crafted package IR) is a checker rejection, not a skip.
- Add `loc: IrSourceLoc` to every `IrValue`/`IrOp` variant that lacks one
  (`src/ir/value.rs`, `op.rs`) — spans on every node so relocated diagnostics can
  point at the same source location the AST checker did today.
- **Declaration/shape-level rules need a survives-lowering audit.** Rules in the
  §6 F/G groups judge declaration facts, not expression types. Some already
  survive (`IrParam.default`, `IrType.includes/variants/members` —
  `src/ir/types.rs`); others may not reach the IR today: out-of-range literals
  (folded or rejected during lowering — elaboration must instead stamp an
  `Error`-typed `Const` carrying the raw literal), binding-shape rules
  (`TYPE_LET_REQUIRES_VALUE`, `TYPE_BINDING_REQUIRES_TYPE_OR_VALUE` — the
  ill-shaped binding must still lower to a checkable `Bind`), and the
  import-level `PACKAGE_INVALID` checks (the IR has no import declarations —
  either add them or check them in the resolver, whose structural checks are
  out of this plan's scope; decide during 20-H, under the §1 "improve the IR,
  never fork the rule" principle). Each porting sub-plan's first task is this
  audit for its family.
- Extend `encode_value`/`decode_value`, `encode_op`/`decode_op`
  (`src/ir/binary.rs`) for the new fields; bump `BINARY_REPR_VERSION` 2 → 3;
  keep the decoder's clean unknown-version rejection.
- JSON projection (`src/ir/json.rs`, `to_json`) gains the fields (the `-ir` dump).

### 4.2 Elaborator (AST → typed IR)

- One pass that produces a fully-typed IR. Seeded by merging
  `lower.rs:expression_type` + `typecheck/inference.rs` inference into a single
  type-computing engine (`types.rs:parse_type`/`compatible` become its type
  algebra). Every produced node is stamped with a type; failures stamp `Unknown`.
- Remove the 25 `.expect("typecheck requires …")` panics — each becomes a total
  branch that stamps `Unknown` and proceeds.
- **No diagnostics.** Elaboration is rejection-free; all rejection moves to §4.3.

### 4.3 The one checker (`src/ir/verify/`)

- Replaces plan-19's subset with the complete rule set (§6 census). Input: a
  fully-typed `IrProject` + a `TypeEnv` (types + signatures — built exactly as
  `check_project` builds them today, **including** the builtin-source
  augmentation, and incl. imported package types on the package path).
- **Accumulates diagnostics — it does not stop at the first.** Today's checker
  prints every violation in traversal order and fails at the end
  (`report`/`had_error`); invalid goldens embed several diagnostics per file,
  including cascades where an `Unknown`-typed node triggers a follow-on rule
  (`TYPE_UNKNOWN_VALUE` after a failed operand). The IR checker must reproduce
  the same multiset *and order*, so its traversal order over functions/ops must
  match `typecheck`'s — this is a G4 requirement, not a nicety. (plan-19's
  `verify_semantics` returns first-error `Result<(), String>`; that contract is
  replaced. First-error remains acceptable on the package path only, where no
  diagnostic goldens exist.)
- Runs on: (source path) the freshly-elaborated IR in `build.rs`, replacing the
  `typecheck::check_project` call site; (package path) the merged IR in
  `merge_packages`, replacing plan-19's `verify_semantics` and `verify_package`.
- Emits the existing rule ids (`TYPE_*`/`NATIVE_*`/`EXIT_*`/`PACKAGE_*`/
  `SYMBOL_*`/`SUB_*`/`CONTINUE_*`/`UNREACHABLE_*`) via the same diagnostics
  machinery `typecheck` uses (so `mfb spec diagnostics error-codes` needs no new
  codes — verify the rule-id set is a subset of the registry).

## Layout / ABI Impact

- **`.mfp` on-wire format changes** (typed IR): `BINARY_REPR_VERSION` 2→3;
  `mfb spec package binary-representation` / `08_ir-section.md` /
  `12_verifier-rules.md` updated; all `.mfp`, `-br` hex, and `.ir` goldens
  regenerate deterministically. Old (v2) packages are rejected with the existing
  unsupported-version error (acceptable: pre-release format).
- **Native output unchanged.** Types/spans are checker/metadata only; codegen
  consumes the same information it does today. Native `.nir/.nplan/.nobj/.ncode/
  .mir` goldens and every executable stay **byte-identical** — this is the oracle
  for the elaboration consolidation.

## 5. Anti-hand-wave guardrails (how "done" is proven, not asserted)

These are the objective gates. Each names the specific dodge it blocks (the ones
this exact task already fell into once).

- **G1 — Census coverage, no subset.** §6 lists every rule with its id. "Done"
  requires a machine-checkable map: a test (`ir::verify` coverage test) asserts
  that for every `*-invalid` golden, **every** emitted rule id (goldens hold
  several) is produced by `ir::verify` and that `typecheck` produced **no**
  diagnostic. Blocks: shipping a subset and calling it done.
- **G2 — `typecheck/` has zero rejection logic at the end.** Grep gate: the
  checking rules' error-emission sites in `src/typecheck/{mod,checking,inference,
  resources}.rs` are removed; a CI-style check asserts `typecheck` emits no
  diagnostic (e.g. the diagnostic-emit helper is no longer called from the
  relocated rules). Blocks: leaving the duplicate checker in place.
- **G3 — No skip-if-unknown on the source path.** On elaborated (source) IR the
  checker must not silently pass a node whose type is a bare `Unknown` where a
  real type is required; in debug builds that is a `debug_assert!`/panic
  (elaboration bug), not a skip. Blocks: re-introducing plan-19's conservative
  skips to fake completeness.
- **G4 — All 371 `*-invalid` goldens green via the IR checker.** Hard gate:
  every rule id + span, in today's order, including cascaded diagnostics — no
  diagnostic dropped, added, or re-ordered. Soft (per §8, decided Option B):
  message *text* may change, per-rule, with the diff reviewed and recorded in
  the landing commit. Blocks: mass-regenerating invalid goldens to paper over
  id/span/order drift, and quietly dropping the second-and-later diagnostics.
- **G5 — Byte-identical native output** for every valid program
  (`scripts/test-accept.sh`; the native `.nir/.ncode/…` and executable goldens).
  Blocks: an elaboration consolidation that silently changes codegen.
- **G6 — Package path is complete, not conservative.** A `pkg-02`-style fixture
  battery covers each type-relational class (operand mismatch, arg/param
  mismatch, return mismatch, non-exhaustive MATCH, use-after-move) in *crafted
  decoded IR* and asserts rejection — proving the package checker no longer skips
  on "unknown". Blocks: declaring the security goal met while the package path
  still only catches the structural subset. **Staged (see plan-20-C):** each
  class's fixture lands with the rule family that owns it — **member confusion**
  is complete at 20-C (fixture `pkg-02b-computed-confusion`, using the serialized
  types to catch a confused *computed* target the subset skipped); operand /
  arg-param / return land in 20-E, MATCH-exhaustiveness in 20-I, use-after-move
  in 20-F, because each needs the front end's exact compatibility/linearity
  algebra relocated (approximating it early would risk false rejections on the
  shared source path). G6 is fully met when the last family lands; 20-C banks
  the member-confusion win (the actual PKG-02 audit finding).

"I hand-waved again" is defined precisely as: any of G1–G6 not literally passing.

## 6. Rule census (the porting ledger — port each, delete from `typecheck`)

Every rule below moves from `src/typecheck/` to `src/ir/verify/`, keeping its
rule id. Grouped into the sub-plan (§7) that ports it. Each rule is done only
when its `*-invalid` golden(s) pass via the IR checker AND the rule is deleted
from `typecheck`. (Citations are `typecheck` line numbers as of the 2026-07-03
census; expect small drift — re-grep the rule id, don't trust the line. The
authoritative census command is
`grep -rhoE '"[A-Z][A-Z0-9]*(_[A-Z0-9]+)+"' src/typecheck/ | sort -u`
→ **96 distinct rule ids** today; re-run it at 20-Z and reconcile against this
list before declaring G1.)

Resource linearity & annotation (→ sub-plan F): TYPE_USE_AFTER_MOVE
(checking.rs:78); TYPE_RESOURCE_BORROW_INVALIDATE (125); TYPE_RESOURCE_ELEMENT_
NOT_OWNER (336/resources.rs:180); TYPE_RESOURCE_REQUIRES_RES (156/resources.rs:
154); TYPE_RES_REQUIRES_RESOURCE (166); TYPE_STATE_INVALID (197);
TYPE_UNION_STATE_FORBIDDEN (182); TYPE_COLLECTION_OWNERSHIP_VIOLATION
(mod.rs:1855); collection RES-mark (resources.rs:154).

Typing — operators/calls/constructors/members/with/match (→ sub-plan E):
TYPE_BINARY_OPERATOR_MISMATCH (inference.rs:1230…); TYPE_UNARY_OPERATOR_MISMATCH
(1333); TYPE_UNARY_OPERATOR_UNKNOWN (1362); TYPE_REQUIRES_COMPARABLE
(1254/types.rs:291); TYPE_CALL_ARITY_MISMATCH (1456); TYPE_CALL_ARGUMENT_MISMATCH
(1399/1481/mod.rs:1629/builtins.rs); TYPE_UNKNOWN_ARGUMENT_NAME /
TYPE_DUPLICATE_ARGUMENT_NAME (builtins.rs); TYPE_SUB_HAS_NO_VALUE (256);
SYMBOL_NOT_CALLABLE (214/1430); TYPE_CONSTRUCTOR_ARITY_MISMATCH (1119);
TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH (1201); TYPE_CONSTRUCTOR_REQUIRES_RECORD
(772); TYPE_READ_ONLY_RECORD_CONSTRUCTOR (699); TYPE_RESULT_IS_IMPLICIT (724);
TYPE_DUPLICATE_FIELD (873); TYPE_UNKNOWN_FIELD (881/1090/1006/1021);
TYPE_FIELD_ACCESS_REQUIRES_RECORD (1033); TYPE_UNKNOWN_ENUM_MEMBER (963);
TYPE_READ_ONLY_RECORD_UPDATE (819/846; WITH-on-non-record reuses
TYPE_FIELD_ACCESS_REQUIRES_RECORD at 861); TYPE_MATCH_PATTERN_MISMATCH
(372/423/445); TYPE_MATCH_NOT_EXHAUSTIVE (552); TYPE_RESULT_NOT_MATCHABLE (397);
TYPE_LAMBDA_CAPTURE_UNSUPPORTED (1579…); TYPE_ASSIGNMENT_MISMATCH (1660/checking.
rs:626/664); TYPE_ASSIGN_REQUIRES_MUT (609/649).

Control flow / returns / traps / conditions (→ sub-plan I):
TYPE_RETURN_MISMATCH (checking.rs:418); SUB_RETURN_FORBIDDEN (375); EXIT_SUB_IN_
FUNC (454); EXIT_FUNC_FORBIDDEN (464); TYPE_EXIT_PROGRAM_REQUIRES_INTEGER (474);
EXIT_PROGRAM_CODE_OUT_OF_RANGE (495); EXIT_NO_MATCHING_LOOP (441);
CONTINUE_NO_MATCHING_LOOP (509); TYPE_TRAP_FALLTHROUGH (mod.rs:1801/1809);
TYPE_INLINE_TRAP_FALLS_THROUGH (inference.rs:169); TYPE_INLINE_
TRAP_REQUIRES_FALLIBLE (inference.rs:115); TYPE_INLINE_TRAP_ON_INLINED_BUILTIN
(131); TYPE_PROPAGATE_REQUIRES_TRAP (535); TYPE_RECOVER_OUTSIDE_INLINE_TRAP
(546); TYPE_RECOVER_TYPE_MISMATCH (569); TYPE_FAIL_REQUIRES_ERROR (524);
UNREACHABLE_AFTER_EXIT (20); TYPE_CONDITION_REQUIRES_BOOLEAN (749/1016/1049);
TYPE_FOR_REQUIRES_NUMERIC (910); TYPE_FOR_STEP_ZERO (925); TYPE_FOR_EACH_REQUIRES_
COLLECTION (972); FUNC/SUB shape: TYPE_FUNC_REQUIRES_RETURN_TYPE (mod.rs:1640),
TYPE_SUB_CANNOT_RETURN_VALUE (1661), TYPE_FUNC_MISSING_RETURN (1826).

Declarations / unions / collections / literals (→ sub-plan G):
TYPE_BINDING_REQUIRES_TYPE_OR_VALUE (251); TYPE_LET_REQUIRES_VALUE (258);
TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE (266); TYPE_BINDING_MISMATCH (239);
TYPE_UNKNOWN_VALUE (221); TYPE_PARAM_REQUIRES_TYPE (mod.rs:1701);
TYPE_DEFAULT_ARG_ORDER (1721); TYPE_DEFAULT_VALUE_MISMATCH (1747);
TYPE_RESOURCE_FIELD_FORBIDDEN (1521); TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION
(1535); TYPE_UNION_INCLUDE_REQUIRES_UNION (1552); TYPE_UNION_MEMBER_REQUIRES_TYPE
(1570); TYPE_DUPLICATE_VARIANT (1094); TYPE_MIXED_RESOURCE_UNION (1593);
TYPE_ENUM_REQUIRES_MEMBER (1606); TYPE_LIST_ELEMENT_MISMATCH (581/609);
TYPE_MAP_KEY_MISMATCH (649); TYPE_MAP_VALUE_MISMATCH (671); map-key comparable
(754/mod.rs:1869); TYPE_MEMBER_NOT_VISIBLE (mod.rs:1938…); TYPE_RESULT_NOT_USER_
VISIBLE (1889/1927); TYPE_THREAD_RESULT_REMOVED (inference.rs:983); literal
overflow/underflow (Integer inference.rs:40; Byte/Float/Fixed mod.rs:2060…).

Threads / native-link / packages (→ sub-plan H): TYPE_THREAD_NOT_SENDABLE
(mod.rs:1901…/resources.rs:421…); NATIVE_CPTR_ESCAPE (mod.rs:344/357); NATIVE_ABI_
RESULT_MARKER (389/469); NATIVE_ABI_UNBOUND_SLOT (419/432); NATIVE_ABI_UNBOUND_
PARAM (489); NATIVE_CONST_OUT (403); NATIVE_CONST_UNKNOWN_SLOT (502);
NATIVE_ABI_NO_RESULT (456); NATIVE_FREE_INVALID (537); PACKAGE_INVALID (602/648/
825/897); ISOLATED-requires-export-FUNC (mod.rs:1621, emitted under the reused
id TYPE_CALL_ARGUMENT_MISMATCH — port the check, no new id).

(96 distinct rule ids — the literal overflow/underflow line covers 7 of them
(TYPE_{INTEGER,BYTE,FLOAT,FIXED}_LITERAL_{OVERFLOW,UNDERFLOW}). The list is the
checklist; a rule not ticked with a passing invalid golden via `ir::verify` is
not done.)

## 7. Phases (split into small/medium lettered sub-plans before execution)

Per `.ai/planning.md`, this huge plan must be split into complete lettered
sub-plan documents; **execution task 0 is to expand each of the following into
`planning/plan-20-<L>-<slug>.md`** from the template. Ordering puts the
behavior-preserving representation work first and the risky deletion last, each
gated by byte-identical goldens.

- **20-A — Spans on every IR node (medium).** Add `loc` to all `IrValue`/`IrOp`
  variants; lowering populates from AST spans; extend encode/decode; bump format
  v3; regenerate `.mfp`/`-br`/`.ir` goldens. Native goldens byte-identical.
  Depends on: none. Acceptance: G5 + goldens regenerated deterministically.
- **20-B — Result types on every IR node (medium).** Add `type_` to the 8
  computed-value variants; lowering stamps them; encode/decode; goldens. Native
  byte-identical. Depends on: 20-A. Acceptance: `-ir` dump shows a type on every
  node; G5.
- **20-C — Complete the package-path checker (small/medium; the early security
  win).** With types serialized (20-B), the decoded IR is fully typed: remove
  plan-19's conservative skip-if-unknown branches in `src/ir/verify/` and extend
  it to the type-relational classes on decoded+merged package IR (operand
  mismatch, arg/param mismatch, return mismatch, non-exhaustive MATCH,
  use-after-move). Add the G6 fixture battery (`tests/security/pkg-02b-*`)
  **here**, not at 20-Z. Package-path messages need no golden fidelity (no
  diagnostic goldens exist there); the later census port refines these checks
  into the exact relocated rules. This closes the residual PKG-02 gap and is
  the bankable pause point: if the relocation (20-E..I, 20-Z) is deferred,
  the security goal has still landed. Depends on: 20-B. Acceptance: G6 passes;
  native byte-identical (G5); source-path behavior unchanged.
- **20-D — Total, diagnostic-free elaboration (medium/large).** Merge
  `lower.rs:expression_type` + `typecheck/inference.rs` inference into one
  elaborator producing fully-typed IR; remove the 25 `.expect` panics (stamp
  `Unknown`); elaboration emits no diagnostics. `typecheck` still runs (checks
  only) for now. Depends on: 20-B. Acceptance: elaborating every `*-invalid`
  fixture does not panic; native byte-identical (G5).
- **20-E..I — Port the rules onto `ir::verify`, one family per sub-plan
  (medium each), running the IR checker *in addition to* `typecheck` and deleting
  each ported rule from `typecheck` as its invalid goldens go green via the IR
  checker.** E=typing/operators/calls/constructors/members/match;
  F=resource linearity; G=declarations/unions/collections/literals;
  H=threads/native-link/packages; I=control-flow/returns/traps/conditions
  (letters match the §6 group tags). Depends on: 20-D.
  Acceptance per sub-plan: G1 for that family (its invalid goldens emit from
  `ir::verify`, `typecheck` silent for them) + G4 + G5.
- **20-Z — Cutover & delete (medium/large, highest risk, last).** Move the
  `ir::verify` call in `build.rs` to replace `typecheck::check_project`; reduce
  `typecheck/` to elaboration-only (or delete, folding elaboration into `lower`
  per §8); delete `verify_package`/`verify_ops` and plan-19's subset code (now
  subsumed); extend the G6 battery to any class the census port added; delete
  plan-19 doc. Depends on: 20-E..I all landed. Acceptance: G1–G6 all pass; full
  acceptance green; native byte-identical.

## Validation Plan

- Function tests: any rule whose diagnostic text changes gets its
  `tests/…_invalid/**` golden reviewed and re-synced (expected to be common —
  §8 pre-commits to Option B); new package-path fixtures under
  `tests/security/pkg-02b-*` (G6), one per type-relational class, landing at
  **20-C** (the early security win), extended at 20-Z if the census port adds
  classes.
- Runtime proof: not a runtime feature — the proof is byte-identical native
  output (G5) + the diagnostic goldens (G1/G4). No behavior change to observe at
  runtime is itself the requirement.
- Doc sync: `mfb spec package {binary-representation,verifier-rules,ir-section}`
  (format v3, checker now complete); `mfb spec architecture native-ir` (typed
  IR); `mfb spec diagnostics error-codes` verified unchanged (no new codes).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  after every sub-plan; the full suite is the gate for each.

## 8. Open Decisions

- **Elaboration home — Option 1 (recommended): keep `src/typecheck/` as an
  elaboration-only module** (inference stays a named AST pass, rejection removed)
  vs. **Option 2: delete `src/typecheck/` and fold elaboration into
  `src/ir/lower.rs`** (one AST-consuming pass total). Option 1 is lower-risk and
  incremental (typecheck keeps running as a check during 20-E..I, deleted only at
  20-Z); Option 2 is the cleaner end state but couples the cutover to a larger
  refactor. Recommend Option 1; revisit at 20-Z. (§3, §4.2)
- **Diagnostic fidelity — DECIDED: Option B (reviewed re-golden), taken
  liberally.** The fidelity contract is **rule id + span**; message text is not
  worth stalling on — the invalid goldens are our own suite, not a user-facing
  compatibility surface, and chasing byte-identical text from IR context (a
  different traversal medium) is the highest-effort/lowest-value part of the
  relocation. Take Option A (byte-identical) only where it falls out naturally
  from spans-on-every-node (20-A); otherwise re-golden per rule with the diff
  reviewed and recorded in the landing commit. G4 is read accordingly: rule
  id + span + diagnostic order are the hard gate; message-text diffs are
  expected and reviewed, not fought. (§5 G4)
- **Format bump vs. re-elaborate-on-decode.** Serialize types into `.mfp`
  (Option 1, recommended — makes the package checker complete, G6) vs. keep the
  lean IR and re-elaborate types on decode (avoids the format bump but re-runs
  inference on the package path). Recommend serialize; the format is pre-release.

## Non-Goals

- No new language features, no new diagnostics, no new error codes.
- No change to what programs compile — only *where* the check lives.
- Not a performance plan (though consolidating two inference passes into one is a
  likely incidental win; not a goal).

## Summary

The real engineering risk is twofold and both halves are gated by existing
oracles: (1) **elaboration consolidation** must reproduce today's types exactly —
proven by byte-identical native goldens (G5); (2) **rule relocation** must
reproduce today's diagnostics and cover all 96 rule ids with none left in
`typecheck` — proven by the invalid-golden coverage map (G1/G4/G2). The
representation change (typed IR, format v3) is mechanical but wide (golden
regen). The deletion of `typecheck`'s checking and plan-19's subset is last,
behind all the porting. What stays untouched: the language surface, value/move
semantics, and native code — nothing a user runs changes; only the compiler's
internal single source of truth for "is this program well-formed" does.
