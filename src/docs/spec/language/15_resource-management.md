# 15. Resource Management

Resource values, such as files and sockets, are unique handles. At any point in the program, exactly one live owner is responsible for each open handle. A resource is bound with the **`RES`** keyword (§5) — the ownership axis — and never with `LET`/`MUT`. Resources are closed automatically by lexical drop (§14.7) when their owning binding leaves scope, on every exit path: normal scope exit, `RETURN`, `EXIT`/`CONTINUE`, `FAIL`, `PROPAGATE`, an auto-propagated failure, and `TRAP` routing. `EXIT PROGRAM` performs the same cleanup across every live caller frame before terminating. There is no user-visible lifetime construct; a resource is released by the same ownership and drop rules as any other owned value.

```basic
FUNC readFirstLine(path AS String) AS String
  RES f AS File = fs::openFile(path)   ' auto-propagates on failure
  LET line = fs::readLine(f)           ' if this fails, f is still closed on the error exit
  RETURN line                          ' f is dropped (closed) here, on the success exit
END FUNC
```

A resource is closed exactly once. **Ordinary calls do not move ownership.** There is one resource; a `RES` is a pointer to it. Passing a `RES` binding to an ordinary function hands the callee that same pointer to that same one resource: the callee may use the handle and mutate its `STATE`, but the owning scope is unchanged, and the caller's binding stays live after the call. A `RES` binding is invalidated **only** by this fixed set of events, all visible at the call site:

1. the resource's **registered close op** (e.g. `fs::close(f)`) and its re-export aliases;
2. **`thread::transfer`** of the resource (§16);
3. **`RETURN`** of the resource (move out to the caller);
4. **scope-drop** at the end of the binding's lexical scope (auto-close).

A resource is owned by the **outermost scope that touches it**, and a `RES` is a pointer to the one resource. Holding that pointer grants the right to *invalidate*: any holder may close it, `RETURN` it, or `thread::transfer` it, including a callee given only a `RES` parameter. The owning scope then closes it once **if nobody already did** — so `closeSound(RES sound AS SoundFile)`, "take a handle, give it back", is expressible.

A resource is still closed **exactly once**. What changed is *who* may close it, never *how many times*: an already-closed record is flagged, and a second close is a defined no-op reported as `ErrResourceClosed` rather than an operation on a dead handle. There is no per-function ownership inference and no `MOVE` annotation: a call either is one of the four events or it leaves the resource where it was.

Because two names can denote one resource, closing through either marks both, and a later *use* of a closed name is `TYPE_USE_AFTER_MOVE` wherever the compiler can still prove it. Where it cannot — through a call that may hand the same resource back — the runtime flag catches it instead. A resource handle cannot be printed, compared, serialized, or captured by a lambda or ordinary closure. Its pointer may be copied into a `List` element or `Map` value (§15.6) — never duplicating the resource, and never as a `Map` key. A concrete resource handle may be sent to a thread only when that resource type is thread-sendable.

```basic
RES f AS File = fs::open("app.db", "read")
exec(f, "...")        ' pointer passed — f still live
exec(f, "...")        ' pointer passed — f still live
fs::close(f)          ' registered close → f invalidated
' exec(f, "...")      ' COMPILE ERROR: f used after close
```

A resource value may be passed only to a function whose parameter is declared `RES` and explicitly names that concrete resource type, such as `RES f AS File`, `RES s AS Socket`, or a `LINK`-declared resource. A function returns a resource with an explicit `AS RES <Type>` return. There is no generic resource supertype, no structural matching of handles, and no implicit conversion between resource types.

**Resources are atomic — records never hold them.** A record (product type) may never contain a resource field, directly or transitively (`TYPE_RESOURCE_FIELD_FORBIDDEN`): [[src/rules/table.rs:TYPE_RESOURCE_FIELD_FORBIDDEN]] a resource field would either trap copyable data behind move-only semantics or let one value own several resources at once. Data that belongs *with* a resource travels in the resource's `STATE`, and to work with several resources you hold several `RES` bindings.

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

`T` must be an ordinary **copyable, defaultable data type** (`TYPE_STATE_INVALID` otherwise); since no data type may contain a resource, `T` is automatically resource-free. The state is owned by the resource, default-initializes when the resource is produced, rides through `RES` signatures (`RES s AS File STATE FileState`), and is freed when the resource **drops**. `STATE` is optional. An explicit close (`fs::close(s)`) releases the OS handle but reclaims no memory, so `s.state` still reads its payload after one; drop is what frees it.

`s.state` reads the state record. It is updated either by assigning a single field in place (`s.state.field = value`) or by assigning a whole-state `WITH` update (`s.state = WITH s.state { field := value }`); the former is shorthand for the latter. These are the only member-target assignments in the language. Because a `RES` is an alias to one live resource, a state update made through a `RES` parameter (an alias, not a copy) is visible to the owner after the call.

## 15.5 What `STATE` means in each position

A resource *type* carries no `STATE`: a `RESOURCE` declaration is `RESOURCE SfFile CLOSE BY sfClose` and has no `STATE` clause. [[src/ast/items.rs:parse_top_level_resource]] `STATE` is written at the three *use* positions — a parameter, a return, and a binding — and the same two spellings mean different things at each:

| Position | `RES x AS SfFile` (bare) | `RES x AS SfFile STATE FileInfo` |
|---|---|---|
| **Parameter** | accepts a `SfFile` carrying **any state or none**; `x.state` is **not** accessible | accepts **only** a `SfFile` carrying a `FileInfo`; `x.state` is accessible |
| **Return** | **asserts** the resource has no state; rejected if it carries one | returns a resource **carrying** a `FileInfo` |
| **Binding** | **asserts** the resource has no state; rejected if it carries one | **attaches** a default-initialized `FileInfo` (when the value carries none), or **adopts** the one the value already carries |

Bare therefore reads two ways: **"opaque"** at a parameter, and **"asserts none"** at a return or a binding. Bare never *strips* a `STATE` — at a return or a binding it is a claim that there is none, and the claim is checked:

```basic
RES a AS File STATE Cur = fs::open("/tmp/x.txt", "w")
RES b AS File           = a    ' error 2-203-0129: `b` is bare but its initializer
                               '   carries STATE `Cur`
```

The rule behind the asymmetry is **what the compiler can prove about the STATE type**, not escape and not ownership words:

> A resource's `STATE` type is fixed **once, at the binding that creates it**, and carries no runtime tag. Every later declaration that names a `STATE` is therefore checked against that fixed type and must agree (`TYPE_STATE_MISMATCH`) [[src/rules/table.rs:TYPE_STATE_MISMATCH]]. A **return** and a **binding** are positions where the declaration names a type, so each is checked directly — a bare one asserts "no state" and is rejected against a stateful resource, exactly as a mismatched concrete one is. A **parameter** is the single position that deliberately declines to name the type: bare there means *opaque* — "some state or none" — so that one close op serves every state its callers attach. Opacity is sound because it is **unobservable**: `x.state` is inaccessible through a bare parameter, so no code can read the payload under a type the compiler has not checked.

The asymmetry is thus about **knowledge, not lifetime**. `STATE` agreement holds independently of whether a resource escapes: the check happens wherever a type is named, and the one place no type is named is the one place nothing can be misread. A `thread::transfer` across the thread boundary is checked the same way (the plane names the `STATE`; `TYPE_STATE_MISMATCH` on disagreement, §16). See `./mfb spec architecture escape-analysis`.

Because a bare parameter is opaque rather than stateless, an opaque `STATE` may not be **narrowed** to a concrete one: a bare `RES f AS SfFile` parameter cannot be returned or bound as `RES … AS SfFile STATE FileInfo`, since the compiler knows only that `f` has *some* state and cannot discharge the claim that it is a `FileInfo` (`TYPE_STATE_OPAQUE_NARROWING`). Opacity propagates as opacity; only a producer that knows the type may name it.

The parameter row is what lets a close op accept a resource whatever state its owner attached — `FUNC close(RES db AS Db)` names no `STATE` and works for every `Db`, precisely because that alias never escapes to re-read `.state` under a new type.

**Attachment happens exactly once, at the owning binding.** A parameter only observes: a `RES p AS File STATE Cursor` parameter given an argument that carries no `Cursor`, or that carries some other state type, is rejected (`TYPE_STATE_MISMATCH`) [[src/rules/table.rs:TYPE_STATE_MISMATCH]] rather than attaching or re-typing one. The payload carries no runtime type tag, so its type is fixed by the binding that created it and every later declaration must agree.

Consequently a binding that wants only the handle must still restate the `STATE` its initializer carries; there is no owner-side opt-out. That restatement is the price of the bare return's "no state" promise, which a later `STATE T` binding relies on when it attaches.

**Returning a resource that carries state.** A `FUNC` returns one by naming the `STATE` on its return, and the state the callee populated arrives intact at the caller's binding:

```basic
FUNC openTagged(path AS String) AS RES File STATE Cursor
  RES f AS File STATE Cursor = fs::openFile(path)
  f.state.pos = 42
  RETURN f                                    ' the Cursor rides the return
END FUNC

RES h AS File STATE Cursor = openTagged(p)    ' adopts it — h.state.pos is 42
LET here = openTagged(p).state.pos            ' `.state` resolves from the call too
```

The caller's binding **adopts** the carried state rather than re-initializing it, so `h.state.pos` reads `42` and not `0`. A bare binding of that same call (`RES h AS File = openTagged(p)`) is rejected: bare asserts "no state", and accepting it would let a function launder a carried state through a bare return into a caller that attaches a second one over it.

The `STATE` rides an exported signature, so this works across a package boundary exactly as it does in-package.

See `./mfb spec architecture escape-analysis` for the ownership/escape decision procedure this rule rests on.

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

A resource union owns exactly one resource at a time (the active variant), so it is atomic — a *choice* among resources, not a bundle. **Drop is tag-dispatched**: cleanup reads the union tag and calls the active variant's registered close op. Matching a resource union yields a pointer to the active variant (the union retains ownership and closes it on drop). A union may **not mix** data and resource variants (`TYPE_MIXED_RESOURCE_UNION`), and a resource union carries no `STATE`.

To release a resource earlier than the end of its scope, or to observe a close failure, call the resource's explicit close operation (such as `fs::close(f)`). That operation consumes the handle and auto-propagates a close failure like any other call, so the close failure is directly observable. After an explicit close the binding is moved and is not closed again by lexical drop.

**Close releases the OS handle; drop reclaims memory.** They are separate events. A close returns the file descriptor (or the library handle) and reports failure, but frees nothing; the drop that ends the binding's scope reclaims what the resource's record points at — its `STATE` payload and its I/O buffers — so a loop that opens and closes handles retains a flat amount per resource rather than growing with the I/O each one did. The record itself is retained until the thread's arena is torn down: it holds the closed flag that makes a re-close idempotent and that every alias reads. [[src/target/shared/code/builder_codegen_primitives.rs:emit_resource_block_reclaim]]

**A transferred handle is moved, not closed.** `thread::transfer` hands the resource to the receiving thread and marks the sender's handle **moved**. The static move rules normally make using it a compile error; where a handle nonetheless reaches an operation moved (an alias those rules do not track), the operation is refused with `ErrResourceMoved` rather than `ErrResourceClosed`, because the handle is not closed — it belongs to another thread now. [[src/target/shared/code/error_constants.rs:RESOURCE_MOVED_BIT]]

A close that runs as part of an implicit lexical drop cannot inject an error into program flow, because a drop has no source-level result to route. If such a drop-close fails, the failure is emitted as diagnostic/audit metadata associated with the failed cleanup; it does not replace, wrap, or raise a source-level `Error`. Programs that must observe a close failure use the explicit close operation instead. Re-closing an already-closed handle during a drop is not such a failure — it is a benign no-op and is never reported as a drop-close failure.

When a fallible resource-producing operation fails (for example the error binding of a `RES x = <fallible> TRAP` whose handler diverges), the binding is never left holding an invalid handle: it materializes as an **already-closed** handle. Its close op is therefore an idempotent no-op, and no unopened or null handle is ever exposed to a program or to the drop path. This makes every resource default *closed*, so an operation reaching such a value raises the resource-closed error and a drop of it is safe.

This rule does not change the built-in `Error` shape: A secondary close failure is not directly inspectable by ordinary source code unless a future diagnostics API exposes cleanup metadata.

Compiled cleanup metadata must preserve enough information for runtime and audit tooling to report a drop-close failure. Package audit output should identify cleanup regions that retain this failure metadata.

## 15.6 Resources in collections

A resource is owned by a **scope** — never by a binding or a collection. A `RES` binding, a `RES` parameter, and a collection slot (a `List` element or `Map` value) all hold **a copy of the one handle pointer**. Copying the pointer never duplicates the resource, and a collection slot is a pointer, not a resource binding. None of these close the resource; the owning scope closes it exactly once on exit, on every path.

A resource appearing as a collection element carries the **`RES` ownership-axis marker**, exactly as a binding (`RES f`), a parameter (`RES f AS File`), or a return (`AS RES File`) does. The only spelling for a list of files is `List OF RES File` (and `Map OF String TO RES File` for a map value); a bare `List OF File` is rejected just like `LET f AS File` (`TYPE_RESOURCE_REQUIRES_RES`), and `RES` on a non-resource element is rejected like `RES x AS Integer` (`TYPE_RES_REQUIRES_RESOURCE`). The marker is an ownership annotation only — the collection is still an ordinary copyable collection of pointers and owns nothing.

By default the owning scope is the scope where the resource is produced. The single rule that governs collections is **ownership floats up**:

> Adding a pointer to a resource to a collection migrates the resource's owning scope up to the collection's scope when that scope outlives the current owner. Ownership always floats to the **outermost** scope that references the resource; it never moves down. If a referencing collection escapes the function (it is `RETURN`ed), ownership moves out to the caller, exactly like `RETURN`ing the resource itself.

Consequences:

- A pointer added to a **higher-scope** collection raises the owning scope to that collection's scope; the resource closes once when that outer scope exits, and every pointer (the original binding and the collection elements) is within that scope, so none dangles.
- A pointer added to a **same- or lower-scope** collection leaves ownership unchanged; the collection just holds a pointer.
- A binding whose ownership has floated to an outer scope becomes a plain **pointer**: still usable, and it no longer closes at its own scope exit. It *may* still close, `RETURN`, or `thread::transfer` the resource — any holder may — in which case the outer scope finds it already closed and its own close is a defined no-op.

`get` and `FOR EACH` of a resource element yield a **pointer** to the one resource, like any other holder. It may be bound with `RES`, closed, `RETURN`ed, or transferred; the resource is still closed exactly once, by the outermost scope that touches it, with the closed flag making any later close a defined no-op. Collections of resources are ordinary copyable collections of pointers — no move-only or linearity — and the helpers that require a comparable element (`find`, `contains`, `replace`) remain unavailable because handles are not comparable, the same reason resources cannot be `Map` keys.

A resource element placed into a collection may be any pointer to the resource — a named `RES` binding or a temporary alike. The collection holds pointers to resources owned by the outermost scope that touches them.

**Returning a resource collection transfers scope-ownership to the caller**, exactly as `AS RES File` does for a single resource. A function returning `AS List OF RES File` releases the close obligations for the referenced resources — it does not close them — and the caller's binding scope **adopts** them, closing each once at its own exit. (A bare `List OF File` return is rejected for the missing `RES` marker.) On an error exit *before* the return, the resources are still closed by the function's scope, because they ride its owned-list until the `RETURN` transfers it. A resource collection may also be passed to a function, where the callee receives pointers to its elements (and may not close them). The resources must be added to the collection at or after the collection's own binding so the obligation rides the collection; violating this is rejected (`TYPE_RESOURCE_RETURN_ORDER`) rather than compiled. [[src/rules/table.rs:TYPE_RESOURCE_RETURN_ORDER]] Before bug-291 it was not enforced, and a collection declared after its resource silently produced a returned handle that the function had already closed and the caller then closed again. Sharing a resource collection across threads remains out of scope.

The float rules above are the source-level contract. The compiler implements them with a purely syntactic per-function **decision procedure** — which collection a binding floats to (outermost referencing scope, the special case that a returned collection declared before the resource forces a float at the same depth, fixpoint propagation along collection copy/append/nesting edges, the insertion-builtin set, and the declaration-order tiebreak) — that is specified in full by `./mfb spec architecture escape-analysis`. Programs depend only on the contract here, not on the procedure's mechanics.

## See Also

* ./mfb spec architecture escape-analysis — the decision procedure that assigns each `RES` binding its owner (`Local`/`Float`)
* ./mfb spec package resource-regions — how resources are encoded in the `.mfp` `RESOURCE_TABLE`
* ./mfb spec language threads — `thread::transfer`/`accept` resource plane
