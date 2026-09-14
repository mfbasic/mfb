# plan-136-A: Parameter defaults resolve at the declaration and are filled on every call form

Last updated: 2026-09-13
Overall Effort: x-large (1d–3d) — letters A (large), B (large), C (large)
Effort: large (3h–1d)
Depends on: nothing

plan-136 closes **bug-614** (`bugs/bug-614-parameter-default-reading-a-global-lowers-in-the-callers-scope.md`)
and the defects found while planning it. The owner settled the language rules on 2026-09-13:

1. A parameter default's names resolve **where the function is declared** — never through a
   caller's locals.
2. A default is evaluated **on each call** that omits the argument.
3. A default is **external** to its function: it may not name any parameter of that function
   (to give two parameters the same default, write the same default twice). Naming one is a
   located compile error.
4. **Packages and executables behave identically**, so the `.mfp` format carries non-constant
   defaults (plan-136-B).
5. **No local binding may share a name with a visible top-level `LET`/`MUT`** (plan-136-C).

**This letter (A)** makes rules 1–3 true for everything inside one project: every call that omits a
defaulted argument of a `FUNC`, `SUB` or `LINK` function passes the default evaluated on that call
in the declaration's scope. A default that names a parameter, and a lambda parameter default
(which `mfb man lambda` already says is not allowed), are located compile errors.

References:

- `bugs/bug-614-parameter-default-reading-a-global-lowers-in-the-callers-scope.md` — the bug.
- `mfb spec language functions` (`src/docs/spec/language/06_functions.md`, "Default args",
  "Named args", "Overload resolution": defaults never combine with overloading).
- `mfb spec language memory-semantics` (`src/docs/spec/language/14_memory-semantics.md`:
  "Default arguments are evaluated at the call site…") — to be revised.
- `mfb spec language bindings-and-scope` (`05_bindings-and-scope.md`, resolver scope model).
- `.ai/compiler.md` (Hard Completion Gate, Validation), `.ai/codegen-invariants.md` ("A rename fix
  can leak a mangled name into a diagnostic"), `.ai/testing-gates.md` (acceptance harness).
- Precedent fix: bug-612 `53f10b1fe` — `ir::lower::lower_binding` lowers a top-level initializer
  with an empty local scope; its runtime test `tests/runtime/rt_top_level_initializer_globals.rs`
  is the harness to copy.

## Prerequisites

This is the gate for the whole of plan-136; letters B and C point here.

| Must be true | Command | Status |
|---|---|---|
| The bug-612/613 empty-scope fix is on main (the precedent A mirrors) | `git merge-base --is-ancestor 53f10b1fe main && echo MET` | MET (2026-09-13) |
| No other session has claimed plan-136 | `git log --all --oneline --grep='plan-136'` → only this plan's commits; `ls planning planning/completed \| grep plan-136` → only `plan-136-A/B/C` | MET (2026-09-13) |
| bug-614 is still open and unfixed | `ls bugs/bug-614-*.md` → one file | MET (2026-09-13) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- Every call in a project that omits a defaulted argument of a user `FUNC`/`SUB`/`LINK` function —
  positionally or around named arguments — passes that default's value **evaluated on that call,
  with names resolved at the declaration**. The bug-614 program prints `5`, and
  `f()` with `FUNC f(x AS Integer = helper())` prints `helper()`'s value even when the caller has a
  local lambda named `helper`.
- A default that names a parameter of its own function is a **located** error (a new
  `SYMBOL_` rule), never the unlocated `NIR local reference '<name>' does not resolve`.
- A lambda parameter default is a **located** error (a new rule), never silently dropped.

### Non-goals (explicit constraints)

- **Literal defaults lower byte-identically.** A default that is a literal (string, number,
  boolean, scalar, `NOTHING`) or a built-in package constant produces the same IR and native code
  as today. The containment of golden diffs to fixtures with computed defaults IS the check.
- **Registry/built-in defaults do not change:** `codegen::registry::default_argument_padding`
  (`DefaultValue::Fill`), the `strings.padLeft`/`padRight` pad in `ir::lower`,
  `normalize_builtin_call_arguments`, and `target::shared::nir::lower::apply_default_args`
  (`fs.*`). They are call-site constants by design.
- **No `.mfp` format change in this letter** — plan-136-B owns it.
- **No shadowing rule in this letter** — plan-136-C owns it.
- Defaults still never combine with overloading (06_functions "Overload resolution"), and a call
  through a function value still carries no defaults (`ir::shape` `CalleeParams::FunctionValue`).
- Argument evaluation order stays as `06_functions` "Named args" states (named arguments are
  evaluated in declaration order after omitted defaults are filled).
- No change to how an explicit argument is passed.

## 2. Current State

Read for this plan on 2026-09-13; symbols cited, not lines.

**Representation.** `ast::types::Param.default: Option<Expression>`; `ast::items::parse_params`
accepts any expression after `=` (grammar `19_grammar.md`: `param = … [ "=" expr ]`).
`hir::HirParam.default: Option<HirExpression>` (`hir::elaborate_param`); LINK:
`hir::HirLinkParam.default: Option<ast::Expression>` (cloned by `elaborate_link_function`).
`ir::types::IrParam.default: Option<IrValue>`, set by `ir::lower::lower_param`, which lowers the
default **with the callee's own scope** and the parameter type as the expected type (so
`a AS List OF Fixed = [1, 2]` coerces).

**The capture (bug-614).** Call sites do not use `IrParam.default`. `ir::lower::function_params`
builds a side table of `CallParam { default: Option<HirExpression> }`, and
`ir::lower::lower_local_call_arguments` fills an omitted slot with
`lower_expression_with_expected(default, Some(&param.type_), locals, context)` — the **caller's**
`locals`. It is the only filler for user functions: `grep -rn '\.default\b' src --include='*.rs' |
grep -v 'Default::default' | wc -l` → 44 readers, of which this is the one that turns a user default
into an argument. `normalize_local_call_arguments` only orders arguments. The identifier arm looks
at `locals` before globals, and `resolve_callable` looks at locals before functions, so a caller
local (or local lambda) with the default's name wins.

**Front-end checks let a parameter reference through.** `resolver::resolution::resolve_function`
inserts each parameter into `locals` and then resolves its default against that map;
`ast::scope_privates` rewrites a default with the earlier parameters as locals; `ir::verify`
(`check_value(default, &locals)`) and `ir::shape::walk_function` likewise see the parameters. No
check rejects a default that names a parameter, so it reaches NIR and fails unlocated (P2).

**Lambdas.** `parse_lambda` reuses `parse_params`, so `LAMBDA(x AS Integer = 4) -> …` parses and
resolves; lambda lowering (the `HirExpression::Lambda` arm in `ir::lower`) builds
`IrParam { default: None }` — the default is silently dropped (P9), while `mfb man lambda` says
"parameters cannot declare default values".

**LINK.** `HirLinkParam.default` is never read by `ir::lower_link`; LINK functions live in
`function_types`, not `function_params`, so an omitted defaulted argument is simply not passed (P10).

**Generics.** The monomorphizer (`monomorph::lower::lower_function`) copies `param.default`
unchanged into each concrete instance; `ir::lower` then lowers concrete functions. Packages carry
no open templates (`src/docs/spec/package/04_type-table.md`: "There are no open template
declarations in package binary representation").

**Precedents the design mirrors.**
- Lambda lifting: the `HirExpression::Lambda` arm of `ir::lower` names a synthesized function
  `$lambda{n}`, pushes an `IrFunction { visibility: "private", … }` onto `LowerContext.lambdas`, and
  `lower_augmented_project` appends them. `ir::verify` skips `$`-named functions for return-type
  rules. `src/internal_name.rs` reserves `$` for synthesized names.
- bug-612: `lower_binding` lowers with an empty `HashMap` local scope.
- An inline `TRAP` cannot be a value: the `HirExpression::Trapped` arm of
  `lower_expression_with_expected` is `unreachable!("inline TRAP must be lowered as a statement
  value")`. Statement lowering (`lower_statement_inner`) owns every statement-position desugar.

### Measured populations

| What | Count | Command |
|---|---|---|
| `.default` readers in `src/` | 44 | `grep -rn '\.default\b' src --include='*.rs' \| grep -v 'Default::default' \| wc -l` |
| Fixture lines declaring a default | 7 in 6 files | `grep -rEn --include='*.mfb' '\b(FUNC\|SUB)\b[^(]*\([^)]*[^:<>=!]=[^=>]' tests \| wc -l` → 7 |
| …of which a **computed** (non-literal) default | 1 fixture: `tests/rt-behavior/functions/user-function-default-args-result-valid` (`mark("default-extra", 2)`) | same grep, read each line |
| Inline Rust test sources declaring a default | 13 lines in 9 files | `grep -rEn --include='*.rs' '(FUNC\|SUB) [A-Za-z_]+\([^)]*AS [A-Za-z ]+ = ' src tests \| wc -l` |
| Existing default diagnostics | `TYPE_DEFAULT_ARG_ORDER` `2-203-0026`, `TYPE_DEFAULT_VALUE_MISMATCH` `2-203-0027` | `grep -n -B1 'TYPE_DEFAULT_ARG_ORDER\|TYPE_DEFAULT_VALUE_MISMATCH' src/rules/table.rs` |
| Highest `2-201` SYMBOL code | `2-201-0018` | `grep -n '"2-201-00' src/rules/table.rs \| tail -1` (re-check at implementation — codes race between sessions) |

### Verified properties (probes, 2026-09-13)

Run with `target/release/mfb` built 2026-09-13 17:34 (the only later main commit, `14c9fc1ca`,
touched `planning/plan-133-*` only), in `mfb init` projects under `/tmp`.

| # | Program | Result |
|---|---|---|
| P1 | bug-614 repro: global `limit = 5`, `f(x = limit)`, caller `LET limit = 99` | prints `99` (bug doc, measured at `53f10b1fe`) |
| P2 | `FUNC f(a AS Integer, b AS Integer = a)`; `f(3)` | build exit 1, unlocated `error: NIR local reference 'a' does not resolve` |
| P3 | `MUT current = 1`; `f(x = current)`; `f()`, `current = 2`, `f()` | `1` then `2` — already evaluated per call |
| P4 | `f(x = next1())`, `next1` increments a `MUT` | `1 2` — already evaluated per call |
| P5 | global `limit`; parameter `limit`; `FOR limit = 1 TO 2` | builds, prints `1` `2` (C rejects) |
| P9 | `LAMBDA(x AS Integer = 4) -> x + 10`; `f(5)` | builds, prints `15`; default silently dropped |
| P10 | `LINK "c"` `FUNC absval(n AS Integer = -5)` → `abs`; `absval(-3)`, `absval()` | `3`, then runtime `Error: 7-705-0010` (overflow), exit 255 — omitted argument never passed |
| P11 | `FUNC helper() = 5`; `FUNC f(x = helper())`; caller `LET helper = LAMBDA() -> 99`; `f()` | prints `99` — a function-call default is captured by a caller local |

P11 is the regression shape that survives plan-136-C: C bans locals named like top-level
`LET`/`MUT` bindings, not like functions.

**UNVERIFIED (Phase 1 measures):** which of the 44 readers break on a `Call`-shaped
`IrParam.default`; how an uncaught error raised inside a synthesized `$lambda` body names its
function (the precedent for the hidden default function).

## 3. Design Overview

Two kinds of default, decided once per parameter by a total classifier over `HirExpression`:

- **Literal** — a string, number, boolean or scalar literal, `NOTHING`, or a built-in package
  constant (`builtins::is_package_constant`): exactly the forms `ir::lower` lowers to a scalar
  `IrValue::Const`. It contains no names, so scope cannot matter. The call site lowers it exactly
  as today (with an empty local scope, which is equivalent for a name-free expression).
  **Byte-identical output.**
- **Computed** — everything else (a global, a function call, `-5`, `[1, 2]`, an enum member, …).
  For each concrete function, `ir::lower` synthesizes a **hidden default function**
  `$default$<function>$<index>`: private, no parameters, returns the parameter's type, body a single
  `RETURN <default>` lowered with an **empty** local scope and the parameter type as expected type,
  located at the parameter's line in the declaring file. `IrParam.default` becomes a zero-argument
  call to it, and every call site that omits the argument emits that call.

The resolver resolves every default against an **empty** local scope (declaration scope = the
file's top level: globals, functions, imports, own-file `PRIVATE` names). A name that is one of the
function's parameters gets a dedicated located error. Lambda parameter defaults get a located error.

Evaluation per call is automatic: each omission is its own call to the hidden function.

**Correctness risk** concentrates in `lower_local_call_arguments`/`function_params` (every local
call with an omitted argument) and in the readers of `IrParam.default` (`ir::verify`'s
default-value checks, `ir::shape`, `optimizer::opt1::local_rewrites`,
`target::shared::validate::body`). **Design uncertainty** concentrates in those readers accepting a
call-shaped default and in how a raise inside the hidden function is reported — Phase 1 measures
both before any lowering changes.

**Byte-identity is not this letter's gate** — behavior changes. What must hold: literal-default
fixtures are byte-identical; the only `.ir`/native golden expected to diff is
`user-function-default-args-result-valid` (its two computed defaults become calls to two hidden
functions). A diff anywhere else is a bug to root-cause (inspect one fixture), not a verdict on the
design.

**Rejected alternatives.**
- *Lower the default at the call site with an empty scope* (bug-614's first design): fixes
  executables, but gives the importer nothing to call across a package boundary (B), and keeps
  re-lowering the expression per call site.
- *Copy the callee's declaration-lowered `IrParam.default` into each call*: sound only for pure
  value trees — a statement-position desugar cannot travel inside an `IrValue` (the inline-TRAP
  value arm is `unreachable!`) — and an importer lowers its calls before package IR is merged, so it
  would need a merge-time patch pass.
- *Every default through a hidden function, literals included*: churns every literal-default golden
  and (B) every existing package's bytes and `sigHash`, for no semantic gain.
- *Evaluate once at startup*: owner ruled per call. *Caller-scope resolution*: owner rejected.

## 4. Detailed Design

### 4.1 Classifier

`ir::lower::default_kind(&HirExpression) -> DefaultKind { Literal, Computed }` — a `match` with **no
wildcard arm** (a new `HirExpression` variant is a build error, per `.ai/codegen-invariants.md`
"A classifier's fall-through default is a decision nobody made"). `Literal` only for the forms named
above; a built-in package constant is `Literal` iff lowering it yields `IrValue::Const` (read
`ir::lower`'s `is_package_constant` arm to confirm the predicate matches).

### 4.2 Hidden default functions

- Name: `internal_name::hidden_default_function_name(function: &str, index: usize)` →
  `$default${function}${index}`; predicate `internal_name::is_hidden_default_function`. `function`
  is the concrete (post-monomorph) function name, so each instantiation gets its own.
- Created while lowering each concrete HIR function, beside lambda lifting, pushed with
  `visibility: "private"`, `kind: "func"`, `file` = the function's file, `loc` = the parameter line.
- Body: lower a synthesized `RETURN <default>` statement through `lower_statement_inner` with an
  empty local map, so every statement-position desugar applies.
- A `SUB` or `LINK` function's computed default is the same.

### 4.3 Call sites

`CallParam.default` becomes `Option<CallDefault>`, `enum CallDefault { Literal(HirExpression),
Function(String) }`. `lower_local_call_arguments` fills an omitted slot with the literal (empty
locals, parameter type expected) or a zero-argument call to the named hidden function, typed as the
parameter. LINK calls with omitted arguments are routed through the same fill (find the LINK call
path: `grep -n 'function_types' src/ir/lower.rs`).

### 4.4 Front end

- `resolve_function`: resolve each default with an empty local map. An unresolved identifier that
  equals a parameter name of the same function reports the new rule
  `SYMBOL_DEFAULT_NAMES_PARAMETER` at the parameter's line: *"the default value of `b` cannot use
  parameter `a`; a default is evaluated outside the function"*. Names pass through
  `internal_name::display_name`.
- `ast::scope_privates`: rewrite a default with an empty local set.
- `ir::verify` and `ir::shape::walk_function`: check a default with an empty local map.
- Lambda: the `HirExpression::Lambda` arm of `resolve_expression` reports the new rule
  `SYMBOL_LAMBDA_PARAMETER_DEFAULT` for any lambda parameter carrying a default.

## Phases

> **NOTE — keep the checkboxes current as you go.**
> Tick `- [x]` in the same commit as the work. `- [~]` = partial, say what remains. Moot:
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the moment a phase lands. Add any task you
> discover. **An unticked box means NOT DONE.**

### Phase 1 — RED tests and the reader audit (no behavior change)

- [ ] Add `tests/runtime/rt_parameter_default_scope.rs`, copying the build/run helpers of
      `tests/runtime/rt_top_level_initializer_globals.rs`, and register it in `Cargo.toml` exactly as
      that test is (`grep -n -B1 -A2 'rt_top_level_initializer_globals' Cargo.toml`). Cases:
  - [ ] `a_default_calling_a_function_ignores_a_callers_same_named_local` — P11, expects `5`
        (RED: `99`).
  - [ ] `a_default_reading_a_global_ignores_a_callers_same_named_local` — P1, expects `5`
        (RED: `99`). plan-136-C converts this case; its four answers are recorded there.
  - [ ] `a_named_call_fills_an_omitted_default_in_declaration_scope` — `FUNC f(x AS Integer =
        helper(), y AS Integer = 0)`, caller has `LET helper = LAMBDA() -> 99`, calls `f(y := 1)`,
        expects `5` (RED).
  - [ ] `a_sub_default_ignores_a_callers_same_named_local` — the P11 shape on a `SUB` that prints
        its parameter (RED).
  - [ ] `a_generic_function_default_is_filled_per_instantiation` — a generic function (template
        parameters per `19_grammar.md` `funcDecl`) with the P11 default on a non-generic parameter,
        called at two instantiations, both print `5` (RED).
  - [ ] `a_link_function_fills_an_omitted_default` — P10 (`libraries` entry copied from
        `tests/rt-error/native/native-cbuffer-overrun-rt/project.json`), `absval()` prints `5`
        (RED: `7-705-0010`).
  - [ ] `a_default_reading_a_mut_global_sees_its_value_on_each_call` — P3, `1` then `2` (GREEN pin).
  - [ ] `a_default_calling_a_function_runs_on_every_call` — P4, `1 2` (GREEN pin).
  - [ ] `a_literal_default_is_unchanged` — `FUNC greet(name AS String, greeting AS String =
        "Hello")`, `greet("Ada")` prints `Hello Ada`-shaped output (GREEN pin).
- [ ] Syntax fixtures (placeholder goldens `golden/{<pkg>.ast,<pkg>.ir,build.log}` + `<pkg>.run`
      so the plain build runs, per `.ai/testing-gates.md`). First prove each leaf name unused:
      `find tests -name '<leaf>' | wc -l` → 0.
  - [ ] `tests/syntax/functions/default-names-a-parameter-invalid` — P2 plus a default naming
        itself (`b AS Integer = b`) and a default naming a LATER parameter (RED today: unlocated
        NIR error / whatever it prints — record it).
  - [ ] `tests/syntax/functions/lambda-parameter-default-invalid` — P9 (RED today: builds).
- [ ] Audit the 44 readers (`grep -rn '\.default\b' src --include='*.rs' | grep -v
      'Default::default'`). For each, record in **Corrections** (or a table under this phase):
      does it (a) resolve/check a default with parameters in scope, (b) assume
      `IrParam.default` is a value tree rather than a call, (c) need no change. At minimum read:
      `ir::verify` default checks (`TYPE_DEFAULT_VALUE_MISMATCH` via `infer_type`,
      `check_value_captures`), `ir::shape::walk_function`, `optimizer::opt1::local_rewrites`,
      `target::shared::validate::body`, `manifest::entry`, `ast::scope_privates`,
      `ir::package::visit_project_targets_mut`, `binary_repr::writer::lower_function`.
- [ ] Measure how an uncaught error raised inside a lambda body prints (build a probe in `/tmp`
      whose lambda calls `toInt("nope")`); record the output. That is the precedent the hidden
      default function's error report must match or improve.

Acceptance: every RED case fails for the documented reason; every GREEN pin passes; the audit
table and the lambda-error measurement are recorded in this file.
  Check: `cargo build --release && cargo test --release --test rt_parameter_default_scope --
  --test-threads=1` → the six RED cases fail, three pins pass (est. 6 min: release build + ~9
  builds); `scripts/test-accept.sh target/release/mfb /tmp/p136a-p1-accept
  default-names-a-parameter-invalid lambda-parameter-default-invalid` → both mismatch against the
  placeholders (est. 1 min). (The second argument is `rm -rf`ed — always a fresh `/tmp` path.)
Commit: —

### Phase 2 — the front end sees only the declaration scope

- [ ] `src/rules/table.rs`: add `SYMBOL_DEFAULT_NAMES_PARAMETER` and
      `SYMBOL_LAMBDA_PARAMETER_DEFAULT` (error) at the next free `2-201` codes — re-check with
      `grep -n '"2-201-00' src/rules/table.rs | tail -1` and `git log --all -S'2-201-0019'
      --oneline` immediately before choosing.
- [ ] `src/docs/spec/diagnostics/01_rule-codes.md`: one row each in the `2-201` table; update the
      `2-201` population in "Subsystems (`SSS`)".
- [ ] `resolver::resolution::resolve_function`: resolve defaults with an empty local map; report
      `SYMBOL_DEFAULT_NAMES_PARAMETER` (located, `display_name`) when an unresolved name is a
      parameter of the function.
- [ ] `resolver::resolution::resolve_expression` `HirExpression::Lambda`: report
      `SYMBOL_LAMBDA_PARAMETER_DEFAULT` for a lambda parameter with a default.
- [ ] `ast::scope_privates`: rewrite defaults with an empty local set.
- [ ] `ir::verify` / `ir::shape::walk_function`: check defaults with an empty local map (per the
      Phase 1 audit).
- [ ] Generate the two syntax fixtures' goldens (`scripts/sync-goldens.sh target/release/mfb
      default-names-a-parameter-invalid lambda-parameter-default-invalid`) and read each
      `build.log`: one located error per offending parameter, no NIR line.

Acceptance: both syntax fixtures pass with located errors; rules census passes.
  Check: `cargo build --release && scripts/test-accept.sh target/release/mfb /tmp/p136a-p2-accept
  default-names-a-parameter-invalid lambda-parameter-default-invalid` → `acceptance tests passed (2
  test(s) ran)` (est. 5 min); `cargo test --release --bin mfb rules::` → `test result: ok`
  (est. 3 min).
Commit: —

### Phase 3 — hidden default functions and call-site filling

- [ ] `src/internal_name.rs`: `hidden_default_function_name`, `is_hidden_default_function`, and
      `display_name` handling so a user-facing message never shows `$default$` (per the Phase 1
      lambda measurement), with unit tests.
- [ ] `ir::lower`: `DefaultKind` + `default_kind` (total match, unit-tested per literal form and
      one computed form of each shape the grep finds).
- [ ] `ir::lower`: synthesize hidden default functions for each concrete function's computed
      defaults (§4.2); set `IrParam.default` to the zero-argument call.
- [ ] `ir::lower`: `CallParam.default: Option<CallDefault>`; `function_params` builds it;
      `lower_local_call_arguments` fills per §4.3.
- [ ] `ir::lower` / `ir::lower_link`: LINK calls fill omitted defaulted arguments through the same
      path; a computed LINK default gets its hidden function.
- [ ] Apply every change the Phase 1 audit found (readers that assumed a value tree or a
      parameter scope).
- [ ] Update the stale comments that state the old model: `ir::verify` ("evaluated in the caller's
      frame") and `optimizer::opt1::local_rewrites` ("Defaults are lowered at call sites").
- [ ] Unit tests in `src/ir/tests.rs`: `a_computed_default_lowers_to_a_hidden_function_call`,
      `a_literal_default_lowers_as_a_constant`; re-run the existing default tests
      (`local_call_named_and_default_arguments_lower_in_param_order`,
      `lowers_default_argument_padding_for_local_call`, `call_with_named_and_default_arguments`) —
      if one fails, answer AGENTS.md's four questions in this file before touching it.

Acceptance: all `rt_parameter_default_scope` cases pass; literal-default fixtures byte-identical;
the only golden diff is `user-function-default-args-result-valid`.
  Check: `cargo build --release && cargo test --release --test rt_parameter_default_scope --
  --test-threads=1` → all pass (est. 6 min); `cargo test --release --bin mfb ir::` → `test result:
  ok` (est. 4 min); `scripts/test-accept.sh target/release/mfb /tmp/p136a-p3-accept default named
  functions` → the only mismatch is `user-function-default-args-result-valid`'s `.ir`, whose diff
  is exactly two hidden functions plus the two call sites (inspect it, then `sync-goldens.sh` that
  one fixture) (est. 3 min).
Commit: —

### Phase 4 — spec and man sync

- [ ] `06_functions.md` "Default args": a default's names resolve at the declaration (globals,
      functions, imports, own-file `PRIVATE` names), never a parameter or a caller local; it is
      evaluated on each call that omits the argument; literal vs computed; the two new rules;
      `[[path:symbol]]` citations to `default_kind` and `hidden_default_function_name`.
- [ ] `14_memory-semantics.md`: replace "Default arguments are evaluated at the call site" with the
      per-call, declaration-scope rule.
- [ ] `src/docs/spec/architecture/04_ir.md`: hidden `$default$` functions beside `$lambda`.
- [ ] `mfb man`: find the narrative topic that teaches `FUNC` defaults (`grep -rln -i 'default'
      src/docs/man/*/package.md`); add that a default cannot use the function's other parameters
      and is evaluated on each call, with a compiled example (build it in `/tmp`; man examples are
      unchecked — `.ai/man-content.md`); check `scripts/man-census.sh --memory-scope` → 0
      unclassified.
- [ ] Citations: `cargo test --release --bin mfb spec` → ok; `scripts/spec-census.sh --citations` →
      no new unresolved.

Acceptance: `mfb spec language functions` states the rule with resolving citations.
  Check: `cargo test --release --bin mfb spec` → `test result: ok` (est. 3 min);
  `target/release/mfb spec language functions | grep -n -i 'declar'` → the new sentences.
Commit: —

## Validation Plan

- Tests: `tests/runtime/rt_parameter_default_scope.rs` (runtime, RED→GREEN plus pins), two
  `tests/syntax/functions/*-invalid` fixtures, `src/ir/tests.rs` and `src/internal_name.rs` unit tests.
- Coverage check: the fill path is exercised by the RED cases at runtime (a lowering that ignored
  `CallDefault::Function` keeps P11 printing `99`).
- Runtime proof: P11 prints `5`; P10's `absval()` prints `5`.
- Doc sync: Phase 4.
- Final gate: runs **once, at the end of plan-136-C** (the full suite covers all three letters).

## Open Decisions

- Rule names — recommended `SYMBOL_DEFAULT_NAMES_PARAMETER` and `SYMBOL_LAMBDA_PARAMETER_DEFAULT`
  (resolver errors, `2-201`) vs. `TYPE_`-range names next to `TYPE_DEFAULT_ARG_ORDER`. They are
  name-resolution facts, so `SYMBOL_`.
- `-5` is `Computed` (it lowers as `IrValue::Unary`, not `Const`) — recommended: accept the hidden
  call; folding unary-minus literals into `Literal` is an optimization this plan does not need.

## Corrections

<Filled in during execution.>

## Summary

The risk is the single call-site filler every omitted argument passes through, and the readers of
`IrParam.default` that assumed a value tree; literal defaults are held byte-identical to bound it.
Untouched: registry defaults, the `.mfp` format (B), and shadowing (C).
