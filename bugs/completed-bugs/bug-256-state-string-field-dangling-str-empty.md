# bug-256: a resource `STATE` record with a `String` field fails to link (`_mfb_str_empty` dangling)

Last updated: 2026-07-16
Effort: small (<1h)
Severity: HIGH
Class: Correctness (codegen — link failure)

Status: Fixed
Regression Test: `tests/rt-behavior/resources/bug256_state_string_field`

Giving a resource's `STATE` a `String` field made the program **impossible to
build**:

```
error: native code data relocation target '_mfb_str_empty' is not a data object
or defined symbol
```

Not a diagnostic, not a runtime error — a raw linker-stage failure naming an
internal symbol, from ordinary source with no `LINK` and no threads. Any
`STATE` carrying a `String` was unusable, which is a large hole for a feature
whose whole purpose is hanging user data on a resource.

The correct behavior a fix produces: **a `STATE` record's `String` field
default-initializes to the empty string like any other record's, and the program
builds and runs.**

References:

- `./mfb spec language resource-management` §15 — `STATE T` requires only that
  `T` be a copyable, defaultable data type. A record with a `String` field is
  both, so this was always meant to work.
- bug-05 / bug-45 / bug-67 — the same class: a demand for the `_mfb_str_empty`
  sentinel that the emit-decision analysis does not see.
- Found while establishing plan-52-A's row-5 fixture (a `Label{name AS String}`
  STATE). plan-52-C §2 attributed this error to "an unrelated build error on the
  mid-plan-50 tree" and moved on; it in fact reproduced on a green tree and was a
  live bug.

## Failing Reproduction

Minimal — one owner, one state type, no borrow, no mismatch:

```basic
IMPORT io
IMPORT fs

TYPE Label
  name AS String
END TYPE

FUNC main AS Integer
  RES f AS File STATE Label = fs::openFile("src/main.mfb")
  f.state.name = "hello"
  io::print(f.state.name)
  fs::close(f)
  RETURN 0
END FUNC
```

Before the fix: `mfb build` fails with the `_mfb_str_empty` relocation error.
After: prints `hello`.

## Root Cause

`_mfb_str_empty` is emitted into the module's data objects only when
`module_requires_empty_string_constant` says something demands it
(`src/target/shared/code/mod.rs`, gating `EMPTY_STRING_SYMBOL`). That predicate
walked ops via `op_requires_empty_string_constant`
(`src/target/shared/code/module_analysis.rs`), which recognised exactly one
demand:

```rust
NirOp::Bind { type_, value: None, .. } =>
    type_requires_empty_string_constant(type_, type_model, &mut HashSet::new()),
NirOp::Bind { value: Some(_), .. } => false,   // "supplies its own value"
```

The reasoning — *an initialized bind supplies its own value, so it needs no
default* — is true for the **bound value** and false for a resource's **STATE
payload**. Codegen default-initializes the STATE record at the bind itself:

```rust
// builder_control.rs — at every RES bind
if let Some(state_type) = crate::builtins::resource::state_type_name(type_) {
    self.emit_resource_state_init(stack_offset, &state_type)?;
}
```

A `RES` bind **always** carries a value (the handle it is binding), so it always
took the `value: Some(_) => false` arm. The `STATE` type was never inspected, its
`String` field's demand was never registered, the sentinel was never emitted, and
the relocation `emit_resource_state_init` had already generated dangled.

Nothing in-tree gave a `STATE` a `String` field — every existing fixture uses
all-`Integer` state records — so no test exercised the combination.

## The Fix

`src/target/shared/code/module_analysis.rs`: check the binding's STATE type on
**every** `Bind`, regardless of whether it carries a value.

```rust
NirOp::Bind { type_, value, .. } => {
    crate::builtins::resource::state_type_name(type_).is_some_and(|state| {
        type_requires_empty_string_constant(state, type_model, &mut HashSet::new())
    }) || (value.is_none()
        && type_requires_empty_string_constant(type_, type_model, &mut HashSet::new()))
}
```

The `value.is_none()` clause is the original rule, unchanged. The new clause
mirrors `builder_control.rs`'s condition exactly — `state_type_name(type_)` is
the same predicate that decides whether the init is emitted at all — so the
analysis and the emitter now agree by construction rather than by coincidence.

## Notes

The recurring shape (bug-05, bug-45, bug-67, this): a *demand* analysis and the
*emitter* that creates the demand are written in different files and drift.
`module_analysis.rs` already enumerates `NirOp` exhaustively (no `_ => false`)
specifically to catch a new body-bearing variant; that guard cannot catch this,
because the variant was already handled — just incompletely.
