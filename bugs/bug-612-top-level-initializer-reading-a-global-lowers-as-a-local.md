# bug-612: a top-level `LET`/`MUT` whose initializer reads a global fails to build — `NIR local reference '<name>' does not resolve`

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet — see Phase 1

A top-level `LET` or `MUT` whose initializer reads **another global** does not build.
That includes an imported package's `EXPORT LET`/`EXPORT MUT` (`test::SomeConst`)
and the file's own top-level binding (`LET top = own`). The build exits 1 with an
internal error that has no location:

    error: NIR local reference 'test.SomeConst' does not resolve

The same read inside a `FUNC`/`SUB` builds and runs. A package is worse off: a
package whose own top-level `LET B = A` **builds a `.mfp` with exit 0**, and every
importer then fails, even one that reads `test2::B` only inside a function.

**The single correct behavior a fix produces:** there is no error. A top-level
`LET`/`MUT` initializer that reads a global (the program's own, or an imported
package's export) builds and the binding holds that global's value at run time. In
the reproductions below, `top` prints `100`, `200` and `7`, and `test2::B` prints `1`.

References:

- `mfb spec language` §13 (modules and packages): an exported top-level `LET`/`MUT`
  is importer-visible, and "variables and constants" take the `pkg::` prefix.
- bug-551 (`bugs/completed/bug-551-exported-package-constant-does-not-resolve-for-an-importer.md`,
  `9a5aacb6e`) made `pkg::Name` typed and usable. Its tests
  (`tests/runtime/rt_imported_package_global.rs`) only read the global inside a function.
- bug-613: found in the same session, different root cause (initialization order).

## Failing Reproduction

Measured 2026-09-13 with the release binary at `bfb0cfbfc` (`9a5aacb6e` is an ancestor).

`pkg/project.json`

    {"name":"test","version":"0.1.0","mfb":"1.0","kind":"package","description":"c",
     "sources":[{"root":"src","role":"package","include":["**/*.mfb"]}]}

`pkg/src/lib.mfb`

    EXPORT LET SomeConst AS Integer = 100
    EXPORT MUT SomeVar AS Integer = 200
    EXPORT FUNC getVar() AS Integer
      RETURN SomeVar
    END FUNC

`app/project.json`

    {"name":"capp","version":"0.1.0","mfb":"1.0","kind":"executable",
     "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
     "packages":[{"name":"test","version":"=0.1.0","source":"file:packages/test.mfp"}],
     "entry":"main","targets":["native"]}

`app/src/main.mfb`

    IMPORT test
    IMPORT io

    LET top = test::SomeConst

    FUNC main() AS Integer
      io::print(toString(top))
      RETURN 0
    END FUNC

```
mfb build pkg && cp pkg/test.mfp app/packages/ && mfb build app; echo $?
```

- Observed: `error: NIR local reference 'test.SomeConst' does not resolve`, exit 1.
- Expected: builds; `app/build/capp.out` prints `100`.

Matrix of `main.mfb` variants (macOS aarch64; each is a separate build):

| Initializer / use | Result |
| --- | --- |
| top-level `LET top = test::SomeConst` | fails ✗ |
| top-level `LET top AS Integer = test::SomeConst` | fails ✗ |
| top-level `LET top = test::SomeVar` | fails ✗ |
| top-level `MUT top AS Integer = test::SomeVar` | fails ✗ |
| top-level `MUT top AS Integer = test::SomeConst` | fails ✗ |
| no import: `LET own AS Integer = 7` / `LET top = own` | fails ✗ (`NIR local reference 'own'`) |
| package `EXPORT LET A AS Integer = 1` / `EXPORT LET B AS Integer = A` | package build exit 0 ✗; importer reading `test2::B` inside `main` fails with `NIR local reference 'A'` |
| in a function: `LET a = test::SomeConst` | works ✓ prints `100` |
| in a function: `test::SomeVar = 5`, read back directly and via `test::getVar()` | works ✓ prints `5`, `5` |
| top-level `LET top = getOwn()`, where `getOwn` returns own `MUT own = 7` | works ✓ prints `7` |
| top-level `LET top = test::getVar()` | builds but prints `0`, not `200`: **bug-613** |

## Root Cause

`src/ir/lower.rs:lower_binding` uses the whole global type table as its local scope:

```rust
let locals = context.binding_types.clone();
...
value: binding.value.as_ref()
    .map(|value| lower_expression_with_expected(value, Some(&type_), &locals, context)),
```

`binding_types` holds every top-level binding and, since bug-551, every imported
export under its `package.Name` spelling (`lower_facts`). In
`lower_expression_with_expected`, the `HirExpression::Identifier` arm checks
`locals.contains_key(value)` **first**, so any global name in a top-level initializer
lowers as `IrValue::Local(name)`. The `binding_types` → `IrValue::Global` arms after it
never fire. The IR shows this:

    { "name": "top", ..., "value": { "kind": "local", "name": "test.SomeConst" } }

while the same read inside `main` is `{ "kind": "global", "name": "test.SomeVar" }`.

After that:

- `src/ir/package.rs:rewrite_value_targets` qualifies only `IrValue::Global` (and
  calls / function refs) to `<id>.package.Name`, so the `Local` keeps its unmerged
  name.
- `src/target/shared/nir/lower.rs:lower_global_initializer` emits
  `storeGlobal top = Local("test.SomeConst")`.
- `src/target/shared/validate/body.rs:validate_value` finds no local by that name and
  rejects it, with no source location.

A package build does not reach that validation for its own `.mfp` (the `B = A`
package builds with exit 0), so the bad `Local("A")` ships and fails in every importer.

Why the contrast cases are immune: inside a function, `locals` is the function's
real local scope, so a global falls through to the `Global` arm. A call initializer
(`getOwn()`) lowers through the `Call` arm, which never consults `locals`.

`let locals = context.binding_types.clone();` dates to `0854a0a1d` (2026-06-16, then
in `src/ir.rs`). Before bug-551, an imported global had no type at all, so this form
never reached lowering for packages. The own-global form (`LET top = own`) has been
broken since then.

## Goal

- Every failing row of the matrix above builds, runs and prints the expected value
  (`100`, `200`, `7`, `1`), with no error.
- The IR for a top-level initializer that reads a global carries
  `{ "kind": "global" }`, qualified to `<id>.package.Name` after the package merge.

### Non-goals (must NOT change)

- Function-body lowering: `locals` there is a real scope and must keep shadowing
  globals.
- bug-551's surface: typed reads, `EXPORT MUT` writes shared with the package,
  `TYPE_ASSIGN_REQUIRES_MUT` on an `EXPORT LET` write, `PRIVATE` and unexported names
  still refused.
- The `.mfp` GLOBAL table format.
- **Tempting wrong fixes (forbidden):** teaching `validate_value` to accept a dangling
  `Local` that names a global, or mapping `Local` → global in NIR lowering. Both hide
  the wrong IR kind, and the package rewrite still would not qualify the name.
  Refusing the form with a diagnostic is also not a fix; the user's requirement is
  that it works.
- Initialization ORDER (a top-level read of a global that is initialized later) is
  bug-613 and is not fixed here.

## Blast Radius

Found by reading `lower.rs`. The Phase 1 audit confirms each one:

- `src/ir/lower.rs:lower_binding`: the `locals` passed to
  `lower_expression_with_expected` — fixed by this bug.
- `src/ir/lower.rs:lower_binding`: the same `locals` passed to `expression_type`
  for an untyped binding. Typing works today (`LET top = test::SomeConst` fails at
  NIR, not with `TYPE_UNKNOWN_VALUE`), so the type side is unaffected, but the audit
  confirms it once `locals` changes.
- `src/ir/lower.rs:infer_binding_types`: passes an empty `locals` — unaffected.
- Parameter default values (`param.default`, rewritten by `apply_package_identity`)
  that read a global — **unaudited**; check whether their lowering scope has the same
  shape.
- Package build: a `.mfp` whose initializer carries a dangling `Local` builds with
  exit 0 — latent. A package build that ran the same NIR validation would have caught
  it. Out of scope beyond noting it; the fix removes this instance.
- Thread workers re-run the declaring project's initializer (spec §on
  `thread::start`) — they use the same initializer function, so fixed by the same change.

## Fix Design

In `lower_binding`, lower the initializer with an **empty** local scope (a top-level
initializer has no locals), matching `infer_binding_types`. The `Identifier` arm then
reaches the existing `binding_types` checks: `value` → `Global(value)`, and an
imported name → `Global(canonical_value)` (bug-551's arm). That arm already handles
`IMPORT … AS` aliasing.

Risk: any top-level initializer shape that depended on a global resolving as `Local`
(for example a lambda/closure capture or a TRAP inside an initializer). Phase 1's
audit must grep the `.ir` goldens for top-level `"kind": "local"` values to find them.
Golden drift is expected only where a top-level initializer read a global. Today such
a program could not build, so no passing golden should change. Any golden that does
change is a bug-hunt trigger.

Rejected: the validator / NIR-lowering workarounds listed under Non-goals.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add tests to `tests/runtime/rt_imported_package_global.rs` (or a sibling
      runtime test) for every failing row of the matrix. Each builds and asserts the
      printed value. Confirm each is RED with the documented error.
- [ ] Add a package-internal case: a package with `EXPORT LET B = A`, imported and
      read inside a function, prints `1`.
- [ ] Audit each Blast Radius row, including parameter defaults, and grep the
      `.ir` goldens for top-level bindings with `"kind": "local"` values; record verdicts here.

Acceptance: new tests fail for the documented reason; every audit row has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] `src/ir/lower.rs:lower_binding`: lower the initializer (and its
      `expression_type`) with an empty local scope.
- [ ] Apply the same change to any in-scope sibling from the audit.

Acceptance: Phase 1 tests pass; function-body contrast cases unchanged; bug-551's
ten tests still pass.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/test-accept.sh`; any drifted golden inspected and justified one by one.
- [ ] `cargo test --release --no-fail-fast`.
- [ ] Re-run the reproduction end to end on macOS and the Linux boxes.

Acceptance: full suite green; golden delta is exactly the intended change.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 runtime tests.
- Runtime proof: the reproduction's `capp.out` prints `100`; the own-global and
  package-internal variants print `7` and `1`.
- Doc sync: none expected. §13 already describes the behavior.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Whether a package build should run the NIR validation the importer runs, so a
  broken `.mfp` can never ship with exit 0. Recommend filing separately if the audit
  finds another way to produce one.

## Summary

The fix itself is one scope change in `lower_binding`. The risk is in the audit: any
top-level initializer shape that relied on `locals` being the global table. Typing,
the `.mfp` format and function-body lowering stay untouched.
