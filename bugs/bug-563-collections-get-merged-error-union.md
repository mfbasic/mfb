# bug-563: `collections::get`'s two overloads declare a merged error union, and the map form carries the list form's parameter prose

Last updated: 2026-09-06
Effort: small-to-medium (the audit is the work, as it was for bug-553)
Severity: MEDIUM (documentation correctness; same class as bug-553)
Class: Registry metadata / documentation correctness

Status: Open
Regression Test: — (a `src/cli/man.rs` rendering test, plus the per-overload error assertion)

## How it was found

bug-558 made `mfb man` name which numbered overload raises each error. That
immediately surfaced this: `collections::get` renders `1, 2` on **both**
`ErrIndexOutOfRange` and `ErrNotFound`, i.e. it claims the list form can raise
`ErrNotFound` and the map form can raise `ErrIndexOutOfRange`.

Surfacing it is the point of that change — the union hid it.

## The finding

Both implementations declare the identical set
(`src/codegen/builtins/collections/func_get.rs:115,136`):

```rust
errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],   // list overload
errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],   // map overload
```

But the lowering **branches by shape**: `typed_list_element_type(...)` routes to
`builder.lower_list_get(...)`, and `typed_map_type_parts(...)` routes to the map
path. Two different paths, one declared set.

The natural reading — a list index that is out of range raises
`ErrIndexOutOfRange`, a map key that is absent raises `ErrNotFound` — is exactly
what the union destroys.

**Second defect, same file.** The map overload's key parameter carries the LIST
overload's description verbatim:

> "The list index, zero-based. Out of range raises — use `collections::getOr` to
> supply a fallback instead."

That is wrong prose for a map key, which is not an index, is not zero-based, and
is not "out of range".

## What a fix must produce

Each overload declares the errors **its own lowering** can raise, derived from
the lowering rather than from the prose — the method bug-553 used for the 34
`tcp`/`tls` implementations.

Two properties, and the second is the one an audit gets wrong:

- Every error a shape's lowering can raise appears in that shape's list.
- **No shape declares an error its lowering cannot raise.** A wrong list is worse
  than a merged one: it states a member raises something it cannot, and for any
  member that ever moves to an inline-lowerable body it feeds
  `inline_builtin_is_infallible` bad data — the mechanism behind three
  dead-handler MISCOMPILES.

And the map key gets its own description.

## Blast radius — do not assume it is only `get`

`collections::get` was found because bug-558's column made it visible on one
page. **The same merge is invisible anywhere the overloads happen to agree**, so
census the package rather than fixing the one member: any multi-overload member
whose implementations share an `errors:` vector by copy-paste rather than by
derivation. `getOr`, `set`, `removeAt`/`removeKey`, `hasKey` and `keys`/`values`
are the obvious neighbours.

## Non-goals

- Do not change the renderer. bug-558's column is correct; it is reporting what
  the descriptors say.
- Do not make the two overloads agree in order to make the union honest.

## References

- `src/codegen/builtins/collections/func_get.rs:115,136` (the declarations),
  `:169-190` (the shape branch)
- `bugs/completed/bug-553-*` — the derive-from-lowering method, and the two traps
  it names (a raise gated on a Rust `if` over a lowering parameter; a `bl` by
  symbol that no Rust call site shows)
- `bugs/completed/bug-558-*` — the change that made this visible
