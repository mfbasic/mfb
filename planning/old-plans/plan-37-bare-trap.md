# Bare `TRAP` (optional error binding) Plan

Last updated: 2026-07-12
Effort: small (<1h)

Allow both TRAP forms — the function-level `TRAP … END TRAP` at the bottom of a
FUNC/SUB, and the postfix inline `expr TRAP … END TRAP` — to omit the
parenthesized `(err)` error identifier when the handler never references the
error. `TRAP(e) … END TRAP` stays valid and unchanged; bare `TRAP … END TRAP`
becomes an accepted equivalent whose handler simply has no name bound for the
`Error`. Every existing trap semantic (RECOVER / RETURN / EXIT SUB / FAIL /
PROPAGATE, resource drop, one-trap-per-function, inline-only-on-binding rules)
is unchanged; PROPAGATE inside a bare handler still re-propagates the same error
because the error remains internally bound — the user just has no name for it.

The single behavioral outcome: a program using `x = toInt(s) TRAP` / `RECOVER
-1` / `END TRAP` (no `(e)`) and a FUNC/SUB whose bottom trap is written `TRAP` /
`PROPAGATE` / `END TRAP` compiles, runs, and behaves identically to the same
program written with an unused `(e)` binding.

References:

- `mfb spec language error-model` (§8) — the TRAP contract this plan extends.
- `src/docs/spec/language/08_error-model.md` — §8.3/§8.4 prose to update.
- `src/docs/spec/language/19_grammar.md:63` — the `trap` EBNF production.
- `.ai/compiler.md` — front-end change gate (validation, function tests).
- `.ai/specifications.md` — spec-sync obligation for any language change.

## 1. Goal

- Bare `TRAP` (no `(ident)`) parses and compiles in **both** positions:
  - function-level: `FUNC … <body> TRAP <handler> END FUNC`;
  - inline postfix: `LET x = call() TRAP <handler> END TRAP` (and the MUT /
    assignment / bare-expression-statement forms inline TRAP already allows).
- The handler runs on failure exactly as today. `RECOVER v`, `RETURN v`,
  `EXIT SUB`, `FAIL e`, and `PROPAGATE` all behave identically to the
  `TRAP(e)`-with-unused-`e` spelling.
- **`PROPAGATE` must still work inside a bare handler** (both positions). Even
  though the user has no name for the error, `PROPAGATE` re-propagates the same
  trapped `Error` — with its original `code`, `message`, and `source` intact —
  to the enclosing trap or the caller. This is the one handler verb that *reads*
  the caught error implicitly, so it is the sharpest test that bare TRAP keeps
  the error live internally rather than discarding it.
- `TRAP(e) … END TRAP` continues to parse and behave exactly as before
  (byte-identical output for all existing programs).

### Non-goals (explicit constraints)

- No change to trap **semantics**: at-most-one function-level trap, trap at the
  bottom after normal flow, inline TRAP legal only as the value of a LET/MUT
  binding / assignment / bare expression statement, every handler path must
  RECOVER-or-diverge, resource lexical drop on the error path — all unchanged.
- **No leak.** A bare trap must free the caught `Error` on every handler exit
  exactly as `TRAP(e)` does today. The user has no name for the error, but the
  error object still exists (a routed arena block), so it must still be dropped.
  See §4 — under the chosen design this is byte-identical to a named-but-unused
  `TRAP(e)`, so the existing bug-151 cleanup covers it with no new code.
- No change to the `Error`/`ErrorLoc` record shapes, the fallible-call ABI, or
  any `.mfp` binary representation of trap regions.
- No new keyword; `RECOVER`/`RESUME` semantics untouched (`RESUME` remains a
  non-keyword — this plan does not add it).
- Existing `TRAP(e)` programs must produce **byte-identical** artifacts (the
  artifact-gate oracle): the synthesized-name path is reached only when `(` is
  absent, so a present binding takes the current code path verbatim.
- Public grammar surface only grows (an optional `[ "(" ident ")" ]`); it never
  removes or repurposes existing syntax.

## 2. Current State

Two independent parse paths, both in `src/ast/`:

- **Function-level trap** — `Parser::parse_trap` at `src/ast/items.rs:166`. The
  FUNC/SUB body loop at `src/ast/items.rs:99` recognizes the `TRAP` keyword
  regardless of what follows, then calls `parse_trap`, which currently
  *requires* `(`: `consume_kind(LParen, …)` → `consume_identifier(…)` →
  `consume_kind(RParen, …)` (lines 168–182). It builds `Trap { name, body, line
  }` (`src/ast/types.rs:429`), where `name: String` is the error identifier.

- **Inline postfix trap** — `Parser::maybe_attach_postfix_trap` at
  `src/ast/stmt.rs:347`. It only recognizes a trap when the token *after* `TRAP`
  is `LParen` (the lookahead at lines 354–357). With no `(`, it returns the
  subject expression unchanged, so `TRAP` becomes a stray token and the parser
  emits "Expected end of statement after assignment" — the exact failure seen
  in `examples/audio/src/main.mfb:26`. On the `(` path it consumes
  `( ident )` and builds `Expression::Trapped { expression, binding, handler,
  line }` (`src/ast/types.rs:668`), where `binding: String`.

Downstream, the bound error name flows as an ordinary local through every pass,
always inserted as an `"Error"`-typed local into a cloned handler scope:

- resolver: `src/resolver/resolution.rs:1122` (`handler_locals.insert(binding…)`).
- inference: `src/syntaxcheck/inference.rs:77` (Trapped arm).
- monomorph: `src/monomorph/lower.rs:1274` (clones `binding`, inserts `"Error"`).
- IR lowering, function-level: `src/ir/lower.rs:685-700`
  (`trap_locals.insert(trap.name…)`, emits `IrOp::Trap { name, … }`).
- IR lowering, inline: `src/ir/lower.rs:1311` `lower_inline_trap`, which binds
  `binding` to `IrValue::ResultError { … }` at lines 1382-1393 and pushes a
  `RecoverTarget` so `RECOVER`/`PROPAGATE` resolve.

There is **no unused-binding diagnostic** in `src/resolver/` or
`src/syntaxcheck/` (grep for "unused" is empty), so a bound-but-unreferenced
error name produces no warning.

**Precedent to mirror.** `#` is the internal-sentinel prefix for synthesized
names — the lexer can never produce it for a user identifier
(`is_identifier_continue` = `[A-Za-z0-9_]` at `src/lexer.rs:1117`; identifiers
start alpha/`_`, and `#` is not an identifier char), so a `#`-prefixed name is
collision-proof. Existing `#`-sentinel names: `#{hash}$helper`
(`src/internal_name.rs:131`), `#dump_{list}` / `#idx_{list}`
(`src/testing/desugar.rs:640-641`), `#r{id}` resource keys
(`src/binary_repr/sections.rs:257`). (The separate `$`-prefix used by
`make_temp_local_name` at `src/ir/lower.rs:1676` is the temp-local convention,
not the sentinel; this plan uses the `#` sentinel per project convention.)

**Formatter.** `src/fmt.rs` is token-stream based: `K::Trap =>
Op::Open(Block::Trap)` at `src/fmt.rs:429` opens the block on the keyword alone,
with no dependence on a following `(`. Bare `TRAP` therefore already formats
correctly; only a golden test is needed to lock it.

**Grammar/spec.** The EBNF models only the function-level trap:
`trap = "TRAP" "(" ident ")" block "END" "TRAP" ;` (`19_grammar.md:63`). The
inline postfix trap is prose-only in §8.4. §8.3/§8.4 of `08_error-model.md`
describe `TRAP(err)`/`TRAP(e)`.

## 3. Design Overview

**Chosen approach — synthesize a `#`-sentinel reserved binding at parse time.**
When the `(ident)` is absent, the parser fills `Trap.name` / `Trapped.binding`
with a fixed reserved name that cannot collide with any user identifier (a
`#`-sentinel string, `#err`). The AST field types stay `String`; **every
downstream pass is untouched**. The error is still internally bound to that
name, so:

- `PROPAGATE` in the handler still refers to a live bound error → works.
- `RECOVER`/`RETURN`/`EXIT SUB`/`FAIL e` never needed the name → work.
- Scope is per-handler (each pass clones `handler_locals`), so nested or sibling
  bare traps that all use `#err` never conflict — inner simply shadows outer,
  and neither is user-referenceable anyway.

Parser changes are tiny and local:

1. `parse_trap` (items.rs): make the `( ident )` optional; when absent, use the
   reserved name and skip straight to the block.
2. `maybe_attach_postfix_trap` (stmt.rs): relax the lookahead so `TRAP` is
   recognized as a postfix trap when it is followed by `(` **or** by a
   statement terminator; parse `( ident )` only when the `(` is present, else
   use the reserved name.

**Where the correctness risk concentrates:**

- The inline lookahead relaxation (stmt.rs). It must still (a) respect the
  `allow_else_terminator` guard that forbids attaching a trap inside an inline
  `IF` branch, and (b) not swallow a legitimately different construct. Since
  `TRAP` is a reserved keyword that can never begin a following statement, any
  bare `TRAP` immediately after a completed binding/assignment/expr-statement is
  unambiguously a postfix trap — but this must be covered by a negative/format
  test, not assumed.
- The reserved name must survive IR-verify / binary-repr identifier handling.
  `#`-sentinel names already do (temps/desugar use them), so risk is low; the acceptance
  test that actually builds and runs a bare-TRAP program is the proof.

**Rejected alternative — make the AST fields `Option<String>`.** Change
`Trap.name` and `Trapped.binding` to `Option<String>` and teach resolver,
inference, monomorph, and both IR-lowering sites to skip the insert when `None`.
More faithful to "there is no binding," but it touches ~6 consumers, risks a
missed site (e.g. an unwrap on `binding`), and — critically — is the design that
could **leak** the Error (see §4): if the `None` path stops binding the error to
a trap slot, the routed `Error` arena block loses its scope-drop owner and the
message String is never freed. The synthesized-name approach keeps the exact
slot-based ownership the current cleanup relies on. Rejected for churn, risk, and
the leak hazard, with zero user-visible benefit.

## 4. Memory: the caught `Error` is freed by slot, not by name

This is the concern that decides the design. A trapped `Error` is not freed
because the user named it — it is freed because the handler registers a
slot-keyed scope-drop cleanup. In the function-level lowering,
`src/target/shared/code/builder_control.rs:785-805` (bug-151) pushes an
`ActiveCleanup::OwnedValue { type_: "Error", stack_offset: trap_offset }` as the
**first** owned value of the handler's cleanup scope, so the routed error block
is `arena_free`d exactly once on every handler exit (RETURN / FAIL /
fall-through) and never on the success path that branches over the handler.
Escapes stay safe: `RETURN e` elides via `plan_returned_move` and `FAIL e`
deep-copies in `store_pending_error_from_value` before the free. The inline path
binds the error via an ordinary `IrOp::Bind` of an `Error`-typed local
(`src/ir/lower.rs:1382-1393`), which takes the same `owns_freeable_value`
scope-drop path (`builder_control.rs:239-245`) as any other owned value.

**None of this reads the binding name for a use-count.** The cleanup is keyed on
the trap slot / bind slot, so a bound-but-unreferenced error (a named-unused
`TRAP(e)`, or the synthesized `#err` of a bare TRAP) is dropped identically.

**Invariant this plan must preserve:** a bare trap lowers to the *same*
`IrOp::Trap { name: "#err", … }` / inline `Bind` of an `Error` local as the
named form, so the routed `Error` block still lands in a trap/bind slot that
carries the bug-151 (function-level) or `owns_freeable_value` (inline)
scope-drop cleanup. The plan changes only the **parser**; the lowering and
cleanup paths are untouched, which is exactly why the drop stays correct. This is
proven, not assumed, by the loop leak-regression test in the Validation Plan.

## Compatibility / Format Impact

- **Grammar**: `trap` production gains an optional binding; a new prose line
  documents the inline bare form. Purely additive — no existing program changes
  meaning.
- **Artifacts**: programs that already write `TRAP(e)` take the identical parse
  branch and emit byte-identical `.nobj`/output (artifact-gate oracle). Only
  programs that *newly* use bare `TRAP` produce new artifacts.
- No `.mfp` binary-representation change: a bare trap lowers to the same
  `IrOp::Trap` / inline `IrOp::If` shape as a named trap, just with a
  `#`-sentinel name string.

## Phases

### Phase 1 — Function-level bare `TRAP`

Lowest risk: the FUNC/SUB body loop already recognizes the keyword; only
`parse_trap` needs the `( ident )` made optional.

- [ ] Add a reserved-name constant (`const SYNTHETIC_TRAP_BINDING: &str =
      "#err";`) in `src/ast/items.rs` (or a shared `src/ast/` location reused by
      Phase 2).
- [ ] In `parse_trap` (`src/ast/items.rs:166`): if the next token after `TRAP`
      is `LParen`, parse `( ident )` as today; otherwise use the reserved name
      and proceed directly to `consume_statement_end` + block. Keep the existing
      `Trap { name, body, line }` construction.
- [ ] Tests: add a positive case under `tests/rt-behavior/functions/`
      (e.g. `func-bare-trap-propagate-rt/`) exercising a FUNC bottom trap
      written bare with `PROPAGATE`, and a SUB bottom trap written bare with
      `EXIT SUB`. Generate goldens.
- [ ] Leak test: a fixture that takes a bare function-level trap **in a loop**
      (many iterations, each catching a fresh error) and prints a final
      arena/live-block figure that stays flat — the bug-151 scenario (§4), now
      guarding the bare form. Prove the routed `Error` block is freed per catch.

Acceptance: a FUNC/SUB with a bare bottom `TRAP` compiles and, at runtime,
propagates/handles the error identically to the `TRAP(e)` spelling; the loop
leak test shows no per-catch growth; existing `TRAP(e)` acceptance tests stay
green and byte-identical.
Commit: —

### Phase 2 — Inline postfix bare `TRAP`

Relax the inline lookahead so a bare postfix trap attaches.

- [ ] In `maybe_attach_postfix_trap` (`src/ast/stmt.rs:347`): keep the
      `allow_else_terminator` guard; change the recognition condition so a
      `TRAP` keyword is treated as a postfix trap when the following token is
      `LParen` **or** a statement terminator (end-of-statement / newline / EOF).
      When `(` is present, parse `( ident )` as today; otherwise use the shared
      reserved name. Everything after (handler loop, `END TRAP`, `Trapped { … }`
      construction) is unchanged.
- [ ] Tests: add a positive case under `tests/rt-behavior/` (mirror the fixed
      `examples/audio` shape: `idx = toInt(s) TRAP` / `RECOVER -1` / `END TRAP`)
      covering LET, MUT, assignment, and bare-expression-statement inline forms;
      plus a negative `tests/syntax/` case confirming a bare `TRAP` is still
      rejected where inline TRAP is illegal (e.g. inside an inline `IF … THEN …`
      branch) with a golden `build.log`.
- [ ] PROPAGATE test: a fixture whose bare inline handler does `PROPAGATE` and
      whose caller (or entry-point trap) observes the re-propagated error's
      `code`/`message`/`source` unchanged — proving the trapped error stays live
      under the bare inline form (§4).

- [ ] Leak test: a fixture taking a bare **inline** trap in a loop (e.g.
      `MUT n = toInt(bad) TRAP` / `RECOVER 0` / `END TRAP` repeated), asserting
      the arena/live-block count stays flat across iterations — proves the
      inline routed `Error` is freed per catch under the bare form (§4).

Acceptance: `x = call() TRAP` / `RECOVER v` / `END TRAP` compiles and recovers
at runtime identically to `TRAP(e)`; the loop leak test shows no per-catch
growth; the illegal-position case still errors with the same diagnostic as
today; `TRAP(e)` inline tests stay byte-identical.
Commit: —

### Phase 3 — Docs, grammar, and formatter lock (highest-visibility work last)

- [ ] `src/docs/spec/language/19_grammar.md:63`: change to
      `trap = "TRAP" [ "(" ident ")" ] block "END" "TRAP" ;` and add a prose
      note that the inline postfix trap's binding is likewise optional.
- [ ] `src/docs/spec/language/08_error-model.md`: in §8.3/§8.4 note that the
      `(err)` binding may be omitted when the handler does not reference the
      error, and that bare-handler `PROPAGATE` still re-propagates the trapped
      error. Keep all existing `TRAP(e)` examples.
- [ ] Update `.ai/specifications.md` if it enumerates trap syntax; re-run
      `mfb spec` to confirm the embedded spec renders the new production.
- [ ] Fix `examples/audio/src/main.mfb` to the intended bare-TRAP form (drop the
      invalid `RESUME` for `RECOVER`), so the shipped example builds and
      demonstrates the feature end-to-end.
- [ ] Tests: add a `mfb fmt` golden confirming bare `TRAP` (both positions)
      round-trips unchanged, alongside the existing
      `inline_trap_block_indents_handler` fmt test (`src/fmt.rs:745`).

Acceptance: `mfb spec language error-model` and `… grammar` render the optional
binding; `examples/audio` builds and runs; `mfb fmt` leaves bare `TRAP`
untouched (idempotent) in a golden test.
Commit: —

## Validation Plan

- Tests: positive rt-behavior fixtures for function-level and inline bare TRAP
  (RECOVER, PROPAGATE, EXIT SUB, FAIL paths); negative syntax fixture for
  bare TRAP in an illegal inline position; fmt round-trip golden. Follow the
  4-folder split (`tests/{acceptance,syntax,rt-error,rt-behavior}`, fixtures by
  name).
- Runtime proof: build and run the fixed `examples/audio` (and a minimal
  `LET n = toInt("x") TRAP` / `RECOVER -1` program) and observe the recovered
  value / propagated error on stdout/stderr — end-to-end, not just unit tests.
- Leak proof: run the bare-trap-in-a-loop fixtures (function-level and inline)
  for a large iteration count and confirm the arena/live-block figure is flat —
  the caught `Error` is freed on every catch (§4, bug-151). This is the concrete
  check that "no name in scope" did not turn into "no owner, so it leaks."
- PROPAGATE proof: run the function-level and inline bare-handler PROPAGATE
  fixtures and confirm the re-propagated `Error` reaches the caller / entry-point
  trap with `code`, `message`, and `source` identical to the origin — bare TRAP
  keeps the error live internally even though the user cannot name it.
- Doc sync: `19_grammar.md`, `08_error-model.md`, `.ai/specifications.md`, and
  re-render via `mfb spec`.
- Acceptance: run the execution-free artifact gate first
  (`scripts/artifact-gate.sh`, per `.ai/compiler.md`) to confirm existing
  `TRAP(e)` artifacts are byte-identical, then the full `scripts/test-accept.sh`
  golden cycle. Do not rebuild the binary while acceptance runs (SIGKILL hazard
  on macOS).

## Open Decisions

- Reserved binding spelling — use `"#err"` (the `#` internal-sentinel prefix,
  guaranteed non-collision because `#` is not an identifier char; mirrors the
  conventional `err`/`e` name). Alternative considered and rejected: a
  per-occurrence unique `"#err{n}"` — unnecessary because handler scopes are
  cloned, so a single fixed `#err` never collides. (§3)

## Summary

The real engineering risk is the one-line lookahead relaxation in
`maybe_attach_postfix_trap` (must preserve the inline-IF guard and not
mis-attach) and confirming the `#`-sentinel synthetic name survives IR-verify —
both nailed by an actually-building, actually-running acceptance test. The AST
field types, all downstream passes (resolver, inference, monomorph, IR lowering,
codegen), the fallible-call ABI, and the `.mfp` format are left completely
untouched; existing `TRAP(e)` programs stay byte-identical.
