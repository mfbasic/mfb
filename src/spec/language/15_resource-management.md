# 15. Resource Management

Resource values, such as files and sockets, are unique handles. At any point in the program, exactly one live owner is responsible for each open handle. A resource is bound with the **`RES`** keyword (§5) — the ownership axis — and never with `LET`/`MUT`. Resources are closed automatically by lexical drop (§14.7) when their owning binding leaves scope, on every exit path: normal scope exit, `RETURN`, `EXIT`/`CONTINUE`, `FAIL`, `PROPAGATE`, an auto-propagated failure, and `TRAP` routing. `EXIT PROGRAM` performs the same cleanup across every live caller frame before terminating. There is no user-visible lifetime construct; a resource is released by the same ownership and drop rules as any other owned value.

```basic
FUNC readFirstLine(path AS String) AS String
  RES f AS File = fs::openFile(path)   ' auto-propagates on failure
  LET line = fs::readLine(f)           ' if this fails, f is still closed on the error exit
  RETURN line                          ' f is dropped (closed) here, on the success exit
END FUNC
```

A resource is closed exactly once. **Ordinary calls borrow.** Passing a `RES` binding to an ordinary function creates an exclusive, call-scoped borrow: the callee may use the handle and mutate its `STATE`, but does not take ownership, and the caller's binding stays live after the call. A `RES` binding is invalidated **only** by this fixed set of events, all visible at the call site:

1. the resource's **registered close op** (e.g. `fs::close(f)`) and its re-export aliases;
2. **`thread::transfer`** of the resource (§16);
3. **`RETURN`** of the resource (move out to the caller);
4. **scope-drop** at the end of the binding's lexical scope (auto-close).

A borrow grants *use* but never the right to *invalidate*: a callee that only borrowed a resource cannot close it, `RETURN` it, or `thread::transfer` it (`TYPE_RESOURCE_BORROW_INVALIDATE`) — those require ownership. There is no per-function borrow/consume inference and no `BORROW`/`MOVE` annotations: a call either is one of the four events or it borrows. A resource handle cannot be printed, compared, serialized, or captured by a lambda or ordinary closure. Its pointer may be copied only as a **borrow** into a `List` element or `Map` value (§15.6) — never duplicating the resource, and never as a `Map` key. A concrete resource handle may be sent to a thread only when that resource type is thread-sendable.

```basic
RES f AS File = fs::open("app.db", "read")
exec(f, "...")        ' borrow — f still live
exec(f, "...")        ' borrow — f still live
fs::close(f)          ' registered close → f invalidated
' exec(f, "...")      ' COMPILE ERROR: f used after close
```

A resource value may be passed only to a function whose parameter is declared `RES` and explicitly names that concrete resource type, such as `RES f AS File`, `RES s AS Socket`, or a `LINK`-declared resource. A function returns a resource with an explicit `AS RES <Type>` return. There is no generic resource supertype, no structural matching of handles, and no implicit conversion between resource types.

**Resources are atomic — records never hold them.** A record (product type) may never contain a resource field, directly or transitively (`TYPE_RESOURCE_FIELD_FORBIDDEN`): [[src/rules.rs:782]] a resource field would either trap copyable data behind move-only semantics or let one value own several resources at once. Data that belongs *with* a resource travels in the resource's `STATE`, and to work with several resources you hold several `RES` bindings.

**`STATE` — data carried by a resource.** A `RES` binding may attach an associated data value with `STATE T`:

```basic
TYPE FileState        ' an ordinary, copyable data record
  pos AS Integer
  len AS Integer
END TYPE

RES s AS File STATE FileState = fs::open("app.db", "read")   ' state default-initialized
LET here = s.state.pos                                       ' read a state field
s.state.pos = 10                                             ' update one field in place
s.state = WITH s.state { pos := 10 }                         ' or replace the whole state
```

`T` must be an ordinary **copyable, defaultable data type** (`TYPE_STATE_INVALID` otherwise); since no data type may contain a resource, `T` is automatically resource-free. The state is owned by the resource, default-initializes when the resource is produced, rides through `RES` signatures (`RES s AS File STATE FileState`), and is freed when the resource drops or is closed. `STATE` is optional.

`s.state` reads the state record. It is updated either by assigning a single field in place (`s.state.field = value`) or by assigning a whole-state `WITH` update (`s.state = WITH s.state { field := value }`); the former is shorthand for the latter. These are the only member-target assignments in the language. Because a resource value is a shared handle, a state update made through a borrowed `RES` parameter is visible to the owner after the call.

**Resource unions.** A union whose every variant is a resource type is itself a resource — a *resource union* — and is `RES`-bound like any other resource:

```basic
UNION Stream            ' every variant is a resource → Stream is a resource
  File
  Socket
END UNION

RES s AS Stream = fs::open("app.db", "read")   ' a File wraps into the union
MATCH s
  CASE File(f)
    LET line = fs::readLine(f)
  CASE Socket(sock)
    LET data = net::read(sock, 1024)
END MATCH
' scope end → drop closes the active variant via its registered close op
```

A resource union owns exactly one resource at a time (the active variant), so it is atomic — a *choice* among resources, not a bundle. **Drop is tag-dispatched**: cleanup reads the union tag and calls the active variant's registered close op. Matching a resource union *borrows* the active variant (the union retains ownership and closes it on drop). A union may **not mix** data and resource variants (`TYPE_MIXED_RESOURCE_UNION`), and a resource union carries no `STATE`.

To release a resource earlier than the end of its scope, or to observe a close failure, call the resource's explicit close operation (such as `fs::close(f)`). That operation consumes the handle and auto-propagates a close failure like any other call, so the close failure is directly observable. After an explicit close the binding is moved and is not closed again by lexical drop.

A close that runs as part of an implicit lexical drop cannot inject an error into program flow, because a drop has no source-level result to route. If such a drop-close fails, the failure is emitted as diagnostic/audit metadata associated with the failed cleanup; it does not replace, wrap, or raise a source-level `Error`. Programs that must observe a close failure use the explicit close operation instead.

This rule does not change the built-in `Error` shape: A secondary close failure is not directly inspectable by ordinary source code unless a future diagnostics API exposes cleanup metadata.

Compiled cleanup metadata must preserve enough information for runtime and audit tooling to report a drop-close failure. Package audit output should identify cleanup regions that retain this failure metadata.

## 15.6 Resources in collections

A resource is owned by a **scope** — never by a binding or a collection. A `RES` binding, a borrowed `RES` parameter, and a collection slot (a `List` element or `Map` value) all hold a **borrow**: a copy of the one handle pointer. Copying the pointer is a borrow, never a duplication of the resource, and a collection slot is a borrow, not a resource binding. None of these close the resource; the owning scope closes it exactly once on exit, on every path.

A resource appearing as a collection element carries the **`RES` ownership-axis marker**, exactly as a binding (`RES f`), a parameter (`RES f AS File`), or a return (`AS RES File`) does. The only spelling for a list of files is `List OF RES File` (and `Map OF String TO RES File` for a map value); a bare `List OF File` is rejected just like `LET f AS File` (`TYPE_RESOURCE_REQUIRES_RES`), and `RES` on a non-resource element is rejected like `RES x AS Integer` (`TYPE_RES_REQUIRES_RESOURCE`). The marker is an ownership annotation only — the collection is still an ordinary copyable collection of borrows and owns nothing.

By default the owning scope is the scope where the resource is produced. The single rule that governs collections is **ownership floats up**:

> Adding a borrow of a resource to a collection migrates the resource's owning scope up to the collection's scope when that scope outlives the current owner. Ownership always floats to the **outermost** scope that references the resource; it never moves down. If a referencing collection escapes the function (it is `RETURN`ed), ownership moves out to the caller, exactly like `RETURN`ing the resource itself.

Consequences:

- A borrow added to a **higher-scope** collection raises the owning scope to that collection's scope; the resource closes once when that outer scope exits, and every borrow (the original binding and the collection elements) is within that scope, so none dangles.
- A borrow added to a **same- or lower-scope** collection leaves ownership unchanged; the collection just holds a borrow.
- A binding whose ownership has floated to an outer scope becomes a plain **borrow**: still usable, but it no longer closes at its own scope exit and may not close, `RETURN`, or `thread::transfer` the resource (`TYPE_RESOURCE_BORROW_INVALIDATE`).

Because all references are within the owning scope, `get` and `FOR EACH` of a resource element yield a **borrow**, statically safe with no runtime dependence on the closed flag (the flag is only a backstop that keeps the single close idempotent when a handle is reachable by more than one path). Such a borrow is not an owner: binding it with `RES`, or closing/returning/transferring it, is an error (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`). Collections of resources are ordinary copyable collections of pointers — no move-only or linearity — and the helpers that require a comparable element (`find`, `contains`, `replace`) remain unavailable because handles are not comparable, the same reason resources cannot be `Map` keys.

A resource element placed into a collection must be a named `RES` binding (the owner); a temporary or a borrowed element is not an owner and cannot be stored (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`). [[src/rules.rs:614]]

**Returning a resource collection transfers scope-ownership to the caller**, exactly as `AS RES File` does for a single resource. A function returning `AS List OF RES File` releases the close obligations for the referenced resources — it does not close them — and the caller's binding scope **adopts** them, closing each once at its own exit. (A bare `List OF File` return is rejected for the missing `RES` marker.) On an error exit *before* the return, the resources are still closed by the function's scope, because they ride its owned-list until the `RETURN` transfers it. A resource collection may also be passed to a function, where the callee borrows its elements (and may not close them). The resources must be added to the collection at or after the collection's own binding so the obligation rides the collection. Sharing a resource collection across threads remains out of scope.

## See Also

* ./mfb spec package resource-regions — how resources are encoded in the `.mfp` `RESOURCE_TABLE`
* ./mfb spec language threads — `thread::transfer`/`accept` resource plane
