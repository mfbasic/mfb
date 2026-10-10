# 15. Resource Management

A resource has one identity, such as an open file or socket, and may have several handle values referring to that identity. Cleanup responsibility and the validity of a particular binding are separate from identity (see the model below). A resource is bound with the **`RES`** keyword (§5) — the ownership axis — and never with `LET`/`MUT`. Resources are closed automatically by lexical drop (§14.7) when their cleanup scope exits, on every exit path: normal scope exit, `RETURN`, `EXIT`/`CONTINUE`, `FAIL`, `PROPAGATE`, an auto-propagated failure, and `TRAP` routing. `EXIT PROGRAM` performs the same cleanup across every live caller frame before terminating. There is no user-visible lifetime construct; a resource is released by the same ownership and drop rules as any other owned value.

```basic
FUNC readFirstLine(path AS String) AS String
  RES f AS fs::File = fs::openFile(path)   ' auto-propagates on failure
  LET line = fs::readLine(f)           ' if this fails, f is still closed on the error exit
  RETURN line                          ' f is dropped (closed) here, on the success exit
END FUNC
```

A resource is closed exactly once. **Passing a resource argument does not itself move ownership.** There is one resource; a `RES` is a reference to it. Passing a `RES` binding to an ordinary function hands the callee another reference to that same one resource: the callee may use the handle and mutate its `STATE`, but passing the argument leaves the cleanup scope unchanged. The callee may nevertheless close, return, or transfer that resource. Resource consumption and cleanup events are:

1. the resource's **registered close op** (e.g. `fs::close(f)`) and its re-export aliases;
2. **`thread::transfer`** of the resource (§16);
3. **`RETURN`** of the resource from the scope that owns it (release to the caller, below);
4. **scope-drop** at the end of the assigned cleanup scope (auto-close).

Separately, a direct-name `RES new = old` can invalidate the source name under
the static move rule below without closing the resource or changing its cleanup
scope. Consumption inside an ordinary callee is not necessarily statically
visible at the caller; runtime closed/moved checks still apply.

A resource has a **cleanup scope**, determined by production, container routing (§15.6), and explicit return or thread transfer. Reading a reference or passing it as an ordinary argument does not by itself change that scope. Holding a reference grants the right to *invalidate*: any holder may close it, `RETURN` it, or `thread::transfer` it, including a callee given only a resource parameter. The owning scope then closes it once **if nobody already did** — so `closeSound(sound AS SoundFile)`, "take a handle, give it back", is expressible.

### The owner of a resource

The cleanup scope belongs to the **resource**, not to any name that refers to it:

1. **A resource holds its cleanup scope. A `RES` does not.** Every `RES` binding,
   resource parameter, record field, and collection slot is a *reference* to the one
   resource; none of them is an owner.
2. **A resource's scope only floats up, never down.** It starts at the scope whose
   binding first takes the freshly produced resource. Adding it to a collection whose
   scope is further out moves it out (§15.6). Nothing moves it in: binding a reference
   in an inner scope, passing it to a callee, or a callee handing it back leaves it
   where it is.
3. **`RETURN` from the owning scope releases it to the caller.** A resource owned by
   a scope of the returning function — produced there, or floated to one of its
   scopes — is unowned between the `RETURN` and the caller's binding, and the caller's
   binding scope takes it. A resource the function only received is still owned
   further out, and `RETURN`ing it changes nothing.

So `RES y = ident(a)` inside an inner block, where `ident` returns its `RES`
parameter, is one more reference to `a`'s resource: the inner block's exit closes
nothing, and `a`'s scope closes it once.

Every scope exit — normal, `EXIT`, `CONTINUE`, `FAIL`, `PROPAGATE`, an
auto-propagated failure, `TRAP` routing, `EXIT PROGRAM` — closes a resource it
registered only when that scope is still the resource's owner. A call statement
that discards a resource result closes it when nobody owns it (a resource the callee
produced or released) and leaves a resource that still has an owner alone. The same
rule closes a resource produced **inline** that nothing takes — a producer call
used as a call argument (`useF(fs::open(p, "read"))`), as the target of a member
read (`openTagged(p).state.pos`), or as an element of a list or record literal that
is itself passed on (`useList([mk()])`): at the end of its statement, every
resource the value reaches that is still unowned and not already closed is closed,
on every exit from the statement. A call to a function with a body that returns an
aggregate reaching resources is such a producer too (`len(mkList())`, a returned
record, a `List` or `Map` of them): its `RETURN` released exactly the resources its
own scopes owned, so the ones still unowned at the end of the statement are the ones
it handed over and nothing took. Two aggregates are views and are left alone: one a
natively implemented builtin returns, which can name resources another owner holds,
and one from a function whose every `RETURN` hands back a parameter it never
reassigns or a module-level binding, which transfers nothing. One a binding, an adopting container, a global or
a `RETURN` took by then has an owner and is left to it, so `RES k = ident(mk())`
and `RETURN ident(mk())` keep theirs. An inline `TRAP` over a call that hands out
resources (`LET xs = mkList() TRAP … END TRAP`) adopts them into the binding's scope
exactly as the plain call does. [[src/codegen/engine/value/builder_values.rs:register_resource_temp]] [[src/codegen/resource/owner.rs:emit_resource_temp_close]] [[src/codegen/resource/owner.rs:emit_claim_returned_value]] [[src/codegen/cleanup/owned/builder_owned_cleanup.rs:trap_call_result_locals]]

The owner is a word on the resource record (`./mfb spec memory heap-values`, "The
owner word"): `0` while unowned, else the token of the owning scope. A binding
**claims** an unowned resource, at its bind and at every assignment that stores a new
one (for a resource union, the active variant's record); a float raises the owner to the outer collection
scope; a `RETURN` writes `0` for every resource its value carries that a scope of
the returning function owns; a scope-exit close runs only when the owner equals that
scope's token. `thread::accept` produces an unowned copy that its binding claims.
[[src/codegen/resource/owner.rs:emit_resource_claim]] [[src/codegen/resource/owner.rs:emit_resource_float]] [[src/codegen/resource/owner.rs:emit_release_returned_value]] [[src/codegen/resource/owner.rs:emit_skip_unless_owned]] [[src/codegen/resource/owner.rs:emit_close_discarded_value]]

**Thread handles are resources too.** A parent `Thread` follows every rule here, with one difference: its registered close op (`thread.drop`) is hidden, so event 1 is not available to a program; explicit resource hand-off uses `thread::transfer` or `RETURN`, and automatic close occurs at its cleanup scope's drop — which cancels a worker still running (§16). A `ThreadWorker` is a resource as well — bound with `RES`, aliased, moved and stored by the same rules — that has no close op at all: the runtime closes it when the worker returns, so its cleanup scope's drop closes nothing. Whether a type is a resource is decided by the type, never by whether it has a close op. [[src/codegen/builtins/thread/mod.rs:DROP]] [[src/codegen/resource/mod.rs:is_builtin_resource]]

An already-closed resource record is flagged, and a second close is a defined no-op reported as `ErrResourceClosed` rather than an operation on a dead handle. There is no source `MOVE` annotation. A call site need not reveal a close or transfer performed inside its callee.

Because two names can denote one resource, closing through either marks both, and a later *use* of a closed name is `TYPE_USE_AFTER_MOVE` wherever the compiler can still prove it. Where it cannot — through a call that may hand the same resource back — the runtime flag catches it instead. A resource handle cannot be printed, compared, or serialized. A reference to it may be copied into a `List` element, a `Map` value, a record field, or a closure's captures (§15.6) — never duplicating the resource, and never as a `Map` key. A concrete resource handle may be sent to a thread only when that resource type is thread-sendable.

### Resource identity, views, cleanup, and binding validity

Resource validity is enforced by a combination of conservative static checks and
runtime closed/moved checks, not by full static linearity. A statically invalid
name reports `TYPE_USE_AFTER_MOVE`; a guarded operation through an untracked
closed view reports `ErrResourceClosed`, and through a transferred view reports
`ErrResourceMoved`. These checks are distinct from the lifetime of the resource
record itself. How a transfer's success and failure edges enter the static check
is specified in §14.9.
[[src/ir/verify/resources.rs:check_resource_moves]]
[[src/codegen/resource/mod.rs:closed_or_moved_guard]]

- **Identity** is the single resource record and its underlying handle. Its
  closed/moved state and attached `STATE` are shared by every view.
- **View** is a reference in a binding, parameter, match binding, field, or
  collection slot. Copying a view does not duplicate the resource.
- **Cleanup obligation** is responsibility for closing a still-open resource
  on scope exit. It belongs to the resource's owning scope, never to a view.
  **Cleanup scope** is where that obligation is discharged. Several cleanup sites
  can be registered for the same identity; the owner word lets only the owning
  scope's close run, and the shared closed flag makes any other close a no-op. A
  unique resource does not imply a unique reference or a unique statically
  scheduled cleanup site.
- **Binding validity** is the verifier's permission to read a name. A name
  marked moved is unusable even if another view still refers to an open resource.
  This static state is distinct from the resource's runtime closed/moved state.

The following table separates initialization from cleanup. The direct-name move
rule applies when the destination is a `RES` owner-table binding and the source's
concrete resource type has a registered close operation. It does not ask whether
that source name owns cleanup; parameters and other non-owning names qualify too.
[[src/ir/verify/resources.rs:check_resource_moves]]

| Initializer / binding form | Identity and source-name validity | Cleanup consequence |
| --- | --- | --- |
| `RES b = a`, where `a` is a concrete resource local | Same identity; `a` is marked moved | Reference alias; no additional cleanup at `b`'s scope merely for copying the reference |
| `RES b = p`, where `p` is a resource parameter | Same direct-name rule; `p` is marked moved in this function | Does not acquire the caller's cleanup obligation |
| `RES b = item`, where `item` is a concrete `MATCH` or `FOR EACH` resource binding | Same direct-name rule; `item` is marked moved | Existing union/resource/container cleanup remains responsible |
| `RES b = a` after `a` has floated | Same direct-name rule; floating does not exempt `a` | The existing container cleanup remains responsible |
| `RES b = holder.handle` | View of the field's resource; no direct source name is moved | No new close obligation |
| `RES b = collections::get(xs, i)` or resource-valued `getOr` | View of the selected resource; container remains usable | No new close obligation; the same rule applies to resource-valued `tcp::poll`, `udp::poll`, and `tls::poll` |
| `RES b = producer(...)`, or a function that returns a resource it produced (or one floated to its own scope) | New or released identity; arguments are not marked moved merely by the call | `b`'s scope claims it and closes it once, unless its obligation floats |
| `RES b = f(...)` where `f` hands back a resource with a live owner — a resource argument, or anything it received | Same identity as that argument | Reference only: `b`'s scope closes nothing; the existing owner still does |
| `MATCH` extraction or `FOR EACH` element binding itself | View of the existing resource; extraction alone is not a direct-name `RES` move | No independent close obligation |

Native lowering classifies local and member reads and the named extraction calls
as aliases. A direct-name move therefore changes static name validity without
necessarily relocating runtime cleanup. Resource-union wrapping has separate
variant cleanup bookkeeping; it is not the concrete-close-op test above.
[[src/codegen/engine/value/builder_values.rs:value_aliases_live_resource]]
[[src/codegen/engine/control/builder_control.rs:lower_ops_inner]]

Closing ends the resource's open state. `RETURN` hands a resource value to the
caller; when a scope of the returning function owns it, the return releases it and
the returning scopes' cleanups skip it; `thread::transfer` changes its thread ownership. Neither operation is a synonym
for close. Container routing changes cleanup placement as specified in §15.6;
reading, matching, or extracting a view does not itself extend the lifetime.
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_cleanup_call]]

```basic
RES f AS fs::File = fs::open("app.db", "read")
exec(f, "...")        ' reference passed — f still live
exec(f, "...")        ' reference passed — f still live
fs::close(f)          ' registered close → f invalidated
' exec(f, "...")      ' COMPILE ERROR: f used after close
```

**A parameter is a resource because its type is one.** A parameter carries no `RES` keyword: `FUNC size(f AS fs::File)` takes a resource parameter because `fs::File` is a resource, and writing `RES f AS fs::File` is a parse error (`MFB_PARSE_PARAMETER_RES`), located on the keyword. [[src/ast/items.rs:parse_params]] [[src/rules/table.rs:MFB_PARSE_PARAMETER_RES]] The type decides it with the rule that makes a binding of the type need `RES`: a type with a registered close op (a built-in resource, a `RESOURCE` declaration, an imported package's resource) or a resource union; a parameter that carries a `STATE` clause is a resource parameter too. [[src/hir/resource_params.rs:ResourceTypes]] A resource value may be passed to a parameter that either **explicitly names that concrete resource type** — such as `f AS fs::File`, `s AS tcp::Socket`, or a `LINK`-declared resource — **or names a resource union of which that type is a variant** (a `File` value passed to an `s AS Stream` parameter, where `Stream` is the resource union below). A function returns a resource with an explicit `AS RES <Type>` return.

This variant→union widening is **directional** and applies only in **non-owning parameter position**: a variant value widens *into* a union-typed parameter, never the reverse. A union-typed value passed to a parameter that names a **concrete** resource type is rejected (`TYPE_CALL_ARGUMENT_MISMATCH`). Consequently every registered close op, `thread::transfer`, and `thread::accept` — each of which declares a concrete-typed parameter — keeps its exact type and is unaffected: a resource union is dropped by dispatching on its tag to the active variant's close op, never by handing the whole union to a concrete close. The widening is **representation-neutral**: a resource value already carries its own kind, so a variant argument lowers to the same single handle whether the parameter names the concrete type or the union — no tagged temporary is materialized. Binding, returning, and consuming a resource union are unchanged; only the non-owning-parameter position widens.

There is otherwise no generic resource supertype (a resource union is a specific, user-declared set of variants, not a universal handle type), no structural matching of handles, and no implicit conversion between unrelated resource types.

**A record field may hold a resource, under the same rules a collection slot does.** A field marked `RES` — `TYPE Holder { handle AS RES fs::File }` — holds a reference to the one resource and owns nothing by itself; copying the record copies the reference and aliases the same resource, it does not duplicate it (§15.6). The field is governed by the two marker rules a collection element takes: a bare resource field is rejected for the missing marker (`TYPE_RESOURCE_REQUIRES_RES`), and `RES` on a non-resource is rejected as meaningless (`TYPE_RES_REQUIRES_RESOURCE`). [[src/rules/table.rs:TYPE_RESOURCE_REQUIRES_RES]] Cleanup routing follows the same eligible-scope and return-order rules as for a collection (§15.6); merely copying a reference into a shorter-lived record does not shorten the resource's cleanup lifetime.

A record carrying a resource is still **not comparable**, cannot be a `Map` or `Set` key, and cannot cross a thread's data plane (`TYPE_THREAD_RESOURCE_PLANE_REQUIRED`, §16). Data that belongs *with* a resource may still travel in the resource's own `STATE`.

**`STATE` — data carried by a resource.** A `RES` binding may attach an associated data value with `STATE T`:

```basic
TYPE FileState        ' an ordinary, copyable data record
  pos AS Integer
  len AS Integer
END TYPE

RES s AS fs::File STATE FileState = fs::open("app.db", "read")   ' state default-initialized
LET here = s.state.pos                                       ' read a state field
s.state.pos = 10                                             ' update one field in place
s.state = WITH s.state { pos := 10 }                         ' or replace the whole state
```

`T` must be an ordinary **copyable, defaultable data type** (`TYPE_STATE_INVALID` otherwise). A direct `RES` field prevents a record from being defaultable, so such a record cannot be `T`. A record with a collection of resource references can be defaultable because the collection defaults to empty; it remains prohibited on a thread data plane (`TYPE_THREAD_RESOURCE_PLANE_REQUIRED`, §16). [[src/ir/verify/resources.rs:is_defaultable]] The state is owned by the resource, default-initializes when the resource is produced, rides through `RES` signatures (`RES s AS fs::File STATE FileState`), and is freed when the resource **drops**. `STATE` is optional. An explicit close (`fs::close(s)`) releases the OS handle but reclaims no payload memory; drop frees it. Static move checking still rejects a later read through a binding known to have been closed.

`s.state` reads the state record. It is updated either by assigning a single field in place (`s.state.field = value`) or by assigning a whole-state `WITH` update (`s.state = WITH s.state { field := value }`); the former is shorthand for the latter. These are the only member-target assignments in the language. Because a `RES` is an alias to one live resource, a state update made through a resource parameter (an alias, not a copy) is visible to the owner after the call.

## 15.5 What `STATE` means in each position

A resource *type* carries no `STATE`: a `RESOURCE` declaration is `RESOURCE SfFile CLOSE BY sfClose` and has no `STATE` clause. [[src/ast/items.rs:parse_top_level_resource]] `STATE` is written at the *use* positions — a parameter, a return, a binding, and a **slot** (a collection element/map value, or a record field) — and the same two spellings mean different things at each:

| Position | bare (`… x AS SfFile`) | `… x AS SfFile STATE FileInfo` |
|---|---|---|
| **Parameter** | accepts a `SfFile` carrying **any state or none**; `x.state` is **not** accessible | accepts **only** a `SfFile` carrying a `FileInfo`; `x.state` is accessible |
| **Return** | **asserts** the resource has no state; rejected if it carries one | returns a resource **carrying** a `FileInfo` |
| **Binding** | **asserts** the resource has no state; rejected if it carries one | **attaches** a default-initialized `FileInfo` (when the value carries none), or **adopts** the one the value already carries |
| **Slot** — a collection element/map value, or a **record field** | the slot holds a handle whose state is not readable through it | the `STATE` rides the **slot**: an element or field extracted from it types `.state`, and a rebind is checked for agreement (`TYPE_STATE_MISMATCH`) |

A slot is the position `STATE` rides rather than a binding: `List OF RES fs::File STATE Cursor` and `TYPE Holder { handle AS RES fs::File STATE Cursor }` both fold the clause into the element's or field's own type, which is what lets an extracted handle read `.state`. As at a binding, `T` must be a copyable, defaultable data type (`TYPE_STATE_INVALID`) — a resource is none of those, so a slot may not name one as its own state.

Bare therefore reads two ways: **"opaque"** at a parameter, and **"asserts none"** at a return or a binding. Bare never *strips* a `STATE` — at a return or a binding it is a claim that there is none, and the claim is checked:

```basic
RES a AS fs::File STATE Cur = fs::open("/tmp/x.txt", "w")
RES b AS fs::File           = a    ' error 2-203-0129: `b` is bare but its initializer
                               '   carries STATE `Cur`
```

The rule behind the asymmetry is **what the compiler can prove about the STATE type**, not escape and not ownership words:

> A resource's `STATE` type is fixed **once, at the binding that creates it**, and carries no runtime tag. Every later declaration that names a `STATE` is therefore checked against that fixed type and must agree (`TYPE_STATE_MISMATCH`) [[src/rules/table.rs:TYPE_STATE_MISMATCH]]. A **return** and a **binding** are positions where the declaration names a type, so each is checked directly — a bare one asserts "no state" and is rejected against a stateful resource, exactly as a mismatched concrete one is. A **parameter** is the single position that deliberately declines to name the type: bare there means *opaque* — "some state or none" — so that one close op serves every state its callers attach. Opacity is sound because it is **unobservable**: `x.state` is inaccessible through a bare parameter, so no code can read the payload under a type the compiler has not checked.

The asymmetry is thus about **knowledge, not lifetime**. `STATE` agreement holds independently of whether a resource escapes: the check happens wherever a type is named, and the one place no type is named is the one place nothing can be misread. A `thread::transfer` across the thread boundary is checked the same way (the plane names the `STATE`; `TYPE_STATE_MISMATCH` on disagreement, §16). See `./mfb spec architecture escape-analysis`.

Because a bare parameter is opaque rather than stateless, an opaque `STATE` may not be **narrowed** to a concrete one: a bare `f AS SfFile` parameter cannot be returned or bound as `RES … AS SfFile STATE FileInfo`, since the compiler knows only that `f` has *some* state and cannot discharge the claim that it is a `FileInfo` (`TYPE_STATE_OPAQUE_NARROWING`). Opacity propagates as opacity; only a producer that knows the type may name it.

The parameter row is what lets a close op accept a resource whatever state its owner attached — `FUNC close(db AS Db)` names no `STATE` and works for every `Db`, precisely because that alias never escapes to re-read `.state` under a new type.

**Attachment happens exactly once, at the owning binding.** A parameter only observes: a `p AS fs::File STATE Cursor` parameter given an argument that carries no `Cursor`, or that carries some other state type, is rejected (`TYPE_STATE_MISMATCH`) [[src/rules/table.rs:TYPE_STATE_MISMATCH]] rather than attaching or re-typing one. The payload carries no runtime type tag, so its type is fixed by the binding that created it and every later declaration must agree.

Consequently a binding that wants only the handle must still restate the `STATE` its initializer carries; there is no owner-side opt-out. That restatement is the price of the bare return's "no state" promise, which a later `STATE T` binding relies on when it attaches.

**Returning a resource that carries state.** A `FUNC` returns one by naming the `STATE` on its return, and the state the callee populated arrives intact at the caller's binding:

```basic
FUNC openTagged(path AS String) AS RES fs::File STATE Cursor
  RES f AS fs::File STATE Cursor = fs::openFile(path)
  f.state.pos = 42
  RETURN f                                    ' the Cursor rides the return
END FUNC

RES h AS fs::File STATE Cursor = openTagged(p)    ' adopts it — h.state.pos is 42
LET here = openTagged(p).state.pos            ' `.state` resolves from the call too
```

The caller's binding **adopts** the carried state rather than re-initializing it, so `h.state.pos` reads `42` and not `0`. A bare binding of that same call (`RES h AS fs::File = openTagged(p)`) is rejected: bare asserts "no state", and accepting it would let a function launder a carried state through a bare return into a caller that attaches a second one over it.

The `STATE` rides an exported signature, so this works across a package boundary exactly as it does in-package.

See `./mfb spec architecture escape-analysis` for the ownership/escape decision procedure this rule rests on.

**Resource unions.** A union whose every variant is a resource type is itself a resource — a *resource union* — and is `RES`-bound like any other resource:

```basic
UNION Stream            ' every variant is a resource → Stream is a resource
  fs::File
  tcp::Socket
END UNION

RES s AS Stream = fs::open("app.db", "read")   ' a File wraps into the union
MATCH s
  CASE fs::File(f)
    LET line = fs::readLine(f)
  CASE tcp::Socket(sock)
    LET data = tcp::read(sock, 1024)
END MATCH
' scope end → drop closes the active variant via its registered close op
```

Variants are spelled package-qualified. Built-in resources are package-qualified
identities (§17), and several packages deliberately share a bare name —
`tcp::Socket`, `udp::Socket` and `tls::Socket` are three distinct resource types —
so a bare `Socket` would not say which one.

A resource union owns exactly one resource at a time (the active variant), so it is atomic — a *choice* among resources, not a bundle. **Drop is tag-dispatched**: cleanup reads the union tag and calls the active variant's registered close op. Matching a resource union yields a reference to the active variant (the union retains ownership and closes it on drop). A union may **not mix** data and resource variants (`TYPE_MIXED_RESOURCE_UNION`). A value of any variant may be passed into a parameter that names the union (variant→union widening, §15.4) — a `File` into an `s AS Stream` parameter — but never the reverse: a `Stream` value is not accepted by a concrete `f AS fs::File` parameter, which is what keeps each variant's registered close op concrete-typed.

**A resource union may carry `STATE`.** Like a concrete stateful resource, a resource union takes a `STATE T` at its *use site* — a binding, parameter, or return (`RES s AS Stream STATE PendingState`) — never on the `UNION` declaration itself. The STATE type is **uniform**: it is the type named at the use site and is the same whichever variant is active, so nothing is read from the union tag to type `.state`. Agreement is checked exactly as for a concrete resource — a later binding, parameter, or return that names a disagreeing STATE type, or a `thread::transfer` plane that does, is rejected (`TYPE_STATE_MISMATCH`). The payload rides the **active variant's** resource record: it is default-initialized when the union is produced, adopted when the initializer already carries it, read and written in place through the union value or through a `… STATE …` parameter (a callee's write is visible to the owner, per this section), reached through a `MATCH`-extracted variant's `.state`, carried intact across a `thread::transfer`, and freed exactly once on drop **after** the tag-dispatched close. Per-variant STATE (a different type per variant) is **not** provided — a single uniform type is what keeps `.state` typable with no runtime tag, and it is the whole reason union STATE is sound.

To release a resource earlier than the end of its scope, or to observe a close failure, call the resource's explicit close operation (such as `fs::close(f)`). That operation consumes the handle and auto-propagates a close failure like any other call, so the close failure is directly observable. After an explicit close the name is statically moved. This does not imply removal of its cleanup entry: a later drop can invoke the close helper, whose already-closed result is a benign no-op, before reclaiming memory.
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_cleanup_call]]

**Binding invalidation, close, and memory reclamation are separate events.**
Static invalidation removes permission to read a name; close ends the underlying
resource's open state; owner-scope reclamation releases compiler-managed payloads
and, where ownership and resource kind permit, the record itself. An explicit
close does not reclaim the attached `STATE` or invalidate the storage needed by
other live views to observe the closed flag. Resource-specific close operations
can release their own native backing allocations.

The record need not survive until arena teardown: native cleanup can reclaim it
after its final valid view lifetime, while other records remain as tombstones.
An explicit close preserves registered drop-time reclamation for record-freeing
resource kinds even though the source name is statically invalid. A direct-name
rebind similarly does not erase the resource's existing cleanup obligation.
See `./mfb spec memory arenas` for the resource-kind and ownership predicates,
reclamation order, and retention cases.
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:resource_record_freed_at_drop]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:deactivate_moved_resource_arguments]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_block_reclaim]]

**A transferred handle is moved, not closed.** A successful `thread::transfer`
hands the resource to the receiving side and marks the sender's record **moved**.
Every sender view shares that moved state, including the caller's view when a
callee transfers a resource parameter. A later guarded operation reports
`ErrResourceMoved`, not `ErrResourceClosed`: the handle belongs to another thread.
A direct read of the transferred name on the success continuation is rejected at
compile time (`TYPE_USE_AFTER_MOVE`, §14.9); the runtime check protects the views
that check cannot connect, such as the caller's binding after a callee transfers
its resource parameter.
[[src/codegen/error/constants/error_constants.rs:RESOURCE_MOVED_BIT]]
[[src/codegen/resource/mod.rs:closed_or_moved_guard]]

### Transfer outcome and cleanup routing

A failed transfer does not consume the resource. Failure recovery and scope
unwinding are separate decisions:

| Edge | Resource state and cleanup consequence |
| --- | --- |
| Successful continuation | Sender views are moved; the receiver owns the transferred handle. Sender cleanup must not close it. Reclamation of eligible sender-side tombstone records can remain scheduled. |
| Inline `TRAP` failure arm | The resource remains in its pre-transfer state and cleanup scope. The handler may use it, close it, or retry. |
| Continuation after inline recovery | The resource is moved if transfer succeeded; it retains the failure handler's resulting state if recovery ran. Statically it is moved on some path, so a read of the transferred name here is `TYPE_USE_AFTER_MOVE` (§14.9). |
| Propagation or function-level `TRAP` | Transfer itself leaves the resource unchanged, but exiting scopes run their cleanup. A local resource whose cleanup scope was exited is not reusable in the handler; a parameter owned further out is not closed merely by leaving the callee. |
| Timeout before enqueue, including a full queue with `timeoutMs = 0` | `ErrTimeout`; no transfer, followed by the chosen error-routing rules above. |
| Cancellation or queue/thread closure before enqueue | No transfer; the queue operation fails and the same error-routing rules apply. Error selection follows `./mfb spec threading queue-semantics`. |

The write helper checks queue state and capacity before copying/enqueueing under
its lock. The resource copy marks the sender moved when the transfer commits.
Function-level trap routing separately invalidates locally owned resources whose
scopes it cleans up. This is why permission to reuse a resource in an **inline**
failure handler is not permission to reuse a local resource after unwinding.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_write_helper]]
[[src/codegen/memory/arena/builder_arena_transfer.rs:copy_resource_to_current_arena]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:retire_moved_resource_cleanup]]
[[src/ir/verify/resources.rs:check_resource_moves]]

A close that runs as part of an implicit lexical drop cannot inject an error into program flow, because a drop has no source-level result to route. If such a drop-close fails, the failure is emitted as diagnostic/audit metadata associated with the failed cleanup; it does not replace, wrap, or raise a source-level `Error`. Programs that must observe a close failure use the explicit close operation instead. Re-closing an already-closed handle during a drop is not such a failure — it is a benign no-op and is never reported as a drop-close failure. Nor is a handle `thread::transfer` moved to another thread: the scope drop recognizes the record's `moved|closed` flag and closes nothing, frees only the sender's tombstone where the binding owns the record, and records no failure — whether the transfer was the binding's own, a callee's transfer of the caller's handle (which the static move check cannot see, §14.9), or one under an inline `TRAP` that succeeded. A transfer that fails leaves the handle open and the sender's drop closes it, inline `TRAP` or not. Every drop path — a binding's, an owned-list drain, a resource union's, a discarded result's, the module instance's teardown — treats `ErrResourceClosed` and `ErrResourceMoved` from its close the same way. [[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_drop_close_outcome]] [[src/codegen/resource/cleanup/builder_resource_cleanup.rs:deactivate_closed_resource_arguments]]

Failure of an acquisition supplies no successful resource value for the source
binding. An inline handler that `RECOVER`s supplies the value that initializes
that binding; if the handler diverges, execution does not continue past the
declaration. This is the initialization rule of §8.4, not an implicit recovery
with a closed resource.

The implementation uses an **already-closed resource record** for internal
default resource temporaries in inline-`TRAP` lowering. This record makes cleanup
of the temporary safe; it does not mean the source acquisition succeeded or the
source binding was initialized. Its closed flag makes cleanup a no-op. Internal
slot initialization and source-level definite initialization are different facts
(handler-local definite initialization is §8.6a).
[[src/ir/lower.rs:lower_inline_trap]]
[[src/codegen/memory/value/builder_value_semantics.rs:emit_closed_resource_record]]

This rule does not change the built-in `Error` shape: A secondary close failure is not directly inspectable by ordinary source code through the built-in `Error` value.

Compiled cleanup metadata must preserve enough information for runtime and audit tooling to report a drop-close failure. Package audit output should identify cleanup regions that retain this failure metadata.

## 15.6 Resources in collections

A resource is owned by a **scope** — never by a binding or a collection. A `RES` binding, a resource parameter, and a collection slot (a `List` element or `Map` value) all hold **a reference to the one resource**. Copying a reference never duplicates the resource, and a collection slot is a reference, not a resource binding. Creating a view does not itself close the resource; cleanup closes it on scope exit if it has not already been closed or transferred out.

A resource appearing as a collection element carries the **`RES` ownership-axis marker**, exactly as a binding (`RES f`) or a return (`AS RES fs::File`) does. (A parameter carries no marker: it is a resource by its type.) The only spelling for a list of files is `List OF RES fs::File` (and `Map OF String TO RES fs::File` for a map value); a bare `List OF File` is rejected just like `LET f AS fs::File` (`TYPE_RESOURCE_REQUIRES_RES`), and `RES` on a non-resource element is rejected like `RES x AS Integer` (`TYPE_RES_REQUIRES_RESOURCE`). The marker is an ownership annotation only — the collection is still an ordinary copyable collection of references and owns nothing. A worker pool is the same shape: `List OF RES Thread OF Msg TO Out` holds references to parent thread handles, each closed once — its worker cancelled if still running — by the scope that owns it (§16).

By default the owning scope is the scope where the resource is produced. The single rule that governs collections is **ownership floats up**:

> Adding a reference to a resource to a collection migrates the resource's owning scope up to the collection's scope when that scope outlives the current owner. Up is the only direction a resource's scope moves: an inner binding, a callee, or a callee's return never lowers it. Among eligible containers reached by the routing analysis, cleanup floats to the **outermost** scope; a mere read or ordinary argument passing is not a routing edge. If a referencing collection escapes the function (it is `RETURN`ed), ownership moves out to the caller, exactly like `RETURN`ing the resource itself.

Consequences:

- A reference added to a **higher-scope** collection raises the owning scope to that collection's scope; the resource closes once when that outer scope exits, and every reference (the original binding and the collection elements) is within that scope, so none dangles.
- A reference added to a **same- or lower-scope** collection leaves ownership unchanged; the collection just holds a reference. So does one added to any collection when the resource is already owned further out — a handle a function was passed, added to the function's own collection.
- A binding whose ownership has floated to an outer scope becomes a plain **reference**: still usable, and it no longer closes at its own scope exit. It *may* still close, `RETURN`, or `thread::transfer` the resource — any holder may — with the corresponding close, escape, or moved-state handling preventing the old cleanup from closing an escaped resource.

`get` and `FOR EACH` of a resource element yield a **reference** to the one resource, like any other holder. It may be bound with `RES`, closed, `RETURN`ed, or transferred; the resource is still closed exactly once, by its assigned cleanup scope if it has not escaped or already been closed, with the closed flag making any later close a defined no-op. Collections of resources are ordinary copyable collections of references — no move-only or linearity — and the helpers that require a comparable element (`find`, `contains`, `replace`) remain unavailable because handles are not comparable, the same reason resources cannot be `Map` keys.

**A resource collection element may carry `STATE`.** The `RES` element (or map value) takes a uniform `STATE T` clause folded into the element type — `List OF RES fs::File STATE Cursor`, `Map OF K TO RES fs::File STATE Cursor`, and `List OF RES Stream STATE PendingState` for a resource union (§15, "A resource union may carry `STATE`"). The STATE rides the **element**, not the binding: the list itself is ordinary copyable data, and the uniform state type is what lets an extracted element type `.state`. `get` or `FOR EACH` of such an element yields a reference whose STATE type is the element's `T`, so binding it with `RES x AS fs::File STATE Cursor` and reading `x.state` is checked for agreement (`TYPE_STATE_MISMATCH`) exactly as for a concrete stateful resource. A `STATE` clause is only meaningful on a `RES` element; `List OF File STATE Cursor` (no `RES`) is a parse error, and the bare no-`RES` element is separately rejected by `TYPE_RESOURCE_REQUIRES_RES`. This mirrors the thread resource plane, which folds the same clause into its `RES` element (`Thread OF RES fs::File STATE Cursor TO Out`, §16). Per-variant STATE is not provided, for the same reason as at a binding: a single uniform type keeps `.state` typable with no runtime tag.

**A record field may carry `STATE` the same way.** `TYPE Holder { handle AS RES fs::File STATE Cursor }` folds the clause into the field's own type, and `h.handle.state` reads the payload — two dots, because the first reaches the field and the second reaches its `STATE`. The payload is the **same** one the owning `RES` binding sees: a field holds a reference to the one resource, and the `STATE` hangs off the resource record, not off the holder. Writing `f.state.pos = 42` through the owning binding and reading `h.handle.state.pos` through a *copy* of the record therefore reads `42`, not a default. As for an element, the clause is only meaningful on a `RES` field: `handle AS fs::File STATE Cursor` (no `RES`) is a parse error, and the agreement check on a rebind (`RES x AS fs::File STATE Other = h.handle`) is `TYPE_STATE_MISMATCH` exactly as it is for a concrete stateful resource.

**Writing `STATE` through a record-field chain is not spelled directly.** `h.handle.state.pos = 5` is not a form the language provides: the member-target assignment that `resource.state.field = v` uses is the one exception to "no member assignment" (§15), and it is anchored to a bare `RES` identifier. To write, bind the handle first — the rebind is an alias, not a second owner, so it costs nothing:

```basic
RES f AS fs::File STATE Cursor = h.handle   ' alias; no new close obligation
f.state.pos = 5                             ' write through the alias
io::print(toString(h.handle.state.pos))     ' reads 5 -- one payload
```

This is a language rule, not a limitation note: a `STATE` write names the resource it writes to, and a record field is a way to *reach* a resource rather than a way to *be* one.

A resource element placed into a collection may be any reference to the resource — a named `RES` binding or a temporary alike. The collection holds references to resources whose cleanup placement follows the container-routing rules above.

**Returning a resource collection transfers scope-ownership to the caller**, exactly as `AS RES fs::File` does for a single resource. A function returning `AS List OF RES fs::File` releases each referenced resource that one of its scopes owns — it does not close them — and the caller's binding scope takes each released resource, and no other, closing each once at its own exit. A returned element still owned further out (a handle the caller passed in) keeps its owner. The same holds for **any returned value that reaches a resource**, at any depth: a record with a `RES` field, a `List OF Holder`, a `List OF List OF Holder`, a `Map` of such records, a data-union variant carrying one, a recursive record — every resource the value reaches is released if the returning function owns it and taken by the caller's binding, and a call whose result is discarded closes the resources nobody took at the call statement. A record reaching two kinds of resource (a `File` field and a resource-union field) transfers both. Assigning such a call result to an existing container binding (`rows = make()`) adopts the same way, into that binding's own scope rather than the assignment's. [[src/codegen/cleanup/owned/builder_owned_cleanup.rs:call_assigned_owner_containers]] A resource that floated into the returned container but was removed from it before the `RETURN` is not transferred: it stays the returning function's, which closes it at its exit. [[src/codegen/resource/owner.rs:emit_release_returned_value]] [[src/codegen/resource/walk.rs:emit_for_each_resource]] (A bare `List OF File` return is rejected for the missing `RES` marker.) On an error exit *before* the return, the resources are still closed by the function's scope, because they ride its owned-list until the `RETURN` transfers it. A resource collection may also be passed to a function, where the callee receives references to its elements (and may close them, subject to the same shared closed flag). The resources must be added to the collection at or after the collection's own binding so the obligation rides the collection; violating this is rejected (`TYPE_RESOURCE_RETURN_ORDER`) rather than compiled. [[src/rules/table.rs:TYPE_RESOURCE_RETURN_ORDER]] A resource collection cannot cross a thread data plane: a `RES`-marked collection on a thread's **data** plane — its message, output, input, or a resource plane's `STATE` payload — is rejected with `TYPE_THREAD_RESOURCE_PLANE_REQUIRED`, which names the resource and points at the resource plane. [[src/rules/table.rs:TYPE_THREAD_RESOURCE_PLANE_REQUIRED]] A resource crosses a thread only on that plane, moved with `thread::transfer`/`thread::accept` (§16).

**A closure is a container too.** Capturing a resource in a `LAMBDA` — a `RES`
binding directly, or a value that reaches one — copies a reference into the
closure exactly as `collections::append` copies one into a `List`. The closure
never owns the resource: when it is held by a longer-lived scope the resource's
cleanup scope rises to that scope, and otherwise nothing changes. Each movement
of a closure follows the `List` rule for the same movement: a closure stored in
an outer binding or an outer collection floats the resources it captured there;
a `RETURN`ed closure releases the ones the function owns and the caller's binding
takes them; a call result that is discarded closes the ones nobody took; a store
into a module-level binding raises them to the module instance; a copy holds the
same references. A closure `RETURN`ed over a container declared after the
resource it carries is refused with `TYPE_RESOURCE_RETURN_ORDER`, as the same
container would be. A value a `List` may not hold — one that reaches a
`ThreadWorker` — may not be captured either (`TYPE_LAMBDA_CAPTURE_UNSUPPORTED`,
§14.4). [[src/ir/resource_escape.rs:scan_collection_expr]]
[[src/codegen/resource/closure_carry.rs:emit_closure_resource_op]]
A `FUNC` type does not say what its closure captured, so across a function
boundary the closure is asked: each capturing `LAMBDA`'s environment routine
walks the resources its captures reach (`./mfb spec memory closures`).

Removing the final stored reference does not close the resource or move its
cleanup scope inward. The cleanup obligation is retained independently of the
container's current contents, including after `removeAt`, replacement, or an
assignment of an empty collection. The scope's registered owned-list drains on
exit even if the collection no longer contains that resource.
[[src/codegen/cleanup/owned/builder_owned_cleanup.rs:emit_owned_list_push]]
[[src/codegen/cleanup/owned/builder_owned_cleanup.rs:emit_owned_list_drain]]

### Aggregate routing examples

These fragments assume `TYPE Holder { handle AS RES fs::File }`, ordinary file
producers `openFile`, and no explicit close or transfer unless shown.

| Shape | Cleanup/validity result |
| --- | --- |
| Outer `MUT h = Holder[seed]`; inner `{ RES f = openFile(path); LET inner = Holder[f]; h = inner }` | The copied record holds the same resource view. `f` floats to `h`'s scope and remains usable through `h.handle` after the inner block exits. |
| Outer `MUT h = Holder[seed]`; inner `{ RES f = openFile(path); h = WITH h { handle := f } }` | The replacement resource floats to the outer scope. Replacing the old field does not close `seed` or move its cleanup inward. |
| Outer `MUT xs AS List OF RES fs::File = []`; inner `{ RES f = openFile(path); xs = append(xs, f); xs = removeAt(xs, 0) }` | The now-empty list does not undo the float. The resource closes at the outer cleanup scope, not when removed or when the inner block exits. |
| `LET a = Holder[f]; LET b = a`; ordinary callee closes `a.handle`; caller uses `b.handle` | Both records refer to one identity. A guarded operation through `b` observes `ErrResourceClosed` when the invalidation was not diagnosed statically. |
| `forEach([Holder[f]], LAMBDA(h AS Holder) -> use(h.handle))` | The callback receives an aggregate carrying a resource view as an argument; it does not capture `f`. Argument passing alone does not move cleanup. The callback may still explicitly close or transfer the handle. |

Record constructors and `WITH` updates participate in the same per-function
routing analysis as collection insertion/copy edges; `WITH` conservatively
carries both the target's existing resource membership and its updates.
[[src/ir/resource_escape.rs:scan_collection_expr]]
[[src/codegen/resource/owner.rs:emit_resource_float]]

**A module-level binding is the outermost scope.** A resource stored into a
module-level `MUT` — directly, inside a record, at any depth of a list or map —
is owned by the **module instance** from that store on: the main program's
instance, or the fresh instance a worker thread runs (§16). It stays valid after
the storing function returns, and closes exactly once at **instance teardown**:
when the program ends — `main` returning, `EXIT PROGRAM`, or an untrapped error —
or when the worker's entry function returns. This holds whether the resource was
produced by the storing function (it floats to the global like any outer
collection, `./mfb spec architecture escape-analysis`) or arrived through a
parameter from a caller scope that exits first (the store itself raises it). An
explicit close or a `thread::transfer` through any view still applies, and the
teardown close is then the defined no-op. Removing the last global view —
`removeAt`, replacement, assigning an empty collection — does not close the
resource or move its cleanup inward. A resource stored into a worker's copy of a
global is that worker's; the parent's copy never holds it.
[[src/codegen/resource/module_owner.rs:emit_module_float]]
[[src/codegen/resource/module_owner.rs:emit_module_store_float]]
[[src/codegen/resource/module_owner.rs:lower_module_resource_drain]]

The float rules above are the source-level contract. The compiler implements them with a purely syntactic per-function **decision procedure** — which collection a binding floats to (outermost referencing scope, the special case that a returned collection declared before the resource forces a float at the same depth, fixpoint propagation along collection copy/append/nesting edges, the insertion-builtin set, and the declaration-order tiebreak) — that is specified in full by `./mfb spec architecture escape-analysis`. Programs depend only on the contract here, not on the procedure's mechanics.

## See Also

* ./mfb spec architecture escape-analysis — the static decision procedure (`Local`/`Float`) that places cleanups; the owner word decides which one runs
* ./mfb spec memory heap-values — the owner word and scope tokens
* ./mfb spec package resource-regions — how resources are encoded in the `.mfp` `RESOURCE_TABLE`
* ./mfb spec language threads — `thread::transfer`/`accept` resource plane
