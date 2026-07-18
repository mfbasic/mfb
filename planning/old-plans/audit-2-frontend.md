# Audit 2 — Surface 2: Language front-end (lexer / parser / resolver / syntaxcheck / monomorph / ir)

Last updated: 2026-07-14
Untrusted party: author of an arbitrary `.mfb` source file the victim compiles.
Must not: crash the compiler with unbounded parse/resolve/monomorph recursion
(stack-overflow SIGABRT), or drive codegen into an unchecked state.

Scope read: `src/lexer.rs`, `src/escape.rs`, `src/numeric.rs`, `src/ast/**`,
`src/resolver/**`, `src/syntaxcheck/**`, `src/monomorph/**`, `src/ir/**`.
Reproductions run against `target/debug/mfb` (macOS-aarch64).

## Verdict on prior audit-1 findings (re-verified)

| ID | Prior sev | Verdict | Evidence |
|----|-----------|---------|----------|
| FE-01 | HIGH | **FIXED** | `MAX_EXPR_DEPTH = 256` with `enter_expr`/`leave_expr` in `parse_expression`/`parse_not`/`parse_power`/`parse_unary`/grouped-paren (`src/ast/expr.rs:8`). Repro: 5000 nested parens → graceful `MFB_PARSE_UNEXPECTED_TOKEN` "Expression nesting is too deep." |
| FE-02 | HIGH | **STILL PRESENT** | monomorph polymorphic recursion → SIGABRT. Reproduced. → **bug-182**. |
| FE-03 | HIGH | **STILL PRESENT** | statement-block recursion → SIGABRT before `ir::verify` runs. Reproduced at N≥2000. → **bug-183**. |
| FE-04 | MEDIUM | **FIXED** | every count-driven alloc wrapped in `bounded_capacity` (`src/binary_repr/util.rs:81`). |
| FE-05 | LOW | **FIXED** | `src/ir/verify/mod.rs:1661-1669` emits `TYPE_FLOAT_LITERAL_OVERFLOW` when `!f.is_finite()` (negated mirror `:1743`). Repro: `LET x = 1e400` → graceful overflow diagnostic. |

No new independent vulnerabilities in code added since audit-1. The plan-41
scalar/backtick lexer (`src/lexer.rs:413` `lex_scalar`, `:560` `\u{}` decoder) is
fully iterative, consumes exactly one scalar, caps `\u{}` at 6 hex digits
(`:587`), with an error-recovery path — no recursion/unbounded loop.
`src/numeric.rs` bounds scientific-notation expansion at `MAX_EXPANDED_DIGITS =
8192` and rejects extreme exponents in O(1).

## Findings (still-open, re-verified & reproduced)

### FE-02 — HIGH — Monomorph polymorphic recursion → unbounded instantiation → SIGABRT
- Location: `src/monomorph/lower.rs:475` `instantiate_function` (dedup `:528`),
  `:639` `instantiate_type` (dedup `:644`).
- Threat/impact: a compiled source aborts the compiler (`fatal runtime error:
  stack overflow, aborting`) with no diagnostic — DoS against anyone building it.
- Mechanism: the dedup guard (`emitted_function_keys.insert("{name}<{args}>")`)
  only suppresses re-emitting an already-seen instantiation; under polymorphic
  recursion each self-call has a distinct concrete type argument, so every key is
  new, the guard never fires, and body lowering re-enters `instantiate_function`
  on fresh native frames with no depth/count cap. Monomorph runs *before*
  `ir::verify`, so verify's `MAX_DEPTH` backstop never executes.
- Reproduction (observed = crash):
  ```
  FUNC recurse OF T(x AS T) AS Integer
    LET y AS List OF T = [x]
    RETURN recurse(y)
  END FUNC
  FUNC main() AS Integer
    RETURN recurse(1)
  END FUNC
  ```
  `mfb build` → stack overflow abort. Expected: a bounded `TYPE_*` diagnostic.
- Best fix: instantiation-depth counter / total-instantiation budget on the
  monomorph context (cap 256, matching `MAX_EXPR_DEPTH`); report `TYPE_*` and
  return `None` past the cap, at both `instantiate_function` and `instantiate_type`.
- Non-goals: precise productive/unproductive recursion detection; language-surface
  change; growing the stack. **bug-182.**

### FE-03 — HIGH — Statement-block recursion has no parser depth limit → SIGABRT
- Location: `src/ast/stmt.rs:710` `parse_statement_block` ↔ `:4` `parse_statement`
  ↔ `:407` `parse_if_statement` (and via `parse_match_statement:484`,
  `parse_for_statement`, `parse_while_statement`, `parse_do_statement`). The only
  parser depth guard, `expr_depth`/`MAX_EXPR_DEPTH` (`src/ast/parser.rs:26`),
  covers expressions only.
- Threat/impact: DoS — compiler stack-overflow abort, no diagnostic.
- Mechanism: each nested block adds a `parse_statement_block → parse_statement →
  parse_if_statement → parse_statement_block …` native-frame chain with no
  counter; the deep AST is then re-walked recursively (also uncapped) by resolver,
  syntaxcheck, monomorph, `ir::lower`. `ir::verify` caps statement nesting at
  `MAX_DEPTH = 256` (`src/ir/verify/mod.rs:385,713-719`) but runs last, after the
  passes that overflow.
- Reproduction (observed): N nested `IF 1 = 1 THEN … END IF`:
  | N | Result |
  |---|--------|
  | 300–500 | graceful parse error (incidental parser limit) |
  | 2000 / 5000 / 20000 | `fatal runtime error: stack overflow, aborting` |
  Threshold is stack/build-mode dependent but trivially reachable.
- Best fix: statement-nesting counter on `FileParser` mirroring `expr_depth`
  (cap 256); report `MFB_PARSE_*` and bail. Capping at parse time protects every
  downstream pass at once.
- Non-goals: growing the stack; weakening `ir::verify`'s backstop; language-surface
  change. **bug-183.**

## Verdict

Two still-open HIGH DoS findings (FE-02, FE-03), both reproduced against a built
binary → bug-182 / bug-183. Theme: `ir::verify` has the right caps but sits last
in the pipeline; the fixes belong upstream (monomorph for FE-02, parser for
FE-03). FE-01/04/05 confirmed fixed.
