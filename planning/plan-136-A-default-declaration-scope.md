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
| The bug-612/613 empty-scope fix is on main (the precedent A mirrors) | `git merge-base --is-ancestor 53f10b1fe main && echo MET` | MET (2026-09-13, re-run at execution start → `MET`) |
| No other session has claimed plan-136 | `git log --all --oneline --grep='plan-136'` → only this plan's commits; `ls planning planning/completed \| grep plan-136` → only `plan-136-A/B/C` | MET (2026-09-13, re-run at execution start → only `caf191edd`; only the three A/B/C files) |
| bug-614 is still open and unfixed | `ls bugs/bug-614-*.md` → one file | MET (2026-09-13, re-run at execution start → one file) |

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

- [x] Add `tests/runtime/rt_parameter_default_scope.rs`, copying the build/run helpers of
      `tests/runtime/rt_top_level_initializer_globals.rs`, and register it in `Cargo.toml` exactly as
      that test is (`grep -n -B1 -A2 'rt_top_level_initializer_globals' Cargo.toml`). Cases
      (`cargo test --release --test rt_parameter_default_scope -- --test-threads=1` →
      `test result: FAILED. 3 passed; 6 failed`):
  - [x] `a_default_calling_a_function_ignores_a_callers_same_named_local` — P11, expects `5`
        (RED: `left: ["99", "99"] right: ["5", "99"]`).
  - [x] `a_default_reading_a_global_ignores_a_callers_same_named_local` — P1, expects `5`
        (RED: `left: ["99"]`). plan-136-C converts this case; its four answers are recorded there.
  - [x] `a_named_call_fills_an_omitted_default_in_declaration_scope` — `FUNC f(x AS Integer =
        helper(), y AS Integer = 0)`, caller has `LET helper = LAMBDA() -> 99`, calls `f(y := 1)`,
        expects `5` (RED: `left: ["991"] right: ["51"]` — the test prints `x * 10 + y`).
  - [x] `a_sub_default_ignores_a_callers_same_named_local` — the P11 shape on a `SUB` that prints
        its parameter (RED: `left: ["99"]`).
  - [x] `a_generic_function_default_is_filled_per_instantiation` — a generic function (template
        parameters per `19_grammar.md` `funcDecl`) with the P11 default on a non-generic parameter,
        called at two instantiations, both print `5` (RED: `left: ["99", "99"]`).
  - [x] `a_link_function_fills_an_omitted_default` — P10 (`libraries` entry copied from
        `tests/rt-error/native/native-cbuffer-overrun-rt/project.json`), `absval()` prints `5`
        (RED: `the app exited exit 255: 3 / Error: 7-705-0010`).
  - [x] `a_default_reading_a_mut_global_sees_its_value_on_each_call` — P3, `1` then `2` (GREEN pin: `ok`).
  - [x] `a_default_calling_a_function_runs_on_every_call` — P4, `1 2` (GREEN pin: `ok`).
  - [x] `a_literal_default_is_unchanged` — `FUNC greet(name AS String, greeting AS String =
        "Hello")`, `greet("Ada")` prints `Hello Ada`-shaped output (GREEN pin: `ok`).
- [x] Syntax fixtures (placeholder golden `golden/build.log` — see Corrections). First prove each
      leaf name unused: `find tests -name '<leaf>' | wc -l` → 0 for both.
  - [x] `tests/syntax/functions/default-names-a-parameter-invalid` — P2 plus a default naming
        itself (`b AS Integer = b`) and a default naming a LATER parameter. RED today (recorded):
        exit 1 with ONE diagnostic, `main.mfb:12 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]:
        Identifier `c` is not declared in this scope.` for the later-parameter case only; the
        earlier/itself cases resolve (parameters are in the resolver's scope) and are masked by
        that error before they could reach P2's NIR failure.
  - [x] `tests/syntax/functions/lambda-parameter-default-invalid` — P9 (RED today: `[exit 0]`,
        writes `.ast`/`.ir`).
- [x] Audit the 44 readers (`grep -rn '\.default\b' src --include='*.rs' | grep -v
      'Default::default' | wc -l` → 44, re-run 2026-09-13). Table below.
- [x] Measure how an uncaught error raised inside a lambda body prints: `/tmp/p136-lambda-err`,
      `LET bad = LAMBDA(s AS String) -> toInt(s)`, `bad("nope")` → `Error: 7-705-0003` /
      `Text parse or non-finite numeric representation conversion failed.`, exit 255. The runtime
      report names **no function at all**, so a raise inside a hidden `$default$` function cannot
      leak its name at runtime; `display_name` handling matters only for compile-time messages.

**Reader audit** (a = checks/lowers a default with parameters in scope; b = assumes a value tree,
not a call; c = no change). Key fact: a computed default is **already** a call-shaped
`IrParam.default` today — `user-function-default-args-result-valid`'s `mark("default-extra", 2)`
lowers to `IrValue::Call`, and that fixture builds and runs — so no executable-path reader is (b);
the only value-tree-only reader is the package writer (plan-136-B's scope).

| # | Reader | Verdict |
|---|---|---|
| 1 | `ir/json.rs` `IrParam::to_json` | c — serializes any value |
| 2 | `optimizer/opt1/local_rewrites.rs` | c — already rewrites a default in an empty `Scopes`; its comment ("lowered at call sites") is stale → Phase 3 |
| 3 | `ir/tests.rs:903` | c — `is_some` |
| 4 | `ir/package.rs` `visit_project_targets_mut` | c — already walks a default's call targets (B relies on it) |
| 5–6 | `ir/variant_corpus_tests.rs` | c — already use a `Call` default |
| 7 | `ir/lower.rs` `lower_param` | **a** — lowers with the callee's `locals` → Phase 3 |
| 8 | `ir/lower.rs` `function_params` | **a** — carries the HIR default the caller re-lowers → `CallDefault` |
| 9 | `ir/lower.rs` `lower_local_call_arguments` | **a** — the bug-614 capture → Phase 3 |
| 10 | `target/shared/validate/body.rs` `validate_function` | **a** — validates the NIR default with parameters already in `locals` → empty map |
| 11 | `target/shared/nir/json.rs` | c |
| 12 | `target/shared/nir/lower.rs` `lower_param` | c — `lower_value` handles `Call` |
| 13 | `hir/mod.rs` `elaborate_link_function` | **a** (LINK) — keeps the AST expression, so nothing can lower it → elaborate to `HirExpression` |
| 14 | `hir/mod.rs` `elaborate_param` | c |
| 15 | `ir/shape.rs` `has_default` | c — arity only |
| 16 | `ir/shape.rs` `walk_function` | **a** — walks a default with `locals` → empty map |
| 17–20 | `cli/man.rs` | c — registry `DefaultValue`, not `IrParam` |
| 21 | `ir/binary.rs` `encode_param` | c — `put_opt_value` encodes a call |
| 22 | `ir/verify/mod.rs:394` | c — `TYPE_DEFAULT_ARG_ORDER` |
| 23 | `ir/verify/mod.rs:408` | **a** — `check_value`/`infer_type` with parameter `locals` → empty map; stale "caller's frame" comment → Phase 3 |
| 24 | `ir/verify/mod.rs:906` | c — optional count |
| 25 | `ir/verify/mod.rs:939` | c — closure collection |
| 26 | `manifest/entry.rs:95` | c — `is_some` on the entry's parameter |
| 27–36 | `codegen/registry/mod.rs` | c — registry `DefaultValue` (non-goal) |
| 37 | `ast/scope_privates.rs:177` | **a** — rewrites a default with the earlier parameters as locals → empty set |
| 38–39 | `codegen/resource/tests/mod.rs` | c — registry |
| 40–41 | `binary_repr/writer.rs` `lower_function` | b — `ConstPool::add` accepts only `Const`; runs only for `-br` dumps and `target::write_package` (`grep -rn 'write_binary_repr_hex\|write_package' src`) → plan-136-B |
| 42 | `ast/serialize.rs` | c |
| 43 | `resolver/resolution.rs:812` `resolve_function` | **a** → Phase 2 |
| 44 | `resolver/resolution.rs:1143` lambda arm | **a** → Phase 2 (rejected outright) |

Also found: `resolver::resolution::resolve_link_block` never resolves a LINK parameter default
(an unknown name in one goes unreported) → Phase 2 task added.

Acceptance: every RED case fails for the documented reason; every GREEN pin passes; the audit
table and the lambda-error measurement are recorded in this file.
  Check: `cargo build --release && cargo test --release --test rt_parameter_default_scope --
  --test-threads=1` → the six RED cases fail, three pins pass (est. 6 min: release build + ~9
  builds); `scripts/test-accept.sh target/release/mfb /tmp/p136a-p1-accept
  default-names-a-parameter-invalid lambda-parameter-default-invalid` → both mismatch against the
  placeholders (est. 1 min). (The second argument is `rm -rf`ed — always a fresh `/tmp` path.)
  **Measured 2026-09-13:** `test result: FAILED. 3 passed; 6 failed; … finished in 82.29s`;
  `acceptance tests failed: 4 mismatch(es) (2 test(s) ran)` (two `build.log` mismatches plus the
  lambda fixture's unexpected `.ast`/`.ir` actuals).
Commit: fbe4f5168

### Phase 2 — the front end sees only the declaration scope

- [x] `src/rules/table.rs`: add `SYMBOL_DEFAULT_NAMES_PARAMETER` and
      `SYMBOL_LAMBDA_PARAMETER_DEFAULT` (error) at the next free `2-201` codes — re-check with
      `grep -n '"2-201-00' src/rules/table.rs | tail -1` and `git log --all -S'2-201-0019'
      --oneline` immediately before choosing. **Done at `2-201-0020` / `2-201-0021`**: `0019` is
      retired and never reused (see Corrections — the table-tail check could not see it).
- [x] `src/docs/spec/diagnostics/01_rule-codes.md`: one row each in the `2-201` table; update the
      `2-201` population in "Subsystems (`SSS`)" (`18` → `20`).
- [x] `resolver::resolution::resolve_function`: resolve defaults with an empty local map; report
      `SYMBOL_DEFAULT_NAMES_PARAMETER` (located, `display_name`) when an unresolved name is a
      parameter of the function. New `Resolver.default_scope` + `resolve_parameter_default` /
      `report_default_names_parameter`, consulted by both `resolve_identifier` and
      `resolve_callable`; `default-names-a-parameter-invalid` → three located `2-201-0020` errors
      (lines 4, 8, 12), the earlier-, self- and later-parameter cases.
- [x] `resolver::resolution::resolve_expression` `HirExpression::Lambda`: report
      `SYMBOL_LAMBDA_PARAMETER_DEFAULT` for a lambda parameter with a default
      (`lambda-parameter-default-invalid` → `main.mfb:5 error[2-201-0021 SYMBOL_LAMBDA_PARAMETER_DEFAULT]`;
      the default is no longer resolved, so it cannot cascade).
- [x] `resolver::resolution::resolve_link_block`: resolve each LINK parameter default with an empty
      local map, reporting `SYMBOL_DEFAULT_NAMES_PARAMETER` the same way (added by the Phase 1 audit).
      Needed `hir::HirLinkParam.default` elaborated to `HirExpression` (`elaborate_link_function`),
      which is also what Phase 3's LINK fill lowers. Probe `/tmp/p136-probe-link-param`
      (`FUNC absval(n AS Integer = n)` in a `LINK "c"` block) → `main.mfb:4 error[2-201-0020
      SYMBOL_DEFAULT_NAMES_PARAMETER]: … The default value of `n` cannot use parameter `n`…`.
- [x] `ast::scope_privates`: rewrite defaults with an empty local set — for `FUNC`/`SUB` parameters,
      and (not rewritten at all before) for `LINK` parameters. Probe `/tmp/p136-probe-private-default`
      (`PRIVATE LET secret AS Integer = 5`, `FUNC f(secret AS Integer, y AS Integer = secret)`,
      `f(1)`) → builds, prints `5`: the default names the file's private, not the parameter.
- [x] `ir::verify` / `ir::shape::walk_function`: check defaults with an empty local map (per the
      Phase 1 audit). Both use a `no_locals` map for the default; verify's bug-297 comment now
      states the plan-136-A model. Evidence: the same probe builds through shape and verify with
      exit 0, and both syntax fixtures pass.
- [x] Generate the two syntax fixtures' goldens (`scripts/sync-goldens.sh target/release/mfb
      default-names-a-parameter-invalid lambda-parameter-default-invalid`) and read each
      `build.log`: one located error per offending parameter, no NIR line. `synced 2 golden
      file(s) across 2 test(s)`; `grep -ho 'error\[[^]]*\]'` over both → `2-201-0020` ×3,
      `2-201-0021` ×1; neither log contains `NIR`.

Acceptance: both syntax fixtures pass with located errors; rules census passes.
  Check: `cargo build --release && scripts/test-accept.sh target/release/mfb /tmp/p136a-p2-accept
  default-names-a-parameter-invalid lambda-parameter-default-invalid` → `acceptance tests passed (2
  test(s) ran)` (est. 5 min); `cargo test --release --bin mfb rules::` → `test result: ok`
  (est. 3 min).
  **Measured 2026-09-13 (after renumbering to `0020`/`0021`):** `cargo build --release --bin mfb` →
  `Finished`; `scripts/test-accept.sh target/release/mfb /tmp/p136a-p2-accept-2
  default-names-a-parameter-invalid lambda-parameter-default-invalid` → `acceptance tests passed (2
  test(s) ran)`; `cargo test --release --bin mfb rules::` → `test result: ok. 17 passed; 0 failed`.
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

- **Syntax-fixture placeholder set (Phase 1).** The plan said `golden/{<pkg>.ast,<pkg>.ir,build.log}`
  + `<pkg>.run`. Both fixtures are `-invalid` fixtures whose final state is a failed build, which
  writes no `.ast`/`.ir`; the sibling `tests/syntax/functions/user-function-default-args-invalid`
  has `golden/build.log` only. Placeholders are `golden/build.log` only, so the Phase 2 goldens
  match the fixture's real final shape.
- **P2 in the combined fixture (Phase 1).** Today `default-names-a-parameter-invalid` reports only
  the later-parameter case (`SYMBOL_UNKNOWN_IDENTIFIER` for `c`); that resolver error stops the build
  before the earlier/itself cases reach P2's unlocated NIR error. P2 stands as measured in
  §Verified properties (a program with only the earlier-parameter case).
- **The next free `2-201` code is not `0019` (Phase 2).** The plan's check (`grep -n '"2-201-00'
  src/rules/table.rs | tail -1` → `0018`) reads only the CURRENT table, so it cannot see a retired
  code. `git log main -G'"2-201-00(19|2[0-9]|13)"' --oneline -- src/rules/table.rs` → `35479878d
  plan-115-B Phase 3: delete IMPORT self from the compiler`, whose message says
  `IMPORT_SELF_IN_EXECUTABLE (2-201-0019) removed … The code is NOT reused`, and `1c61228ba`
  (bug-216) dropped `2-201-0013`. No main commit ever held `0020`/`0021`. The two rules are
  `SYMBOL_DEFAULT_NAMES_PARAMETER` **`2-201-0020`** and `SYMBOL_LAMBDA_PARAMETER_DEFAULT`
  **`2-201-0021`**; plan-136-C's next free code is therefore `2-201-0022` (recorded there). The
  strengthened check is the `git log main -G` query, not the table tail.
- **LINK defaults are never resolved (Phase 1 audit).** `resolve_link_block` resolves parameter
  types only; added a Phase 2 task so a LINK default gets the same declaration-scope resolution.

## Summary

The risk is the single call-site filler every omitted argument passes through, and the readers of
`IrParam.default` that assumed a value tree; literal defaults are held byte-identical to bound it.
Untouched: registry defaults, the `.mfp` format (B), and shadowing (C).
