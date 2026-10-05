# bug-874: `typeName(Enum.Member)` with a literal enum member fails the build with an internal codegen error

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/general/typename-enum-member-literal-rt (new) — name suggested

`typeName` of an enum member written directly as the argument (`typeName(Color.Green)`,
also `typeName((Color.Red))`) aborts the build with the unstructured message
`error: native code cannot determine typeName argument type while lowering ...`. The same
value through a binding (`LET c = Color.Red` / `typeName(c)`) or a call returning the enum
(`typeName(f())`) works and prints `Color`. A valid program is refused with an internal error
and no rule code.

Correct behavior: `typeName(Color.Green)` compiles and returns `"Color"`, exactly like
`typeName(c)` for `c AS Color`.

References:

- src/docs/spec/language/18_builtin-functions.md §18.1: "`typeName(value)` | 1 | `String` | any `T` (never reads the value)".
- Distinct from bug-835 (typeName under an inline TRAP); this needs no TRAP.
- Found during B05 hardening of language/18_builtin-functions.

## Failing Reproduction

```
IMPORT io
ENUM Color
  Red, Green
END ENUM
FUNC main() AS Nothing
  io::print(typeName(Color.Green))
END FUNC
```

`mfb build <proj>`

- Observed: `error: native code cannot determine typeName argument type while lowering eval call io.print` (build fails; with `LET s = typeName(Color.Green)` the text ends `... while lowering bind s AS String`).
- Expected: builds; prints `Color`.

Contrast (all work, measured): `typeName(c)` for `LET c = Color.Red` → `Color`; `typeName(f())` with
`f() AS Color` → `Color`; `typeName(P[x := 1])` → `P`; `typeName(NOTHING)`, `typeName(1.5M)`,
`typeName(error(1,"x"))`, `typeName(LAMBDA(a AS Integer) -> a)` all fold.

## Root Cause

The IR carries the argument as `{ "kind": "memberAccess", "type": "Color", "target": { "kind": "local", "name": "Color" }, "member": "Green" }`
(`mfb build --ir`). `typeName` folds through
`src/codegen/memory/value/builder_value_semantics.rs:static_type_name_for_fold` →
`static_type_name`, which resolves a member access through its *target local*; `Color` is a
type name, not a local, so the lookup yields `None` and the call site
(`src/codegen/engine/value/builder_values.rs`, the `target == "typeName"` arms) turns that into
the error string. The node's own `type: "Color"` is never consulted.

## Goal

- `typeName(<Enum>.<Member>)` folds to the enum's name in every position (argument, binding, parenthesized).

### Non-goals (must NOT change)

- `typeName` must still never evaluate its argument.

## Blast Radius

- The other two `typeName` arms in `builder_values.rs` (`lower_infallible_member`, line ~3660) share the same fold and are fixed together.
- Any other consumer of `static_type_name` on an enum-member literal (overload/code-form selection in `lower_runtime_helper_call`) — latent, not observed.
