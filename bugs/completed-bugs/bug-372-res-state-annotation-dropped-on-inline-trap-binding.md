# bug-372: an explicit `AS T STATE S` on a `RES` binding is dropped when the initializer carries an inline `TRAP`, so the annotated form fails to compile where the inferred form succeeds

Last updated: 2026-07-20
Effort: small (30m–1h) — one omission in `lower_statement`, plus a stripped-vs-unstripped type comparison in verify
Severity: **MEDIUM** (compile-time rejection of valid code; the annotation is strictly harmful, which inverts the usual incentive)
Class: Correctness (IR lowering — type annotation lost on one path)

Status: **FIXED** 2026-07-20.
Regression Test: `tests/syntax/native/native-res-state-inline-trap-valid` (new)
— the annotated form, the bare form, and the no-trap reference form, all
asserted to compile. Proven non-vacuous by disabling the `lower.rs` half and
observing `TYPE_STATE_INVALID` on the annotated form.
`tests/rt-behavior/native/native-link-inline-trap-rt` proves both paths of the
annotated form at runtime.

## Correction: there was a THIRD site

Both sites this report identifies are real and were fixed as written. They were
not sufficient. With the front end fixed, the combination reached native codegen
and failed there:

```
error: native code cannot materialize default value for type 'Db STATE DbInfo'
       while lowering bind $trap_val1 AS Db STATE DbInfo
```

`lower_default_value` recognizes only **built-in** resources
(`builtins::is_resource_type`), so it had no default for a user-declared
`RESOURCE`. This is broader than this report: `RES x AS <user resource> =
<fallible> TRAP` had **never** built, with or without `STATE` — the front end
rejected it first, so the codegen gap was never reached.

Fixed by giving `TypeModel` a `resource_names` set (populated from the module's
`"resource"` type kind, from `link_functions` that return a resource — an
executable declaring its own `RESOURCE` carries it nowhere else — and from an
imported package's native resource exports), and by default-initializing the
`STATE` payload of the closed-resource default so a `RECOVER`ed value's `.state`
is a real record rather than null.

Writing the resource type annotation **causes** a compile error that omitting it
avoids:

```basic
RES h AS SoundFile STATE FileInfo = sndLink::openFile(p) TRAP   ' rejected
  FAIL …
END TRAP

RES h = sndLink::openFile(p) TRAP                                ' accepted
  FAIL …
END TRAP
```

```
error[2-203-0129 TYPE_STATE_MISMATCH]: a resource's STATE type is fixed at its
owning binding and every other declaration of it must agree
    binding `h` is bare but its initializer carries STATE `FileInfo`;
    a bare binding asserts the resource has no state — declare `STATE FileInfo`.
```

The diagnostic instructs the author to write exactly what they already wrote.

## Root Cause

`src/ir/lower.rs`, the `Statement::Let` arm. The inline-TRAP branch destructures
`state_type` and then returns without ever reading it:

```rust
if let Some(Expression::Trapped { expression, binding, handler, .. }) = value {
    let success_type = type_name
        .clone()
        .or_else(|| expression_type(expression, locals, context))
        .unwrap_or_else(|| "Unknown".to_string());
    return lower_inline_trap(
        expression, binding, handler,
        InlineTrapTarget::Bind {
            mutable: *mutable,
            name: name.clone(),
            type_: success_type,      // <-- no `STATE {state}` suffix
            explicit_type: type_name.is_some(),
        },
        locals, context,
    );
}
```

The non-trap path immediately below is where the suffix is applied:

```rust
// A `RES` binding's `STATE T` rides in the lowered type string
// (`File STATE T`) so codegen can default-initialize and address the
// state payload; the bare resource name is recovered for recognition.
let lowered_type = match state_type {
    Some(state) => format!("{lowered_type} STATE {state}"),
    None => lowered_type,
};
```

`InlineTrapTarget::Bind` has no state field at all.

**Why omitting `AS` works:** with no `type_name`, `success_type` falls through to
`expression_type(expression, …)`, which *does* return `"SoundFile STATE
FileInfo"`. Only the explicitly-annotated form loses the state — hence the
inverted incentive.

Downstream, `check_binding_state_agreement`
(`src/ir/verify/mod.rs`) infers `actual = "SoundFile STATE FileInfo"` from the
`$trap_val` slot, sees `state_type_name(type_) == None` on the `Bind`, and emits
the message above.

## Second, related site

`src/ir/verify/mod.rs:987` compares a STATE-stripped expected type against a
non-stripped actual:

```rust
let expected = resource_base_type(expected);          // strips ` STATE …`
if … && !self.expression_compatible(expected, &actual, value) {
    self.emit("TYPE_RECOVER_TYPE_MISMATCH",
        format!("RECOVER has type {actual}, expected {expected}."));
}
```

With the lowering above corrected, this fires on the Ok-path assign into
`$trap_val` — which is not a user `RECOVER` at all — as:

```
error: TYPE_RECOVER_TYPE_MISMATCH: RECOVER has type SoundFile STATE FileInfo, expected SoundFile.
```

Both sides need `resource_base_type` applied, or neither. The message is also
misleading on this path, since no `RECOVER` is involved.

## Suggested Fix

Reattach the suffix in the inline-TRAP branch, mirroring the non-trap path.
`expression_type` already carries the state, so only an explicit `AS T` needs it:

```rust
let success_type = match (type_name, state_type) {
    (Some(type_name), Some(state)) => format!("{type_name} STATE {state}"),
    _ => success_type,
};
```

This was verified to clear `TYPE_STATE_MISMATCH`; it then exposes the
`TYPE_RECOVER_TYPE_MISMATCH` site above, which must be fixed in the same change
for the combination to compile. **Neither fix was landed** — the patch was
reverted, because it was made to unblock a binding change rather than as a
considered compiler change, and the runtime behavior behind it is bug-371.

Fix bug-371 first, or at least together: making this combination *compile* while
its error path is still broken would hand authors a shape that silently bypasses
`ERROR_ON`.

## Impact

No code in the tree hits this today — the combination `RES` + explicit `STATE` +
inline `TRAP` currently has zero occurrences, which is why it has gone unnoticed.
It is the natural shape for a caller wanting to handle a stateful native
resource's open failure at the call site, so the first author to reach for it is
told to add an annotation that is already present.
