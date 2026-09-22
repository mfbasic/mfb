# bug-671: `typeName(<user FUNC call>)` fails to compile

Last updated: 2026-09-21
Effort: small
Severity: MEDIUM (a build error on valid source; no miscompile)
Class: Codegen / typing

Status: **FIXED** (2026-09-21) — landed as a small-ish fix (write-bug), no phased plan.
Regression Test: `tests/rt-behavior/general/typename_user_call`

**The correct behavior:** `typeName(f(…))` for a user `FUNC f` builds and prints
`f`'s declared return type, without evaluating the call (`mfb spec language
builtin-functions`: `typeName` "never reads the value").

## Failing reproduction

```
IMPORT io

FUNC pick(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main() AS Integer
  io::print(typeName(pick(1)))
  RETURN 0
END FUNC
```

- Observed (main at `671fd9b10`, macOS aarch64): `error: native code cannot determine
  typeName argument type while lowering eval call io.print`, exit 1.
- Expected: `Integer`, exit 0.

Found by the bug-667 producer audit (probing `MUT t = typeName(pick(1))`).

## Root cause

`typeName` is folded in codegen from `static_type_name_for_fold`
(`src/codegen/memory/value/builder_value_semantics.rs`). Its `Call` arm resolved a
call's type only through `builtins::resolve_call_return_type_typed` (bug-354 added
that for builtin calls), which knows nothing of module functions, so any user
`FUNC` call answered `None` and the three `typeName` lowerings in
`src/codegen/engine/value/builder_values.rs` raised the build error.

## Fix

The `Call` arm now answers a module function's declared return type
(`self.functions[target].returns`), and a callable local of the same name first —
a `FUNC`-typed parameter shadows a top-level function at the call, as bug-569
established for `callable_value_return_type`.

## Blast radius

- The three `typeName` fold sites share `static_type_name_for_fold`: fixed together.
- `static_type_name_for_fold`'s other callers see a type where they saw `None`
  before only for a user-function call, which previously failed the build; no
  program that built changes. Artifact gate: 0 diffs.

## Validation

- RED: the fixture against the pre-fix compiler → the build error (build.log mismatch).
- GREEN: `scripts/test-accept.sh target/release/mfb target/accept-actual
  typename_user_call func_typename_builtin_calls` → passed.
- `scripts/artifact-gate.sh target/release/mfb all` → `1478 tests, 1653 build(s),
  2088 golden(s) checked, 0 diff(s)`.
- `scripts/test-accept.sh target/release/mfb target/accept-actual` →
  `acceptance tests passed (1504 test(s) ran)`.
- `cargo test --bin mfb` → `test result: ok. 4286 passed; 0 failed; 1 ignored`.
- `cargo fmt --all -- --check` (both workspaces) → exit 0.
