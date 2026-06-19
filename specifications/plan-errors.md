# Plan: Inline `TRAP` error handling, de-overload `MATCH`

Status: proposed (planning only — no spec or compiler changes yet)
Owner: Justin
Date: 2026-06-18

## 1. Motivation

The current error model (`mfbasic.md` §8) is sound in its core — every function
returns `Result`, success auto-unwraps, errors auto-propagate to a single
function-level `TRAP`. The wart is **local** error handling: to intercept an
error at the call site you make the call the direct scrutinee of a `MATCH`
(§8.4). That overloads `MATCH` with two unrelated jobs:

1. its original job — branching over an enum/union/`Result` *value*; and
2. error handling — forcing an `Ok`/`Error` pair where the `Ok` case is pure
   ceremony that just binds the value and continues.

The result reads as if "branch on a union" and "handle a failure" are the same
intent, and every local handler carries a boilerplate `CASE Ok(v)` arm.

## 2. The change (agreed)

Introduce a dedicated **inline, postfix, diverging** `TRAP` for single-expression
error handling, and remove error handling from `MATCH`.

```basic
LET f = fs::openFile(path) TRAP(e)
  io::print(e.message)
  RETURN 0            ' MUST diverge — see §4
END TRAP
```

Reading rule: a call auto-propagates **unless** a postfix `TRAP` is attached;
the `TRAP` is the local override of the default. The happy value auto-unwraps
into the binding (`f`); the error block runs only on failure.

### Decisions locked

- **Postfix**, not prefix (`expr TRAP(e)`, not `TRAP(e) FOR expr`). Keeps the
  call as the subject and preserves left-to-right reading.
- **Diverging-only.** The error block must leave via `RETURN`, `FAIL`,
  `PROPAGATE`, or loop control (`EXIT`/`CONTINUE` where applicable). It may
  **not** fall through `END TRAP`, because there would be no value to bind.
  This mirrors the existing rule-7 discipline ("every path ends in RETURN/FAIL").
- **`MATCH` keeps only union/value matching.** The special "call-as-direct-
  scrutinee suppresses auto-unwrap" rule (§8.4) is removed. A `MATCH` whose
  scrutinee is a call now auto-unwraps like every other call site.

### Decisions to confirm

- **D1 — Keep the function-level `TRAP`.** Recommendation: **keep it.** It is
  not redundant with inline `TRAP`; it is the catch-all for the entry point
  (§8.6 rule 10), the single place to wrap every error from a body with context,
  and the observation point as resources drop. Sell the model as **one keyword,
  two scopes**: `expr TRAP(e) … END TRAP` traps one expression; a bottom-of-
  function `TRAP(e) … END TRAP` traps the whole body.
- **D2 — Unify the trap spelling to `TRAP(e)`** at both scopes (today the
  function trap is `TRAP err`, no parens). Recommendation: **unify**, so the two
  scopes are visibly the same construct. This is a clean break (pre-1.0); no
  back-compat alias. Touches every existing trap test + spec snippet.
- **D3 — Value-fallback (`expr ELSE value`)** is explicitly **out of scope** for
  this change. If "on error, substitute a value and continue" is wanted later,
  add a separate operator rather than letting the `TRAP` body fall through.
  Recorded here so the diverging rule isn't quietly relaxed later.

## 3. Grammar / syntax

```
PostfixTrap   := Expression "TRAP" "(" Identifier ")" Newline
                   StatementList
                 "END" "TRAP"
```

- Legal only as the value/RHS of a `LET`/`MUT` binding, an `Assign`, or a
  bare expression statement (e.g. `doThing() TRAP(e) … END TRAP`).
- The binding identifier (`e`) is an `Error`, scoped to the trap block only.
- No type annotation on the binding (always `Error`), matching the function trap.
- One expression per inline trap — there is intentionally no way to wrap several
  fallible calls in one inline trap. That is what the function-level trap is for.

## 4. Semantics

- **Happy path:** the trapped expression auto-unwraps; its `Ok` value is bound /
  assigned / used exactly as a normal call today.
- **Error path:** control enters the trap block with `e : Error`. Live resource
  bindings in the enclosing scope that are already established are unaffected;
  resources created *by the trapped expression itself* are dropped before the
  block runs (same lexical-drop rules as §8.1/§14.7/§15).
- **Divergence required:** every path through the inline trap block must end in
  `RETURN`, `FAIL`, `PROPAGATE`, or valid loop control. Fall-through to
  `END TRAP` is a **compile error** (new diagnostic, see §6).
- **`PROPAGATE` inside an inline trap** re-raises `e`: routes to the enclosing
  function-level `TRAP` if present, else returns the error to the caller. (Same
  meaning as in a function trap; lift the "PROPAGATE only valid in a trap"
  check to accept inline traps too.)
- **Nesting:** the trapped expression may itself contain calls; their errors are
  caught by *this* inline trap (it is the nearest enclosing trap for that
  expression). An inline trap block body may contain further statements with
  their own inline traps. Document that this is single-expression scoping, not
  arbitrary block nesting (the thing the user wanted to avoid with TRY/CATCH).

## 5. Spec edits (`specifications/mfbasic.md`)

- **§ summary / line 5 & 15:** restate the model — "errors auto-route to an
  inline `TRAP` on the failing expression, a function-level `TRAP`, or
  propagate." Keep "no TRY, no GOTO, no exceptions."
- **§8.1:** unchanged core; add a sentence that a call auto-propagates unless a
  postfix `TRAP` is attached.
- **§8.3:** rename to cover both scopes; document inline `TRAP` and the
  diverging rule. Keep the trap-outcomes table (RETURN/PROPAGATE/FAIL) — it
  applies to both scopes.
- **§8.4:** rewrite. Remove "make the call the direct scrutinee of a `MATCH`."
  Replace the local-handling example and the `getUser`/`ErrNotFound` absence
  example with inline `TRAP`:
  ```basic
  LET user = getUser(id) TRAP(e)
    IF e.code = errorCode::ErrNotFound THEN RETURN defaultUser
    FAIL e
  END TRAP
  ```
  State plainly: `MATCH` no longer intercepts call errors; it matches enum/
  union/`Result` **values** only.
- **§8.6 rules:** add inline-trap rules — diverging requirement, one-expression
  scope, `PROPAGATE` now valid in inline traps, binding scoped to the block.
  If D2 accepted, restate trap spelling as `TRAP(e)` throughout.
- **§4.4 / §7 (`Result`, `Nothing`):** clarify that `MATCH` over a genuinely
  `Result`-typed **value** (e.g. `t.result` from a completed `Thread`, §6.x)
  is still ordinary union matching with `CASE Ok(v)` / `CASE Error(e)`. Only the
  call-as-scrutinee shortcut is removed. Update the §7 `Nothing` `MATCH` example
  (lines ~512) accordingly (it matches a call today — change to inline `TRAP`
  or to a held value).
- **§3.x lint note (line 38):** add inline `TRAP` to the list of constructs the
  dense-line linter should flag.

## 6. Compiler changes

Files and current anchors (from a read of the tree):

- **`src/lexer.rs`** — `Keyword::Trap` already exists (line 81); no new keyword.
  If D2 accepted, no lexer change either (still `TRAP` + `(`).
- **`src/ast.rs`**
  - Add an inline-trap node. Preferred shape: an `Expression::Trapped {
    expression: Box<Expression>, binding: String, handler: Vec<Statement>,
    line }` so it composes anywhere an expression value is expected (LET/MUT/
    Assign/Expression statement). Alternative: a dedicated statement form — but
    the expression form reuses existing binding parsing.
  - Reuse the existing `Trap` struct (line 128) for the function-level trap.
  - Parser (parse functions begin ~line 750; function-trap parse ~line 899):
    after parsing a primary/postfix expression in binding/assign/expr-statement
    position, peek for `Keyword::Trap` immediately followed by `LParen`; if
    present, parse `( Identifier )`, newline, statement list, `END TRAP`.
    Guard: only attach in the three legal positions (§3); error elsewhere.
- **`src/typecheck.rs`**
  - `infer_match_scrutinee` (line 2068): **remove the call-suppression branch**
    that returns `Type::Result(...)` for a call scrutinee (lines ~2075–2118).
    A call scrutinee should infer to its unwrapped success type like any call.
    Keep returning `Type::Result(..)` only when the scrutinee is a value already
    of `Result` type (field access, local of `Result` type, etc.).
  - Ok/Error exhaustiveness (lines ~2226, 2276, 2306, 2512): keep — still valid
    for `Result`-typed *values*. Just no longer reachable via a call scrutinee.
  - New: type-check `Expression::Trapped` — infer the inner expression's
    `Result OF T`, bind the handler's `e : Error`, check the inner success type
    `T` flows to the binding, and **verify the handler diverges** on every path
    (reuse the body-terminator analysis used for function traps at lines
    ~1138–1171). New diagnostics:
    - `TYPE_INLINE_TRAP_MUST_DIVERGE` — handler falls through `END TRAP`.
    - `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` — trapped expression cannot fail
      (no `Result`), so the trap is dead (warn or error — recommend error for
      consistency with how unreachable handlers are treated).
  - `Propagate` check (lines ~1483): widen "valid only inside a TRAP" to also
    accept inline-trap handler bodies.
- **`src/ir.rs`**
  - `IrOp::Trap` (line 123) and lowering (line 445) cover the function trap.
    Lower `Expression::Trapped` to: evaluate inner → on `Ok` bind value and
    continue → on `Err` bind `e`, run handler ops (which must terminate). Reuse
    the `Propagate → Fail` rewrite (line 545) for `PROPAGATE` in inline handlers.
  - Decide IR representation: either a new `IrOp::InlineTrap { … }` or desugar to
    the existing trap/branch primitives. Desugaring keeps backends unchanged;
    prefer it if the existing branch + fail ops are expressive enough.
- **`src/target/shared/code/mod.rs`** and per-target backends
  (`src/target/macos_aarch64/code.rs`, etc.) — only touched if a new `IrOp` is
  added. If inline trap desugars to existing ops, no backend change.
- **`src/bytecode.rs`** — extend if AST/IR serialization formats change (golden
  `.ast`/`.ir` files are regenerated by the test harness).

## 7. Tests

Harness: each `tests/<name>/` has `project.json`, `src/*.mfb`, and `golden/`
(`*.ast`, `*.ir`, `build.log`, plus `*.out` for runtime). Regenerate goldens
with `scripts/test-accept.sh` after the implementation lands.

### New tests
- `control-flow-inline-trap-valid` — happy unwrap into binding; error path with
  `RETURN`, with `FAIL`, with `PROPAGATE`; inline trap in `LET`, `MUT`,
  `Assign`, and bare-expression positions; the `ErrNotFound` absence idiom.
- `control-flow-inline-trap-invalid` — handler falls through (`MUST_DIVERGE`);
  inline trap on an infallible expression (`REQUIRES_FALLIBLE`); inline trap in
  an illegal position; `PROPAGATE` semantics where no enclosing function trap.
- `control-flow-inline-trap-resource-rt` — resource created by the trapped
  expression is dropped before the handler runs (assert via runtime `.out`).
- `control-flow-inline-trap-nested-valid` — call inside the trapped expression
  routes to the inline trap, not the function trap; handler containing its own
  inline trap.

### Migrate existing MATCH-on-call tests (8 dirs use `CASE Ok(`/`CASE Error(`)
Audit each — keep `MATCH` where it matches a real `Result` **value**; convert to
inline `TRAP` where it matched a call:
- `control-flow-match-destructuring`
- `control-flow-match-exhaustiveness-invalid`
- `func_thread_result_valid` — matches `t.result` (a value): **keep** `MATCH`.
- `func_thread_send_valid` — audit.
- `func_typesystem_result_pattern_invalid`
- `func_typesystem_result_pattern_valid` — pattern over a `Result` value: likely
  **keep** `MATCH`.
- `thread-queue-timeout-cancel`
- `user-function-default-args-result-valid`

### Function-trap tests (if D2 — spelling unification)
Update `TRAP err` → `TRAP(err)` across `control-flow-trap-valid`,
`control-flow-trap-invalid`, `control-flow-sub-trap-valid/-invalid`,
`audit-trap-recovery`, `project-entry-*-trap`, and any others. Mechanical;
regenerate goldens.

## 8. Rollout / sequencing

1. Land the spec edits (§5) first so the design is the source of truth.
2. AST node + parser (§6) with parse-only tests (`.ast` goldens).
3. Typecheck: remove MATCH call-suppression; add inline-trap checks + diagnostics.
4. IR lowering (desugar preferred) + backend (only if new `IrOp`).
5. Migrate the 8 MATCH tests; add the new inline-trap tests; regenerate goldens.
6. If D2: mechanical `TRAP err` → `TRAP(err)` sweep across spec + tests.

## 9. Open questions

- D1 (keep function-level trap) — recommend keep. Confirm.
- D2 (unify spelling to `TRAP(e)`) — recommend unify. Confirm (drives test churn).
- D3 (no value-fallback now) — confirm out of scope.
- Should an inline trap be allowed on a bare expression statement whose value is
  discarded (e.g. `doThing() TRAP(e) … END TRAP` with no `LET`)? Recommend yes —
  it's the `SUB`/effectful-call case and is the cleanest replacement for a
  one-off `MATCH` on a fallible effect.
- Diagnostic policy for a `TRAP` on an infallible expression: hard error vs
  warning. Recommend hard error to match existing unreachable-handler treatment.
