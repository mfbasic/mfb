# Plan: `Result` becomes a runtime-only type (never user-visible)

Status: proposed (planning only — no spec or compiler changes yet)
Owner: Justin
Date: 2026-06-18
Companion to: `plan-errors.md` (inline `TRAP`, de-overload `MATCH`)

## 1. Goal

`Result` (and its success member `Ok`) should be a **runtime/implementation
concept only**. A user writing MFBASIC never names it, constructs it, matches on
it, or holds a value of it. The mental model the language teaches is:

> A function call either produces its value (auto-unwrapped) or fails with an
> `Error` (auto-propagated, or caught by a `TRAP`).

`Error` stays **fully public** — users construct `Error[...]`, read `e.code` /
`e.message`, and bind it in `TRAP(e)`. Only `Result`/`Ok` go dark.

This finishes what `plan-errors.md` starts. That plan removes the two places a
user *handles* a `Result` (`MATCH` on a call scrutinee). This plan removes the
last place a user *holds* one (`Thread.result`) and removes `Result`/`Ok` from
the user-facing surface and documentation entirely.

### Non-goal

The compiler keeps its internal `Type::Result(..)` representation
(`src/typecheck.rs:32`) and the `Ok`/`Err` IR forms. "Internal" means **not
expressible or observable in user syntax** — not "deleted from the compiler."
Every function still has effective type `Result OF T` *internally*; the user
just never writes or sees it.

## 2. Current exposure surface (what makes `Result` visible today)

### A. Namable as a type
- Reserved type name in the resolver: `src/resolver.rs:22` (`"Result"` in the
  built-in type list).
- `parse_type` produces `Type::Result`: `src/typecheck.rs:4190`
  (`"Result" => Type::Result(...)`).
- Parser treats `Result` as a template head with `OF`: `src/ast.rs:2117`.
- Spec line 50 lists `Result` among user-facing "compiler-owned templates"
  alongside `List`, `Map`, `Thread`.
- Spec §4.4 (lines 296–317) documents `Result OF T`, `Ok`, and `Error` as the
  error model's types — teaches `Result` as a user concept.
- Already half-closed: §typecheck rejects `Result` as a *return* type
  (`src/typecheck.rs:1039`, "Functions declare their success type; Result
  wrapping is implicit"). This plan extends that to **all** type positions.

### B. Constructible / matchable
- `Ok`/`Err` recognized as pattern/constructor identifiers:
  `src/resolver.rs:673,732`, `src/monomorph.rs:1084`,
  `src/typecheck.rs:2512` (`"Ok" | "Result"`).
- `MATCH` exhaustiveness over `Ok`/`Error`: `src/typecheck.rs:2226, 2276, 2306`.
- (The call-as-scrutinee `MATCH` path is removed by `plan-errors.md`.)

### C. Holdable as a value
- `Thread.result` field, typed `Result OF Out`: spec lines 373, 1104, 1129,
  1131; compiler at `src/typecheck.rs:2736`, `src/ir.rs:1205`,
  `src/builtins/thread.rs:237`. This is the **only** value-level exposure.

### D. Mentioned in documentation type-lists
- Sendability (line 1100), defaultability (line 399), lexical cleanup (1131),
  bytecode invariants (1587–1593), §7 `Nothing`/`Result OF Nothing`
  (lines 464, 497, 509), built-ins return `Result` (1272). These describe the
  *runtime* concept and must be reworded so they don't present `Result` as a
  user type.

## 3. The change

1. **Remove `Thread.result`.** Keep `thread::waitFor(t) AS Out`, which already
   auto-unwraps/auto-propagates the worker outcome like any call (spec line
   1104, 1111). Poll with `thread::isRunning(t)` (line 1110); `waitFor` after
   completion returns immediately. Inspect-and-continue is handled by the inline
   `TRAP` handler binding `e` plus loop control (see `plan-errors.md` §4):
   ```basic
   FOR EACH t IN workers
     LET v = thread::waitFor(t) TRAP(e)
       errors.add(e)        ' the Error value, when you want it
       CONTINUE
     END TRAP
     results.add(v)
   END FOR
   ```
2. **Make `Result`/`Ok` un-nameable.** Any `Result` or `Ok` in a user type
   position (param, field, `LET`/`MUT` annotation, type alias, template arg,
   return type) is a compile error directing the user to the success type.
3. **Make `Ok`/`Error` un-matchable as `Result` members.** With (1) and the
   `plan-errors.md` MATCH change, no user `MATCH` can ever scrutinize a
   `Result`. `CASE Ok(..)` / `CASE Error(..)` in user code becomes a diagnostic.
4. **Scrub the docs.** `Result`/`Ok` disappear from user-facing prose; the error
   model is taught purely as value-or-`Error`. A single implementer-facing note
   records that the runtime represents fallible outcomes internally as
   `Result`/`Ok`.

## 4. Spec edits (`specifications/mfbasic.md`)

### Remove the value exposure (Thread)
- **Lines 373, 1104, 1129, 1131:** delete the `t.result` field and all prose
  about it ("exposes `result AS Result OF Out`", "Field access `t.result`
  waits…"). Keep `thread::waitFor` as the sole retrieval path; keep "retrieves
  the outcome exactly once and consumes/closes the handle." Reword the
  arena/lifetime sentences (1103) to say "before `thread::waitFor(t)` exposes
  the value" without naming `Result`.

### Remove `Result` from user vocabulary
- **Line 50:** drop `Result` from the user-facing template list; it is not a
  template users name. (Keep `List`, `Map`, `Thread`.)
- **§4.4 (296–317):** retitle to "`Error` and absence" and rewrite around the
  user model: functions may fail; failure carries the public `Error` type;
  success auto-unwraps; there is no `Option`/`Maybe` (absence = `Error` with a
  semantic code such as `errorCode::ErrNotFound`). Remove the `Result OF T` /
  `Ok` member exposition. Move the "internally a two-member union" detail into
  the implementer note (below).
- **Lines 464, 497, 509, 665:** reword "every function returns `Result`" /
  "effective type `Result OF T`" / "`Result OF Nothing`" to "every function may
  fail; `FUNC F(...) AS T` yields `T` on success and an `Error` on failure" and
  "a `SUB` yields nothing on success." Replace the §7 `Result OF Nothing`
  `MATCH` example (509–515) with an inline-`TRAP` example (cross-ref
  `plan-errors.md` §5).
- **Line 1272:** "fallible built-ins return `Result`" → "fallible built-ins can
  fail and auto-propagate like any call."
- **Lines 399, 1100, 1131, 1587–1593:** where `Result` appears in
  sendability/defaultability/cleanup/bytecode-invariant lists, reword to the
  user-neutral concept ("a worker outcome", "fallible outcomes") or move to the
  implementer note. These describe runtime behavior, not user types.

### Add one implementer-facing note
- A short subsection (e.g. end of §4.4 or in the bytecode/runtime section)
  stating: *the runtime represents every fallible outcome internally as a
  two-member `Result`/`Ok`+`Error` union; this type is not nameable,
  constructible, or matchable in user code and exists only in compiler IR and
  bytecode metadata.* This keeps the invariant honest for implementers without
  re-exposing it to users.

## 5. Compiler edits

- **`src/resolver.rs`**
  - Remove `"Result"` from the reserved built-in type list (line 22) so it is no
    longer a recognized user type name; instead, intercept it (and `Ok`) in type
    position to emit a targeted diagnostic rather than "unknown type."
  - `Ok`/`Err` pattern recognition (lines 673, 732): remove the user-facing
    acceptance; emit a diagnostic if seen in a user `CASE`.
- **`src/typecheck.rs`**
  - `parse_type` (line 4190): stop mapping `"Result"` to `Type::Result` for
    *user* type strings; route to a diagnostic. (Keep an internal-only
    constructor for compiler-synthesized effective types.)
  - Extend the existing return-type rejection (line 1039–1042) to **all** type
    positions: params, fields, `LET`/`MUT` annotations, type aliases, template
    args. New diagnostic `TYPE_RESULT_NOT_USER_VISIBLE` — "`Result` is internal;
    declare the success type `T` instead."
  - `"Ok" | "Result"` handling (line 2512) and Ok/Error exhaustiveness (2226,
    2276, 2306): remove the user-`MATCH` paths; a `CASE Ok`/`CASE Error` in user
    code is now `TYPE_RESULT_NOT_MATCHABLE`.
  - `t.result` field access (line 2736): remove; accessing `.result` on a
    `Thread` is now an unknown field (or a targeted "use `thread::waitFor`"
    diagnostic).
  - Keep `Type::Result` (line 32) as the internal effective-type representation
    — untouched.
- **`src/ir.rs`**
  - `member == "result"` thread-field lowering (line 1205): remove; retrieval is
    only via `thread::waitFor`. Internal `Ok`/`Err` IR forms stay.
- **`src/monomorph.rs`**
  - `Ok` handling (line 1084): keep for compiler-synthesized nodes; ensure it is
    unreachable from user syntax once the front-end diagnostics above are in.
- **`src/builtins/thread.rs`**
  - Remove the `t.result` output-type plumbing (line 237) and the
    `"List" | "Result"` field branch (line 327) as it pertains to user access.
    Keep `thread::waitFor`'s `AS Out` signature.

## 6. Tests

Harness: `tests/<name>/` with `project.json`, `src/*.mfb`, `golden/`; regenerate
with `scripts/test-accept.sh`.

### New invalid tests
- `result-not-user-visible-invalid` — `Result`/`Ok` in param, field, `LET`
  annotation, type alias, template arg, and return position; each expects
  `TYPE_RESULT_NOT_USER_VISIBLE`.
- `result-not-matchable-invalid` — `CASE Ok(..)` / `CASE Error(..)` in a user
  `MATCH`; expects `TYPE_RESULT_NOT_MATCHABLE`.
- `thread-result-field-removed-invalid` — `t.result` field access; expects the
  unknown-field / use-`waitFor` diagnostic.

### Migrations (the last `CASE Ok/Error` users)
After `plan-errors.md` migrates call-scrutinee MATCHes, these thread/value cases
also convert here:
- `func_thread_result_valid`, `func_thread_send_valid`,
  `thread-queue-timeout-cancel` — `t.result` + `MATCH Ok/Error` →
  `thread::waitFor` + inline `TRAP` (or the loop/`CONTINUE` collect pattern).
- `func_typesystem_result_pattern_valid` / `_invalid` — these exist specifically
  to exercise user `Result` matching; repurpose them into the new invalid tests
  above (they should now *reject* the syntax) or delete if redundant.
- Re-audit any remaining `CASE Ok(`/`CASE Error(` in `tests/*/src/*.mfb` — the
  target is **zero** user-facing occurrences.

## 6a. Companion cleanup: `SUB` becomes value-less (no `RETURN NOTHING`)

Once `Result`/`Ok` are internal, the matching `Nothing`-success plumbing on
`SUB` is the last bit of "unit value" leaking into the surface. A `SUB` cannot
be *truly* non-returning — it still has an error channel (it can `FAIL`,
auto-propagate, and drop resources on the way out) — but it can be **value-less**:
it produces no success value, and its call is a statement, not an expression.

### The change
- A `SUB` produces **no success value.** Internally it remains `Result OF Nothing`
  (invisible, like `Result` itself).
- A `SUB` **call is a statement**, not an expression. `LET x = aSub()` is a
  compile error (today it silently binds `x = NOTHING`).
- **Remove `RETURN NOTHING`.** Bare `RETURN` remains a value-less early exit
  *at this step only*; `plan-exit.md` then bans `RETURN` in a `SUB` entirely and
  makes `EXIT SUB` the early exit. Fall-through to `END SUB` is success. (Keeping
  bare `RETURN` here keeps this step self-contained — `EXIT SUB` does not exist
  until plan-exit lands last.)
- Failure is handled with inline `TRAP` (`plan-errors.md`), never `MATCH Ok/Error`.
- `Nothing` **stays a nameable unit type** — it is still needed for marker union
  members (e.g. `value AS Nothing`) and the `FUNC(...) AS Nothing` callback bridge
  that lets a `SUB` be passed to `forEach`. We are killing the `SUB`-return smell,
  not the unit type. (Fully eliminating `Nothing` — a first-class effect-only
  function kind, a `SUB`-typed `forEach`, respelled marker members — is a deeper
  type-system change and is **out of scope**.)

### Spec edits (`mfbasic.md` §7, §4.6)
- **§7 line 497:** drop `RETURN NOTHING` and `Ok(NOTHING)`; state a `SUB`
  produces no value and fall-through succeeds. (`RETURN`'s final disposition —
  banned, replaced by `EXIT SUB` — is written by `plan-exit.md`.)
- **§7 lines 509–515:** the `Result OF Nothing` `MATCH` example is already being
  replaced by an inline-`TRAP` example (§4 above) — the `fs::writeAll` failure
  case demonstrates the value-less + `TRAP` pattern.
- **§7 line 489 / §4.6 line 343–347:** keep `Nothing` as the unit type for type
  positions; remove the `' Ok(NOTHING)` comment (line 347) and the framing of a
  `SUB` as "success type `Nothing`" in favor of "effect-only, value-less."

### Compiler edits
- **Parser/typecheck:** reject `RETURN NOTHING` inside a `SUB`
  (`SUB_RETURN_TAKES_NO_VALUE`); a bare `RETURN` and fall-through stay valid at
  this step (plan-exit later rejects bare `RETURN` too, via `SUB_RETURN_FORBIDDEN`).
- **Typecheck:** a `SUB` call used in value position (`LET`/`MUT`/`Assign` RHS,
  argument, operand) is a compile error `TYPE_SUB_HAS_NO_VALUE`. A `SUB` call as
  a bare statement, and as a `FUNC(...) AS Nothing` callback value, stays valid.
- Internal `Result OF Nothing` / `Ok(NOTHING)` IR is unchanged — invisible.

### Tests
- New `sub-value-less-invalid`: `LET x = aSub()` → `TYPE_SUB_HAS_NO_VALUE`;
  `RETURN NOTHING` in a `SUB` → `SUB_RETURN_TAKES_NO_VALUE`.
- Audit existing `SUB` tests for `RETURN NOTHING` and any `LET x = <subcall>`;
  migrate to bare `RETURN` / statement calls.

## 7. Sequencing

1. Land `plan-errors.md` first (inline `TRAP`, remove MATCH call-scrutinee). It
   establishes the replacement for every user-facing `Result` interaction.
2. Then this plan: remove `Thread.result`, lock `Result`/`Ok` out of user
   syntax, scrub docs, add the implementer note.
3. Final check: `grep -rE "\bResult\b|\bCASE Ok\b" tests/*/src/*.mfb` and the
   user-facing sections of `mfbasic.md` return nothing. The only surviving
   `Result` mentions are the single implementer note and compiler-internal code.

## 8. Open questions

- **Diagnostic vs silent unknown** for `Result`/`Ok`/`t.result`: recommend
  *targeted* diagnostics (not generic "unknown type/field") so the message can
  point users at the success type or `thread::waitFor`. Confirm.
- **`func_typesystem_result_pattern_*`**: repurpose as rejection tests vs delete.
  Recommend repurpose — they document the now-illegal forms.
- **Keep `Result` reserved (un-usable as an identifier) or fully free it?**
  Recommend keep it reserved so a user can't define a `TYPE Result`, avoiding
  confusion with the internal type. Confirm.
- **Implementer note placement** — §4.4 footnote vs bytecode/runtime section.
  Recommend the bytecode/runtime invariants section, where `Result` legitimately
  remains a typed-metadata concept.
- **Value-less `SUB` (§6a)** — confirm making `SUB` calls statement-only
  (`LET x = aSub()` becomes illegal) and removing `RETURN NOTHING`. Recommend
  yes; keep `Nothing` as a nameable unit type for marker fields and the
  `FUNC(...) AS Nothing` callback bridge (full `Nothing` removal is out of scope).
