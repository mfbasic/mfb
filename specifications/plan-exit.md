# Plan: the `EXIT` family — loop, sub, and process early-exit

Status: proposed (planning only — no spec or compiler changes yet)
Owner: Justin
Date: 2026-06-18
Companion to: `plan-errors.md`, `plan-result-cleanup.md`

## 1. Motivation

MFBASIC currently has **no early-exit primitive at all** — no loop break, no
value-less routine exit, no fast process termination. The only ways out of a
block today are running to its end, `RETURN` (value), `FAIL`/`PROPAGATE` (error),
or an auto-propagated error. This forces guard clauses into nested `IF`s and
gives loops no way to stop early.

`plan-result-cleanup.md` §6a makes `SUB` value-less and bans `RETURN` inside a
`SUB`. That removes the only early-exit a `SUB` had, so a replacement is
required. This plan adds one consistent BASIC-idiomatic keyword — `EXIT` —
covering loops, subs, and the process, with the kind named explicitly so control
flow stays readable and there is no `GOTO`-style ambiguity.

## 2. The forms

| Form | Meaning | Legal where |
|------|---------|-------------|
| `EXIT FOR` | break out of the enclosing `FOR` / `FOR EACH` loop | inside a `FOR`/`FOR EACH` |
| `EXIT DO` | break out of the enclosing `DO … LOOP` | inside a `DO` loop |
| `EXIT WHILE` | break out of the enclosing `WHILE … WEND` | inside a `WHILE` loop |
| `EXIT SUB` | value-less early success exit from the enclosing `SUB` | inside a `SUB` |
| `EXIT FUNC` | **always a compile error** — functions must `RETURN` a value | anywhere (diagnostic only) |
| `EXIT PROGRAM <integer>` | terminate the program with the given exit code, running full RAII cleanup | anywhere |

Loop-kind coverage note: `FOR` and `FOR EACH` both end in `NEXT`, so `EXIT FOR`
serves both. `DO … LOOP UNTIL` and `DO WHILE … LOOP` are both `EXIT DO`.
`WHILE … WEND` is `EXIT WHILE`.

## 3. Semantics

### Loop exits (`EXIT FOR` / `EXIT DO` / `EXIT WHILE`)
- Transfer control to the statement immediately after the matched loop.
- **Target = the innermost enclosing loop whose kind matches the keyword.**
  Naming the kind makes the target explicit; an `EXIT FOR` inside a `DO` inside a
  `FOR` exits that outer `FOR` and the `DO` it passed through (see open question
  Q1 for the stricter "must match innermost loop overall" alternative).
- Compile error `EXIT_NO_MATCHING_LOOP` if there is no enclosing loop of that
  kind.
- Bindings, resources, and `Thread` handles declared **inside** the exited
  loop's body (and any inner blocks unwound) are dropped in reverse declaration
  order on the way out (§14.7 / §15 / §16 rules — `EXIT` becomes a new drop
  edge alongside scope exit / `RETURN` / `FAIL` / `PROPAGATE`).

### `EXIT SUB`
- Value-less early exit from the enclosing `SUB`; semantically identical to
  fall-through to `END SUB` (success, no value).
- Legal only inside a `SUB`. Inside a `FUNC` it is `EXIT_SUB_IN_FUNC` — "use
  `RETURN <value>`."
- Drops all live bindings/resources/threads in the sub on the way out.
- If the entry point is a `SUB`, `EXIT SUB` from it terminates with exit code `0`
  (same as `SUB` success, §8.7).

### `EXIT FUNC`
- Recognized solely to emit a targeted compile error `EXIT_FUNC_FORBIDDEN`:
  "Functions must `RETURN` a value; `EXIT FUNC` is not allowed." Never executes.

### `EXIT PROGRAM <integer>`
- Terminates the program with `<integer>` as the host exit code.
- Legal in any `FUNC` or `SUB`, at any call depth. It is not an error and is
  **not catchable** by any `TRAP`.
- **Cleanup policy (resolved, Q2): clean ASAP — full RAII.** `EXIT PROGRAM`
  initiates an uncatchable unwind that runs lexical drops for every live scope in
  the current function **and every caller up to the entry point**, in the normal
  reverse-declaration drop order (§14.7), closing all live resources and dropping
  all `Thread` handles, then terminates with the code. It is "ASAP" in that it
  skips remaining *work* (no further statements run), not in that it skips
  *cleanup*. The total-RAII guarantee in §15/§16 is preserved — `EXIT PROGRAM` is
  a drop edge like any other exit, just one that unwinds the whole stack at once.
  Mechanically: a terminate signal carrying the exit code propagates up the call
  stack, running each frame's drops, but bypassing every `TRAP`.
- The operand is any `Integer` expression. The exit code follows the same
  host-range rules as an entry-point `FUNC AS Integer` return (§8.7): a constant
  operand outside the host range is a compile error; a non-constant out-of-range
  value is truncated per host convention (e.g. `code & 0xFF` on POSIX).

### Terminator / reachability
All `EXIT` forms (except the always-error `EXIT FUNC`) are **diverging
statements**: control does not continue to the next statement in the same block.
- Code textually after an `EXIT` in the same block is unreachable —
  `UNREACHABLE_AFTER_EXIT` (consistent with the existing post-`RETURN` rule).
- `EXIT SUB` and `EXIT PROGRAM` count as valid block/function **terminators** for
  the path-termination analysis in `plan-errors.md` §4 (a diverging inline-`TRAP`
  handler may end in `EXIT SUB` or `EXIT PROGRAM`; a loop-body `EXIT FOR/DO/WHILE`
  satisfies divergence within that loop).

## 4. Grammar (§19 EBNF additions)

```
ExitStmt   := "EXIT" LoopKind
            | "EXIT" "SUB"
            | "EXIT" "FUNC"          ' parsed, then rejected
            | "EXIT" "PROGRAM" Expression
LoopKind   := "FOR" | "DO" | "WHILE"
```

## 5. Spec edits (`mfbasic.md`)

- **§10 Control Flow:** add the loop-exit forms with the matching rule and a
  short example; note `FOR EACH` uses `EXIT FOR`. State there is still no
  `GOTO`; `EXIT` is structured, lexically-scoped, single-target.
- **§7 Subs:** add `EXIT SUB` as the value-less early-exit (ties into
  `plan-result-cleanup.md` §6a, which bans `RETURN` here).
- **§6 Functions:** note `EXIT FUNC` is forbidden — functions `RETURN` a value.
- **New subsection (§10.x or §8.7-adjacent) `EXIT PROGRAM`:** define termination,
  exit-code rules, non-catchability, and the full-RAII unwind policy.
- **§8.7 entry-point table (lines 656–658):** add a row — `EXIT PROGRAM <n>`
  terminates with code `n`, short-circuiting the normal return-to-exit-code
  mapping (after the stack-wide RAII unwind).
- **§14.7 / §15 / §16:** add `EXIT FOR/DO/WHILE/SUB` to the list of drop edges,
  and add `EXIT PROGRAM` as a stack-wide drop edge that unwinds every live scope
  up to the entry point. No exception to the RAII close guarantee is introduced.

## 6. Compiler edits

- **`src/lexer.rs`:** add `Keyword::Exit` and `Keyword::Program`
  (`FOR`/`DO`/`WHILE`/`SUB`/`FUNC` already exist — lines 47–81). `PROGRAM` is only
  meaningful after `EXIT`; simplest is a plain reserved keyword.
- **`src/ast.rs`:** add `Statement::Exit { target: ExitTarget, code:
  Option<Expression>, line }` with `enum ExitTarget { For, Do, While, Sub, Func,
  Program }`. Parse after the statement dispatch in the parser; `code` is `Some`
  only for `Program`.
- **`src/typecheck.rs`:**
  - `EXIT FOR/DO/WHILE`: walk the enclosing-loop stack; error
    `EXIT_NO_MATCHING_LOOP` if no loop of that kind encloses the statement.
  - `EXIT SUB`: error `EXIT_SUB_IN_FUNC` if the enclosing routine is a `FUNC`.
  - `EXIT FUNC`: always `EXIT_FUNC_FORBIDDEN`.
  - `EXIT PROGRAM`: require the operand to be `Integer`; constant-fold and
    host-range check; emit `EXIT_PROGRAM_CODE_OUT_OF_RANGE` for an out-of-range
    constant.
  - Reachability: flag `UNREACHABLE_AFTER_EXIT`; register `EXIT SUB`/`EXIT
    PROGRAM` as terminators in the path-termination pass (shared with
    `plan-errors.md`).
- **`src/ir.rs`:**
  - New `IrOp` for loop-break (jump to the loop's exit label) — needs a
    loop-context stack during lowering mapping loop kind → exit label.
  - `EXIT SUB` lowers to the sub's success-exit path (run scope drops, produce
    the internal `Ok(Nothing)`).
  - New `IrOp::ExitProgram { code }` — a stack-wide unwind intrinsic. Unlike a
    plain process-exit, lowering must ensure live-scope drops run for the current
    frame and propagate the terminate-and-drop signal through callers
    (uncatchable by `TRAP`) up to the entry point, then exit with the code.
  - Insert lexical-drop ops on **all** `EXIT FOR/DO/WHILE/SUB/PROGRAM` paths;
    `EXIT PROGRAM` additionally drops every enclosing/caller scope (full RAII).
- **`src/target/shared/code/mod.rs` + per-target backends:** loop-break → jump to
  loop-end label; `EXIT SUB` reuses sub-return lowering; `EXIT PROGRAM` →
  stack-wide RAII unwind (run drops for each live frame) then call the runtime/OS
  exit with the code. The unwind reuses the existing drop/cleanup emission used
  for error propagation, minus the `TRAP` routing.

## 7. Tests

Harness: `tests/<name>/` with `project.json`, `src/*.mfb`, `golden/`; regenerate
with `scripts/test-accept.sh`. Runtime exit code / `.out` checks for `EXIT APP`.

- `exit-loop-valid` — `EXIT FOR` from `FOR` and `FOR EACH`; `EXIT DO` from both
  `DO` forms; `EXIT WHILE`; nested loops where the named kind selects the right
  target; a resource declared in the loop body is dropped on `EXIT`.
- `exit-loop-invalid` — `EXIT FOR` with no enclosing `FOR`; code after `EXIT`
  (`UNREACHABLE_AFTER_EXIT`).
- `exit-sub-valid` — `EXIT SUB` guard clause; `EXIT SUB` from a `SUB` entry point
  → exit 0.
- `exit-sub-invalid` — `EXIT SUB` inside a `FUNC` (`EXIT_SUB_IN_FUNC`);
  `EXIT FUNC` (`EXIT_FUNC_FORBIDDEN`).
- `exit-program-valid-rt` — `EXIT PROGRAM 3` terminates with code 3;
  `EXIT PROGRAM` with a computed code; assert a live resource opened up the call
  stack is closed during the unwind (RAII observed before exit).
- `exit-program-invalid` — non-`Integer` operand; out-of-range constant
  (`EXIT_PROGRAM_CODE_OUT_OF_RANGE`).

## 8. Reconciliation with existing plans

- `plan-errors.md` §4 and `plan-result-cleanup.md` §6a reference "loop control
  (`EXIT`/`CONTINUE`)" as ways a diverging inline-`TRAP` handler can leave. After
  this plan, **`EXIT` forms are real** and those references become valid for
  `EXIT`. Update both plans to name the concrete forms.
- **`CONTINUE` is still not added by this plan** (the request was the `EXIT`
  family only). The "collect errors and continue iterating" loop pattern in
  `plan-result-cleanup.md` §3 / §6 uses `CONTINUE`, which does not exist. Either
  add `CONTINUE` (skip-to-next-iteration) in a follow-up, or rewrite that pattern
  to not depend on it. Flagged in Q3 — recommend a small follow-up adding
  `CONTINUE` for `FOR`/`DO`/`WHILE`, since the diverging-`TRAP`-in-a-loop case
  genuinely needs "handle and continue the loop."

## 9. Decisions & open questions

Resolved:
- **Q2 — `EXIT PROGRAM` cleanup → clean ASAP, full RAII.** Stack-wide unwind runs
  every live scope's drops up to the entry point before exit; no exception to the
  RAII close guarantee. (See §3.)
- **Q4 — spelling → `EXIT PROGRAM`** (reserved word `PROGRAM`).

Open:
- **Q1 — loop-target rule.** Recommended: `EXIT <kind>` targets the innermost
  enclosing loop *of that kind*, even across inner loops of other kinds
  (BASIC-traditional; explicit because the kind is named). Stricter alternative:
  require the named kind to equal the innermost enclosing loop overall (no
  multi-level break). Confirm.
- **Q3 — add `CONTINUE`?** The diverging inline-`TRAP`-inside-a-loop pattern
  needs "handle the error and continue the loop," which no `EXIT` form expresses
  (`EXIT FOR` leaves the loop entirely). Likely spelled `CONTINUE FOR/DO/WHILE`
  to mirror the `EXIT` family. Recommend a follow-up `plan-continue.md`. Confirm
  scope.
