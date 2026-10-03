# Resource Escape Analysis

This topic specifies the **decision procedure** that assigns every `RES` binding
in a function an owner: either it is closed at its own producing scope, or its
close obligation **floats** up to a named collection binding. The source-level
*contract* this implements — when ownership floats, what operations remain available through a floated
binding, and how a returned resource collection transfers ownership — is owned
by `./mfb spec language resource-management` (§15.6). This page specifies the
**algorithm** a faithful reimplementer must reproduce, byte-for-byte.

## Result

The analysis produces, per `RES` binding name, one of three decisions
[[src/ir/resource_escape.rs:ResOwner]]:

| Owner | Meaning |
| --- | --- |
| `Local` | Closed at its own producing scope. The per-scope static cleanup is already correct; no float. |
| `FloatBlocked(C)` | A returned owning container cannot accept the resource at its production point because of declaration order; IR verification reports `TYPE_RESOURCE_RETURN_ORDER`. |
| `Float(C)` | Ownership floats to collection binding `C`'s scope. The obligation is drained from `C`'s scope's runtime owned-list, and transferred to the caller when `C` is `RETURN`ed. |

A floated binding does not carry its own scope-close obligation. The owner
table describes cleanup placement, not a prohibition on using a resource alias
to close, return, or transfer the handle. Absent bindings default to `Local`.
[[src/ir/resource_escape.rs:FunctionEscape]]

## HIR analysis and IR ownership metadata

The analysis walks the **concrete HIR**. It tracks `RES` bindings, declaration
depth/order, and container-valued expression shapes. The return-order gate also
uses declared container types and the supplied set of records with `RES` fields;
constructor initializers identify inferred record bindings. [[src/ir/resource_escape.rs:analyze_function]]

It is consumed by **IR lowering**, which records the result per function as
`resource_owners` — carried into the IR (and serialized into `.mfp` packages)
so the resource-ownership rules run on the typed IR, on both the source and
package paths. The escape decisions are what let the IR semantic verifier
recognize `RES` destination bindings without re-deriving the escape decision.
The direct-name move check invalidates the source of `RES b = a` even when `a`
is a resource parameter or `FOR EACH` element. This static invalidation is
separate from native cleanup placement; see the initializer table in
`./mfb spec language resource-management`.
[[src/ir/verify/resources.rs:check_resource_moves]]

```text
resource_owners = <escape analysis of the function> -> owners
```

## Unbound temporaries are named first

The analysis sees only named `RES` bindings, so before it runs IR lowering names
every resource that is produced straight into an element position: each call in a
list or map-literal value, a record-constructor argument, a `WITH` update, or an
element argument of a `collections::` insertion builtin, whose type is a resource
(a built-in one — a parent `Thread` included — a project `RESOURCE`, or a
`RES`-returning function's result), becomes a synthesized
`RES $res_tmpN AS T = <call>` placed immediately before its statement, and the
element becomes `$res_tmpN`. From there it floats exactly like a source binding
in that position. [[src/ir/resource_hoist.rs:hoist_resource_temporaries]] A call
that hands back a pointer to a resource another binding already owns —
`collections::get`/`getOr` of a `RES` element, or a `tcp`/`udp`/`tls::poll`
borrow — is no producer and is never named: a named temporary would float the
borrowed handle into the container, whose scope would then close the owner's
handle. [[src/codegen/engine/value/builder_values.rs:target_returns_borrowed_resource]]

The rewrite never reorders evaluation: a statement's element positions are
visited in evaluation order, and hoisting stops at the first earlier expression
that may have an effect and was not itself hoisted. A call it cannot type, or
whose type is not known to be a resource, is left in place.

There is a single implementation of the analyzer; it is invoked,
not copy-pasted. Lowering stores the decisions in IR; verification and native cleanup
lowering consume that metadata. [[src/ir/lower.rs:lower_function]] [[src/ir/resource_escape.rs:analyze_function]]

This analysis routes cleanup through the container edges listed below; merely
reading a handle or passing an ordinary argument does not float it. Any holder
may close, return, or transfer it (§15), so the syntactic scan is not a complete
account of where a resource can go. The single-close guarantee is carried by
three layers:

1. **Static hand-off where it is syntactically visible.** Returning a `RES`
   binding, a resource union, or a `List OF RES` deactivates that scope's cleanup,
   so the escaping scope emits no close at all.
2. **Runtime pointer identity for what the scan cannot see.** At scope exit a
   cleanup whose record pointer equals the value escaping the scope skips both the
   close and the reclaim.
3. **A closed/moved flag making a second close a defined no-op.** Every resource
   record carries it at offset 16, and both the built-in helpers and every native
   `LINK` thunk test it before acting, reporting `ErrResourceClosed` rather than
   acting on a dead handle.

So this pass computes where ownership *floats*; layers 2 and 3 ensure a resource
that escapes by a route the scan does not model is still closed exactly once
rather than twice or never.

## Walk: building the routing facts

`analyze_function` records every binding's declaration depth and order, then
walks the body collecting *routing facts*. [[src/ir/resource_escape.rs:Analyzer]]

- **`RES` parameters** are declared at depth 0 and entered as resources owned at
  function-entry depth.
- The body is walked at depth 0; each nested block (`IF` then/else, `MATCH`
  case, `FOR`/`FOR EACH`/`WHILE`/`DO UNTIL` body) increments depth by 1.
- **The trap body is walked at depth 1.** [[src/ir/resource_escape.rs:analyze_function]]

`declare(name, depth)` records, on first sight only, the binding's
`decl_depth` and a monotonically increasing `decl_order` index. Declaration
order is the deterministic tiebreak used by the float target selection.
[[src/ir/resource_escape.rs:declare]]

A **routing** is "a collection value carrying resource pointers flows into a
target", where target is a variable (`LET`/`MUT` bind, assignment) or `Returned`
(`RETURN <expr>`). [[src/ir/resource_escape.rs:Routing]] Each routing records:

- `res_elems` — `RES`-binding names inserted **directly** as elements here;
- `src_collections` — collection bindings whose contents also flow into the
  target (copy / `append(C, …)` / nesting).

### Scanning a collection-valued expression

```text
scan_collection_expr(expr):
  Identifier(name)        -> if name is a RES binding: ignore (a bare resource is
                             a move, not a collection); else it is a collection
                             copy: push name into src_collections
  ListLiteral(vs)         -> scan_element on each v
  Constructor(args)       -> scan_element on each argument
  WithUpdate(target, updates)
                          -> scan_collection_expr(target), then scan_element on each update
  MapLiteral{entries}     -> scan_element on each value (keys ignored)
  Call(f, args) if is_insertion_builtin(f):
                             arg[0] is the collection being updated -> recurse
                               scan_collection_expr
                             arg[1..] are elements -> scan_element
  Trapped{expression,handler}
                          -> scan_collection_expr(expression), and
                             scan_collection_expr on each RECOVER value in handler
  _                       -> ignore
```

[[src/ir/resource_escape.rs:scan_collection_expr]]

An inline `TRAP` is *unwrapped*, not ignored: both of its arms produce the same
target, so the guarded expression and each handler `RECOVER` value flow into it
alike. Both arms contribute routing facts.

`scan_element` treats a `RES`-identifier as a direct insertion (push to
`res_elems`); anything else falls through to `scan_collection_expr` so a nested
list/map contributes its own reachable resources. [[src/ir/resource_escape.rs:scan_element]]
A bare `RETURN f` or `LET g = f` of a resource produces **no** routing, so it
never floats (it is an ordinary move).

### Insertion-builtin set

A call counts as a collection insertion when, after mapping qualified
`collections.*` names back to the bare op via `native_member_bare`, the bare name
is one of: [[src/ir/resource_escape.rs:is_insertion_builtin]]

```text
append  prepend  insert  set  mid  removeAt  filter  reduce
```

The qualified→bare mapping ensures a *freed* bare name in user code is never
mistaken for a collection insertion (only the `collections::` qualified op
counts). [[src/codegen/registry/mod.rs:native_bare_target]] For an insertion
call, argument 0 is the collection flowing into the result and arguments 1.. are
candidate element insertions.

## Solve: fixpoint membership, then per-resource owner

`solve` first identifies **returned collections**: the `src_collections` of any
routing whose target is `Returned`. [[src/ir/resource_escape.rs:solve]]

It then computes, to a **fixpoint**, `membership[C]` = the set of resources
reachable from collection binding `C`, propagating along every routing edge:

```text
repeat until no change:
  for each routing R:
    incoming = R.res_elems ∪ ⋃{ membership[s] : s ∈ R.src_collections }
    if R.target = Var(name):  membership[name] ∪= incoming
    ; Returned routings are not accumulated here — the returned-collection set
    ; is gathered once, before the fixpoint, from every Returned routing
```

This closes membership over container copy/append/nesting edges (`List`/`Map`
literals plus insertion builtins) and over **record** edges:
a record `Constructor` argument and a `WITH` update are element positions, and a
`WITH`'s target flows into the result exactly as insertion argument 0 does. A
record is a container in the same sense a collection is — a value that holds
handle pointers and whose binding has a scope — so a resource placed into one
floats to that record's binding and is drained there.

Two asymmetries with collections are worth stating, because both are load-bearing:

* **A record's type does not reveal whether it can own a resource.**
  `List OF RES File` says so structurally; `Named("Holder")` does not. The
  ordering gate below therefore consults a set of record types carrying a `RES`
  field, threaded in from the type table. Without it the gate cannot see the
  container and degrades to `Local` — a silent double close.
* **An inferred record binding still has a knowable type**, because a
  constructor names it. `LET h = Holder[…]` carries no `AS Holder` annotation,
  so the declared-type map (which records only explicit annotations) misses it;
  the type is recovered from the initializer instead.

For each `RES` binding it then chooses an owner in two ordered phases, using the
resource's own `res_depth` and `res_order`:

**Phase 1 — returned-container-before-resource forces a float.** Among returned
containers that contain the resource, consider only those declared **strictly
before** the resource (`decl_order(C) < res_order`); a container whose order
`>= res_order` is skipped, because the container must be live before the
resource so its owned-list exists when the resource is produced.

A `RES` **parameter** is exempt from the blocked-order diagnostic: the caller owns
it and this function never produces it, so there is no production point for the
hazard to attach to. (A consequence for records: a resource the callee
*produces* cannot be returned inside a record at all, because a record — unlike
an empty `List` — cannot be constructed before its handle exists. Taking the
handle as a `RES` parameter is the shape that works.) This is the
special rule that **forces a Float even at the same scope depth**: the
obligation rides the collection's owned-list and transfers to the caller on
`RETURN`, instead of closing here. Pick the candidate with minimum
`(depth, order)`. [[src/ir/resource_escape.rs:solve]]

**Phase 2 — otherwise float to a strictly-outer collection.** Only if Phase 1
found nothing: among all collections containing the resource, keep only those
declared at a **strictly outer** scope (`decl_depth(C) < res_depth`); same-or-
inner scopes do not float. Pick the **outermost** — minimum `decl_depth`, then
minimum `decl_order` as the deterministic tiebreak. [[src/ir/resource_escape.rs:solve]]

If neither phase finds a target, a non-parameter resource reaching a returned
resource-owning container is `FloatBlocked(C)`; otherwise it is `Local`.
`ir::verify` rejects `FloatBlocked` with `TYPE_RESOURCE_RETURN_ORDER`.
[[src/ir/verify/mod.rs:collect_diagnostics_with]]

A `RES` binding whose initializer is a borrowed pointer — the calls above, or a
record's `RES` field read — skips both phases and is always `Local`: it holds no
close obligation, so it has none to float, and codegen binds it as a non-owning
alias. It stays in the owner table, so every ownership rule still treats it as a
`RES` name. [[src/ir/resource_escape.rs:initializer_is_borrowed]]

```text
choose_owner(r):
  if initializer_is_borrowed(r): return Local
  best = none
  # Phase 1: returned collection declared before r (same depth allowed)
  for C in returned_collections where r ∈ membership[C] and order(C) < order(r):
    best = min by (depth(C), order(C))
  # Phase 2: only if Phase 1 empty
  if best is none:
    for C where r ∈ membership[C] and depth(C) < depth(r):
      best = min by (depth(C), order(C))
  if best exists: return Float(best)
  if r is not a RES parameter:
    for C in returned_collections where r ∈ membership[C]:
      if C has a RES-marked collection type or a record type with RES fields:
        return FloatBlocked(C)
  return Local
```

In both phases the running-best comparison keeps the current candidate when its
`(depth, order)` is `<=` the new one, so equal keys never replace an earlier
pick — declaration order is the final, deterministic tiebreak.

A parameter declared at entry cannot
float to a later same-depth container merely because that container is returned.

## Worked outcomes

| Pattern | Result | Why |
| --- | --- | --- |
| `RES f; LET xs = [f]` (same scope) | `f → Local` | same depth, `xs` not returned, not before-and-returned |
| `MUT xs=[]; WHILE { RES f; xs=append(xs,f) }` | `f → Float(xs)` | `xs` strictly outer (Phase 2) |
| `MUT xs=[]; RES f; xs=append(xs,f); RETURN xs` | `f → Float(xs)` | `xs` returned and declared before `f` (Phase 1, same depth) |
| `RES f; RETURN f` | `f → Local` | bare resource return is a move, no routing |
| outer `ys`; inner `{ RES f; xs=[f]; ys=xs }` | `f → Float(ys)` | membership reaches `ys`, the outermost (Phase 2) |

[[src/ir/resource_escape.rs:tests]]

## See Also

* ./mfb spec language resource-management — the source ownership/move/float contract (§15.6) this procedure implements
* ./mfb spec package resource-regions — how resource lifetime is (and is not) encoded in the Binary Representation
* ./mfb spec language memory-semantics — the ownership-tree model and scope-drop frees
* ./mfb spec architecture ir — where `analyze_function` runs during lowering
