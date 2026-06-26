# 5. Bindings & Scope

Three binding forms on two axes — `LET`/`MUT` choose **mutability**, `RES`
chooses **ownership**:

- **`LET`** — immutable binding (copyable data).
- **`MUT`** — reassignable binding (copyable data).
- **`RES`** — a uniquely-owned resource (a `File`, `Socket`, `Listener`, …).
  A resource has no aliases, so mutability is moot and `RES` needs no
  immutable/mutable sub-distinction. See §15.

```basic
LET x = 10
MUT total AS Float = 0.0
total = total + 1         ' OK
' x = 5                   ' ERROR: x is immutable
RES f AS File = fs::open("app.db", "read")   ' a resource is bound with RES
```

The binding keyword is **required and enforced**; it *surfaces* a type property,
it does not choose it:

- A resource **must** be bound with `RES`; `LET`/`MUT` on a resource is an error
  (`TYPE_RESOURCE_REQUIRES_RES`).
- `RES` binds **only** resources; `RES` on copyable data is an error
  (`TYPE_RES_REQUIRES_RESOURCE`).
- A resource appears only in `RES` positions — binding, parameter (`RES f AS
  File`), and return (`AS RES File`) — and **never inside a data type**: a record
  field of a resource type is an error (`TYPE_RESOURCE_FIELD_FORBIDDEN`).
- A `RES` binding may carry a copyable, defaultable data `STATE` (§15):
  `RES f AS File STATE FileState = …`.

Rules:

- **No implicit declaration.** Using an undeclared name is a compile error.
- **Lexical, hierarchical scope.** Inner blocks may read and shadow bindings from enclosing scopes.
- **Outer `MUT` reassignment.** An inner block may reassign an enclosing `MUT` (same live scope, same cell).
- **Collection representation follows the binding.** A collection bound with `LET` is an immutable, fixed snapshot. A collection bound with `MUT` is a locally mutable, growable buffer while it remains in that live binding. Binding a `MUT` collection to `LET`, such as `LET snap = pts`, creates an immutable snapshot; if `pts` is used afterward the snapshot is an independent copy, and if `pts` is not used afterward the compiler may freeze and move the buffer.
- **Bindings die at `END`/scope exit.**
- **Compile-time constants.** A `LET` bound to a constant expression *is* a constant expression (usable where one is required). There is no separate `CONST`.
- **Module-level state.** A top-level `MUT` is module state. There is no `GLOBAL` keyword; visibility (§13) governs sharing, and top-level `MUT` is discouraged.

```basic
LET x = 10
IF cond THEN
  LET y = x + 1           ' OK: inner sees outer x
END IF
' io::print(toString(y))       ' ERROR: y died at END IF

MUT total = 0
FOR i = 1 TO 10
  total = total + i       ' OK: reassigns enclosing MUT
NEXT
```
