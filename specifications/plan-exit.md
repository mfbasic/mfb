# Plan: the `EXIT` / `CONTINUE` family — loop control, sub, and process early-exit

Status: proposed (planning only — no spec or compiler changes yet)
Owner: Justin
Date: 2026-06-18
Companion to: `plan-errors.md`, `plan-result-cleanup.md`

**Implementation order across the plan set:** `plan-errors.md` →
`plan-result-cleanup.md` → `plan-exit.md` (this one last). The earlier plans
reference `EXIT`/`CONTINUE` as ways a diverging inline-`TRAP` handler can leave;
those references become real once this plan lands, so no separate reconciliation
pass is needed — implementing in this order resolves them.

## 1. Motivation

MFBASIC currently has **no loop-control or early-exit primitive at all** — no
loop break, no skip-to-next-iteration, no value-less routine exit, no process
termination. The only ways out of a block today are running to its end, `RETURN`
(value), `FAIL`/`PROPAGATE` (error), or an auto-propagated error. This forces
guard clauses into nested `IF`s and gives loops no way to stop early or skip an
iteration.

Two earlier plans create the need:
- `plan-result-cleanup.md` §6a makes `SUB` value-less and bans `RETURN` inside a
  `SUB`, removing the only early-exit a `SUB` had.
- `plan-errors.md` makes the inline `TRAP` handler *recover-or-diverge*. The
  "handle the error and skip this loop iteration" shape needs a diverging
  statement that means "next iteration" — i.e. `CONTINUE`. (`RECOVER` covers
  "substitute a value and continue past the trap"; `CONTINUE` covers "abandon
  the rest of this iteration.")

This plan adds one consistent BASIC-idiomatic family — `EXIT` (break / leave) and
`CONTINUE` (skip to next iteration) — with the loop kind named explicitly so
control flow stays readable and there is no `GOTO`-style ambiguity.

## 2. The forms

| Form | Meaning | Legal where |
|------|---------|-------------|
| `EXIT FOR` | break out of the enclosing `FOR` / `FOR EACH` loop | inside a `FOR`/`FOR EACH` |
| `EXIT DO` | break out of the enclosing `DO … LOOP` | inside a `DO` loop |
| `EXIT WHILE` | break out of the enclosing `WHILE … WEND` | inside a `WHILE` loop |
| `CONTINUE FOR` | skip to the next iteration of the enclosing `FOR` / `FOR EACH` | inside a `FOR`/`FOR EACH` |
| `CONTINUE DO` | skip to the next iteration of the enclosing `DO … LOOP` | inside a `DO` loop |
| `CONTINUE WHILE` | skip to the next iteration of the enclosing `WHILE … WEND` | inside a `WHILE` loop |
| `EXIT SUB` | value-less early success exit from the enclosing `SUB` | inside a `SUB` |
| `EXIT FUNC` | **always a compile error** — functions must `RETURN` a value | anywhere (diagnostic only) |
| `EXIT PROGRAM <integer>` | terminate the program with the given exit code, running full RAII cleanup | anywhere |

Loop-kind coverage note: `FOR` and `FOR EACH` both end in `NEXT`, so
`EXIT FOR` / `CONTINUE FOR` serve both. `DO … LOOP UNTIL` and `DO WHILE … LOOP`
are both `EXIT DO` / `CONTINUE DO`. `WHILE … WEND` is `EXIT WHILE` /
`CONTINUE WHILE`.

## 3. Semantics

### Loop exits (`EXIT FOR` / `EXIT DO` / `EXIT WHILE`)
- Transfer control to the statement immediately after the matched loop.
- **Target = the innermost enclosing loop whose kind matches the keyword
  (Q1 resolved — multi-level break allowed).** Naming the kind makes the target
  explicit, and `EXIT`/`CONTINUE` may break *through* inner loops of other kinds
  to reach it. Example: inside a `FOR` nested in a `DO`, `EXIT DO` leaves the
  `FOR` to break the enclosing `DO`. Compile error if no enclosing loop of that
  kind exists.
- Compile error `EXIT_NO_MATCHING_LOOP` if there is no enclosing loop of that
  kind.
- Bindings, resources, and `Thread` handles declared **inside** the exited
  loop's body (and any inner blocks unwound) are dropped in reverse declaration
  order on the way out (§14.7 / §15 / §16 rules — `EXIT` becomes a new drop
  edge alongside scope exit / `RETURN` / `FAIL` / `PROPAGATE`).

### Loop skips (`CONTINUE FOR` / `CONTINUE DO` / `CONTINUE WHILE`)
- End the current iteration and proceed to the matched loop's next-iteration
  point: a `FOR`/`FOR EACH` advances to its step/next element and re-tests; a
  `DO WHILE … LOOP` / `WHILE … WEND` jumps to the top condition test; a
  `DO … LOOP UNTIL` jumps to the bottom `UNTIL` test.
- **Target = the innermost enclosing loop whose kind matches the keyword**, same
  rule and tie-break as `EXIT` (Q1 applies identically).
- Compile error `CONTINUE_NO_MATCHING_LOOP` if there is no enclosing loop of that
  kind.
- Bindings/resources/threads declared in the current iteration's body scope (and
  inner blocks unwound) are dropped before the next iteration begins — the same
  drop edge as a normal end-of-iteration.
- It is the loop counterpart to `RECOVER` in `plan-errors.md`: `RECOVER`
  substitutes a value and continues *past the trap*; `CONTINUE` abandons the rest
  of the iteration. See §3 example below.

### `EXIT SUB`
- Value-less early exit from the enclosing `SUB`; semantically identical to
  fall-through to `END SUB` (success, no value).
- Legal only inside a `SUB`. Inside a `FUNC` it is `EXIT_SUB_IN_FUNC` — "use
  `RETURN <value>`."
- Drops all live bindings/resources/threads in the sub on the way out.
- If the entry point is a `SUB`, `EXIT SUB` from it terminates with exit code `0`
  (same as `SUB` success, §8.7).
- **Bans `RETURN` in a `SUB` (finalizes plan-result-cleanup §6a).** With `EXIT
  SUB` providing the value-less early exit, any `RETURN` / `RETURN NOTHING` inside
  a `SUB` is now a compile error `SUB_RETURN_FORBIDDEN` ("a `SUB` returns no
  value — use `EXIT SUB`"). `RETURN` is `FUNC`-only.
- **`EXIT SUB` is the success terminator for a `SUB`'s function-level `TRAP`.**
  A trap may not fall through (§8.6 rule 6) and `RETURN` is banned in a `SUB`, so
  a `SUB` trap that swallows an error and succeeds ends in `EXIT SUB` (its
  `FAIL`/`PROPAGATE` paths are unchanged). `EXIT SUB` is therefore legal inside a
  function-level trap as well as the body. (A `FUNC` trap still succeeds with
  `RETURN <value>`.)

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
All `EXIT` forms (except the always-error `EXIT FUNC`) and all `CONTINUE` forms
are **diverging statements**: control does not continue to the next statement in
the same block.
- Code textually after an `EXIT`/`CONTINUE` in the same block is unreachable —
  `UNREACHABLE_AFTER_EXIT` (consistent with the existing post-`RETURN` rule).
- `EXIT SUB`, `EXIT PROGRAM`, and `CONTINUE FOR/DO/WHILE` all count as valid
  **terminators** for the path-termination analysis in `plan-errors.md` §4: a
  diverging inline-`TRAP` handler inside a loop may end in `CONTINUE <kind>`
  ("handle the error, skip this iteration"); `EXIT SUB`/`EXIT PROGRAM` terminate
  anywhere; a loop-body `EXIT FOR/DO/WHILE` satisfies divergence within that loop.

### Example: skip failed iterations
The `CONTINUE` + inline-`TRAP` combination expresses "log the failure and move
on" — where `RECOVER` (substitute a value) would be wrong because the rest of the
iteration should not run:

```basic
MUT processed AS Integer = 0
FOR EACH path IN paths
  LET f = fs::openFile(path) TRAP(e)
    io::print("skipping " & path & ": " & e.message)
    CONTINUE FOR              ' abandon this iteration entirely
  END TRAP
  process(f)                  ' only reached when the open succeeded
  processed = processed + 1   ' not counted for skipped files
END FOR
```

## 4. Grammar (§19 EBNF additions)

```
ExitStmt     := "EXIT" LoopKind
              | "EXIT" "SUB"
              | "EXIT" "FUNC"          ' parsed, then rejected
              | "EXIT" "PROGRAM" Expression
ContinueStmt := "CONTINUE" LoopKind
LoopKind     := "FOR" | "DO" | "WHILE"
```

## 5. Spec edits (`mfbasic.md`)

- **§10 Control Flow:** add the loop-exit **and loop-skip** forms with the
  matching rule and short examples; note `FOR EACH` uses `EXIT FOR` /
  `CONTINUE FOR`. State there is still no `GOTO`; `EXIT`/`CONTINUE` are
  structured, lexically-scoped, single-target.
- **§7 Subs:** add `EXIT SUB` as the value-less early-exit; state `RETURN` /
  `RETURN NOTHING` are now compile errors in a `SUB` (finalizing
  `plan-result-cleanup.md` §6a, which made the `SUB` value-less). State `EXIT
  SUB` is the success terminator for a `SUB`'s function-level `TRAP`.
- **§8.3 / §8.6 (error model):** the trap-outcomes table's "succeed" terminator
  is `RETURN <value>` for a `FUNC` and `EXIT SUB` for a `SUB`; `FAIL`/`PROPAGATE`
  unchanged. (Reconciles with `plan-errors.md` §8.3, which marks `RETURN`-succeeds
  as `FUNC`-only.)
- **§6 Functions:** note `EXIT FUNC` is forbidden — functions `RETURN` a value.
- **New subsection (§10.x or §8.7-adjacent) `EXIT PROGRAM`:** define termination,
  exit-code rules, non-catchability, and the full-RAII unwind policy.
- **§8.7 entry-point table (lines 656–658):** add a row — `EXIT PROGRAM <n>`
  terminates with code `n`, short-circuiting the normal return-to-exit-code
  mapping (after the stack-wide RAII unwind).
- **§14.7 / §15 / §16:** add `EXIT FOR/DO/WHILE/SUB` and `CONTINUE FOR/DO/WHILE`
  to the list of drop edges (`CONTINUE` drops the current iteration's body scope),
  and add `EXIT PROGRAM` as a stack-wide drop edge that unwinds every live scope
  up to the entry point. No exception to the RAII close guarantee is introduced.

## 6. Compiler edits

- **`src/lexer.rs`:** add `Keyword::Exit`, `Keyword::Continue`, and
  `Keyword::Program` (`FOR`/`DO`/`WHILE`/`SUB`/`FUNC` already exist — lines
  47–81). `PROGRAM` is only meaningful after `EXIT`; simplest is a plain reserved
  keyword.
- **`src/ast.rs`:** add `Statement::Exit { target: ExitTarget, code:
  Option<Expression>, line }` with `enum ExitTarget { For, Do, While, Sub, Func,
  Program }`, and `Statement::Continue { kind: LoopKind, line }` with
  `enum LoopKind { For, Do, While }`. Parse after the statement dispatch; `code`
  is `Some` only for `Program`.
- **`src/typecheck.rs`:**
  - `EXIT FOR/DO/WHILE` and `CONTINUE FOR/DO/WHILE`: walk the enclosing-loop
    stack; error `EXIT_NO_MATCHING_LOOP` / `CONTINUE_NO_MATCHING_LOOP` if no loop
    of that kind encloses the statement.
  - `EXIT SUB`: error `EXIT_SUB_IN_FUNC` if the enclosing routine is a `FUNC`.
    Legal in a `SUB` body and in a `SUB`'s function-level trap.
  - `RETURN` inside a `SUB` (bare or `RETURN NOTHING`): error
    `SUB_RETURN_FORBIDDEN` ("use `EXIT SUB`"). Supersedes
    `plan-result-cleanup.md`'s interim `SUB_RETURN_TAKES_NO_VALUE`.
  - `EXIT FUNC`: always `EXIT_FUNC_FORBIDDEN`.
  - `EXIT PROGRAM`: require the operand to be `Integer`; constant-fold and
    host-range check; emit `EXIT_PROGRAM_CODE_OUT_OF_RANGE` for an out-of-range
    constant.
  - Reachability: flag `UNREACHABLE_AFTER_EXIT`; register `EXIT SUB`/`EXIT
    PROGRAM`/`CONTINUE FOR/DO/WHILE` as terminators in the path-termination pass
    (shared with `plan-errors.md`).
- **`src/ir.rs`:**
  - New `IrOp`s for loop-break (jump to the loop's exit label) and loop-continue
    (jump to the loop's next-iteration label) — the lowering loop-context stack
    maps each loop kind to *both* labels.
  - `EXIT SUB` lowers to the sub's success-exit path (run scope drops, produce
    the internal `Ok(Nothing)`).
  - New `IrOp::ExitProgram { code }` — a stack-wide unwind intrinsic. Unlike a
    plain process-exit, lowering must ensure live-scope drops run for the current
    frame and propagate the terminate-and-drop signal through callers
    (uncatchable by `TRAP`) up to the entry point, then exit with the code.
  - Insert lexical-drop ops on **all** `EXIT FOR/DO/WHILE/SUB/PROGRAM` and
    `CONTINUE FOR/DO/WHILE` paths (`CONTINUE` drops only the current iteration's
    body scope); `EXIT PROGRAM` additionally drops every enclosing/caller scope.
- **`src/target/shared/code/mod.rs` + per-target backends:** loop-break → jump to
  loop-end label, loop-continue → jump to the next-iteration label; `EXIT SUB`
  reuses sub-return lowering; `EXIT PROGRAM` → stack-wide RAII unwind (run drops
  for each live frame) then call the runtime/OS exit with the code. The unwind
  reuses the existing drop/cleanup emission used for error propagation, minus the
  `TRAP` routing.

## 7. Tests

Harness: `tests/<name>/` with `project.json`, `src/*.mfb`, `golden/`; regenerate
with `scripts/test-accept.sh`. Runtime exit code / `.out` checks for
`EXIT PROGRAM` and loop behavior.

- `exit-loop-valid` — `EXIT FOR` from `FOR` and `FOR EACH`; `EXIT DO` from both
  `DO` forms; `EXIT WHILE`; nested loops where the named kind selects the right
  target; a resource declared in the loop body is dropped on `EXIT`.
- `exit-loop-invalid` — `EXIT FOR` with no enclosing `FOR`; code after `EXIT`
  (`UNREACHABLE_AFTER_EXIT`).
- `continue-loop-valid-rt` — `CONTINUE FOR`/`DO`/`WHILE` skip the rest of the
  iteration (observable via `.out`); `CONTINUE FOR` as the diverging tail of an
  inline `TRAP` handler (the skip-failed-iterations example); body-scope resource
  dropped before the next iteration.
- `continue-loop-invalid` — `CONTINUE FOR` with no enclosing `FOR`
  (`CONTINUE_NO_MATCHING_LOOP`); code after `CONTINUE` (`UNREACHABLE_AFTER_EXIT`).
- `exit-sub-valid` — `EXIT SUB` guard clause; `EXIT SUB` from a `SUB` entry point
  → exit 0; `EXIT SUB` as the success terminator of a `SUB`'s function-level
  `TRAP` (swallow-and-succeed).
- `exit-sub-invalid` — `EXIT SUB` inside a `FUNC` (`EXIT_SUB_IN_FUNC`);
  `EXIT FUNC` (`EXIT_FUNC_FORBIDDEN`); `RETURN` and `RETURN NOTHING` inside a
  `SUB` (`SUB_RETURN_FORBIDDEN`).
- `exit-program-valid-rt` — `EXIT PROGRAM 3` terminates with code 3;
  `EXIT PROGRAM` with a computed code; assert a live resource opened up the call
  stack is closed during the unwind (RAII observed before exit).
- `exit-program-invalid` — non-`Integer` operand; out-of-range constant
  (`EXIT_PROGRAM_CODE_OUT_OF_RANGE`).

## 8. Relationship to the other plans

No separate reconciliation pass is needed — implementing in the stated order
(`plan-errors.md` → `plan-result-cleanup.md` → this plan) makes every forward
reference real by the time it matters:

- `plan-errors.md` and `plan-result-cleanup.md` §6a name `EXIT`/`CONTINUE` as
  ways a diverging inline-`TRAP` handler leaves and as the `SUB` early-exit.
  Those keywords land here, last, so the references resolve on completion.
- Note (see §9, Q3): `RECOVER` (from `plan-errors.md`) can already express the
  skip-failed-iteration shape by substituting a harmless value and neutralizing
  the rest of the iteration with flags. `CONTINUE` is therefore an **ergonomic
  convenience** — clearer when several downstream statements would otherwise need
  neutralizing — not a strict necessity. It is included here for that clarity.

## 9. Decisions & open questions

Resolved:
- **Q2 — `EXIT PROGRAM` cleanup → clean ASAP, full RAII.** Stack-wide unwind runs
  every live scope's drops up to the entry point before exit; no exception to the
  RAII close guarantee. (See §3.)
- **Q4 — spelling → `EXIT PROGRAM`** (reserved word `PROGRAM`).

- **Q3 — `CONTINUE` → included here.** Added as `CONTINUE FOR/DO/WHILE`,
  mirroring the `EXIT` family. Positioned as an ergonomic convenience: `RECOVER`
  + flags already covers the functional need (substitute a harmless value, count
  via a flag), but `CONTINUE` reads cleaner when several downstream statements
  would otherwise have to be neutralized.

- **Q1 — loop-target rule → multi-level allowed.** `EXIT <kind>` / `CONTINUE
  <kind>` target the innermost enclosing loop *of that kind*, breaking through
  inner loops of other kinds to reach it (e.g. `EXIT DO` from a `FOR` nested in a
  `DO` leaves the `FOR` to break the `DO`). Compile error if no enclosing loop of
  that kind exists. (See §3.)

No open questions remain.
