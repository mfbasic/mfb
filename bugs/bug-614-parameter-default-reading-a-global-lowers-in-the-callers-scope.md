# bug-614: a parameter default that reads a global is captured by a caller's local of the same name, and cannot be packaged at all

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (silent wrong value)

Status: Open
Regression Test: none yet — see Phase 1

A parameter default that names a global, such as `FUNC f(x AS Integer = limit)` with
top-level `LET limit AS Integer = 5`, is evaluated **in the caller's scope**. If the
caller has a local named `limit`, `f()` receives the caller's local, not the global.
The build succeeds, the program exits 0, and the value is wrong. There is no
diagnostic.

The same default in an **exported package function** does not build for an importer:
`error: only constant IR values can be stored in CONST_POOL`. The error is internal
and has no location.

**The single correct behavior a fix produces:** a default expression resolves its
names in the scope where the function is declared. `f()` passes the global `limit`
(`5`) whatever locals its caller has. A package function's default that reads a
package global either builds and passes that global, or is refused at the package
build with a located, user-facing diagnostic. Which of the two is an Open Decision.

References:

- `mfb spec language functions` §6 ("Default args allowed (trailing)"). The spec
  does not say which scope a default expression resolves names in. Lexical
  (declaration) scope is the only reading under which a function's own signature
  means one thing.
- Found during the bug-612 Blast Radius audit (parameter-defaults row), 2026-09-13.
- Memory note `default-value-is-a-call-site-constant` (registry builtins' defaults
  are call-site constants). That note covers builtins, not user `FUNC`s.

## Failing Reproduction

Measured 2026-09-13, macOS aarch64, release binary built from the bug-612/613
worktree at `53f10b1fe`. Neither path below is touched by that commit.

`project.json`: a plain executable (`"kind":"executable"`, `"entry":"main"`).
`src/main.mfb`:

    IMPORT io

    LET limit AS Integer = 5

    FUNC f(x AS Integer = limit) AS Integer
      RETURN x
    END FUNC

    FUNC main() AS Integer
      LET limit AS Integer = 99
      io::print(toString(f()))
      RETURN 0
    END FUNC

```
mfb build . && ./build/defapp.out
```

- Observed: prints `99`, exit 0.
- Expected: prints `5`.

Package form: package `deflib` with `LET Limit AS Integer = 5` and
`EXPORT FUNC f(x AS Integer = Limit) AS Integer … RETURN x`. The importer calls
`io::print(toString(deflib::f()))`.

- Observed: `mfb build app` → `error: only constant IR values can be stored in
  CONST_POOL`, no executable.
- Expected: prints `5` (or a located diagnostic at the package build; see Open
  Decisions).

| Case | Result |
| --- | --- |
| own global default, caller has no local `limit` | works ✓ prints `5` |
| own global default, caller has `LET limit = 99` | wrong ✗ prints `99` |
| exported package function, default reads a package global | build fails ✗ internal CONST_POOL error |
| literal default (`x AS Integer = 5`) | works ✓ (the common case) |

**Added 2026-09-13 while planning the fix** (release binary, macOS aarch64; programs in
`planning/plan-136-A-default-declaration-scope.md` §Verified properties):

| Case | Result |
| --- | --- |
| default calls a function (`= helper()`), caller has `LET helper = LAMBDA() -> 99` | wrong ✗ prints `99` |
| default names an earlier parameter (`b AS Integer = a`) | build fails ✗ unlocated `NIR local reference 'a' does not resolve` |
| importer omits a **literal** package default (`deflib::f()`, `f(x AS Integer = 5)`) | wrong ✗ prints `4374773792` |
| importer omits a literal `String` package default (`deflib::g(1)`) | crash ✗ exit 139 |
| omitted `LINK` function default (`absval()`, `n AS Integer = -5`) | runtime error ✗ `7-705-0010` |
| lambda parameter default (`LAMBDA(x AS Integer = 4)`) | silently ignored ✗ |

An importer never fills an omitted package default at all: `ir::lower::lower_facts` builds external
call parameters with `default: None` from a `has_default: bool` that carries no value.

## Root Cause

Two mechanisms:

1. **Caller-scope lowering.** `src/ir/lower.rs:lower_local_call_arguments` fills an
   omitted argument by lowering the callee's HIR `param.default` with the **caller's**
   `locals`:

   ```rust
   None => params.get(index).and_then(|param| {
       param.default.as_ref().map(|default| {
           lower_expression_with_expected(default, Some(&param.type_), locals, context)
       })
   }),
   ```

   The `Identifier` arm checks `locals.contains_key(value)` before its
   `binding_types` arms (the same precedence that caused bug-612). A caller local
   with the global's name wins, so the default lowers as `IrValue::Local`, the
   caller's variable. With no such local it falls through to `Global`, which is
   why the contrast case works. The callee-side `lower_param` uses the function's
   own scope (its parameters) and is correct, but the call site does not use it.

2. **Constant-only package defaults.** `src/binary_repr/writer.rs` stores each
   parameter default as `default_const: constants.add(strings, default)?`, and
   `src/binary_repr/sections.rs` (`ConstPool::add`) accepts only `IrValue::Const`.
   Anything else returns `only constant IR values can be stored in CONST_POOL`. The
   `.mfp` format has no slot for a non-constant default, so the failure surfaces as
   an internal error while the dependency is built.

## Goal

- The reproduction prints `5`.
- A default that reads a global resolves that name at the declaration, never through
  a caller's local, for every call form that fills an omitted argument (positional
  omission and named arguments).
- The package form either works or is refused at the package build with a located
  diagnostic. Never the internal CONST_POOL error.

### Non-goals (must NOT change)

- Literal defaults and their call-site coercion to the parameter type
  (`a AS List OF Fixed = [1, 2]`).
- Registry/builtin `DefaultValue`s. They are call-site constants by design.
- The `.mfp` format, unless Open Decision 1 chooses to extend it (a format change
  needs its own plan).
- **Tempting wrong fix (forbidden):** renaming or refusing the caller's local, or
  making the `Identifier` arm prefer globals over locals everywhere. That breaks
  ordinary shadowing inside function bodies.

## Blast Radius

- `src/ir/lower.rs:lower_local_call_arguments`: the capture. Fixed by this bug.
- Other call-lowering paths that fill omitted defaults (named-argument
  normalization `normalize_local_call_arguments`, method/UFCS or lambda call paths):
  to audit in Phase 1.
- `src/binary_repr/writer.rs` parameter `default_const`: the package form. Fixed or
  diagnosed per Open Decision 1.
- `src/ir/package.rs`: an imported function's default is rewritten by
  `apply_package_identity`. Relevant if defaults become non-constant across the
  package boundary.

## Fix Design

Lower the default at the call site with an **empty** local scope (a default has no
access to the caller's locals), mirroring the bug-612 fix for top-level
initializers. The `Identifier` arm then resolves a global name to `Global`. Keep the
expected-type coercion. Risk: a default that names an **earlier parameter** of the
same function, if the language allows that at all. Phase 1 must check. If allowed,
the scope is "the callee's preceding parameters bound to the caller's argument
values", not empty.

For the package form, the smallest correct change is a located diagnostic at the
package build (a non-constant default in an `EXPORT`ed function). Carrying
non-constant defaults through the `.mfp` would need a format change.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Runtime test: the reproduction asserts `5`. Confirm RED (prints `99`).
- [ ] Runtime test: the named-argument form (`f(y := 1)` leaving `x` to its default)
      with the same shadowing local.
- [ ] Package-form test asserting the Open Decision 1 outcome. Confirm RED (internal
      CONST_POOL error).
- [ ] Audit every call path that fills an omitted default. Check whether a default
      may reference an earlier parameter.

Acceptance: tests RED for the documented reason; audit verdicts recorded.
Commit: —

### Phase 2 — the fix

- [ ] `lower_local_call_arguments` (and audited siblings): lower defaults with the
      declaration's scope.
- [ ] Package form per Open Decision 1.

Acceptance: Phase 1 tests pass; literal-default coercion unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/test-accept.sh`; any drifted golden inspected. None expected: no
      passing fixture can depend on a captured caller local and still print the
      right value.
- [ ] `cargo test --release --no-fail-fast`.

Acceptance: full suite green; golden delta empty or justified.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 runtime tests.
- Runtime proof: the reproduction prints `5`.
- Doc sync: state in `mfb spec language functions` that a default expression
  resolves names at the declaration.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

1. ~~Package form: extend the `.mfp` to carry non-constant defaults, vs. refuse a
   non-constant default in an `EXPORT`ed function with a located diagnostic.
   Recommend the diagnostic now and a plan later if the form is wanted.~~

**DECIDED by the owner, 2026-09-13.** The fix is **plan-136** (A–C), which closes this bug:

- A default's names resolve **where the function is declared**, never through a caller's scope,
  and it is evaluated **on each call**.
- A default is external: it may **not** name any parameter of its function (a located error).
- **Packages and executables behave identically**, so the `.mfp` carries non-constant defaults
  (not a diagnostic).
- A local binding may **not** share a name with a visible top-level `LET`/`MUT` (a located error),
  so this bug's own program becomes a compile error; the function-call shape above stays a runtime
  regression test.

The Fix Design and Phases sections above predate the decision; plan-136 supersedes them.

## Summary

A silent wrong value from the same `locals`-before-globals precedence as bug-612,
reached through call-site default filling, plus an internal error on the package
path. The call-site fix is small. The risk is the audit of every default-filling
call path and whether defaults may name earlier parameters.
