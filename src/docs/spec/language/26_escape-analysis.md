# Resource Escape Analysis

This topic specifies the **decision procedure** that assigns every `RES` binding
in a function an owner: either it is closed at its own producing scope, or its
close obligation **floats** up to a named collection binding. The source-level
*contract* this implements — when ownership floats, what a floated binding may no
longer do, and how a returned resource collection transfers ownership — is owned
by `./mfb spec language resource-management` (§15.6). This page specifies the
**algorithm** a faithful reimplementer must reproduce, byte-for-byte.

## Result

The analysis produces, per `RES` binding name, one of two owners
[[src/escape.rs:ResOwner]]:

| Owner | Meaning |
| --- | --- |
| `Local` | Closed at its own producing scope. The per-scope static cleanup is already correct; no float. |
| `Float(C)` | Ownership floats to collection binding `C`'s scope. The obligation is drained from `C`'s scope's runtime owned-list, and transferred to the caller when `C` is `RETURN`ed. |

A floated binding becomes borrow-only: it may not close, `RETURN`, or
`thread::transfer` (`floats()` reports this; absent bindings are `Local`).
[[src/escape.rs:FunctionEscape]]

## Purely syntactic, run twice, must agree

The analysis is **purely syntactic over the AST**. It depends only on which
local names are `RES` bindings, their declaration depth/order, and the shape of
collection-valued expressions — never on inferred types. [[src/escape.rs:analyze_function]]

It is run **independently** by two consumers, which must compute identical
results:

- the type checker, before checking a function body (`current_resource_owners`);
- IR lowering, recorded per function as `resource_owners`.

```text
syntaxcheck.rs: self.current_resource_owners = escape::analyze_function(function)
ir.rs:        resource_owners: escape::analyze_function(function).owners().clone()
```

Both call the **same** `escape::analyze_function`; there is a single
implementation in `src/escape.rs`. (CORRECTION to a common belief: the
`is_insertion_builtin` helper and the analyzer are *not* copy-pasted into
`syntaxcheck.rs` or `ir.rs` — those files only invoke `escape::analyze_function`.
The unrelated `native_member_bare` match for `append|prepend|insert|set` at
`src/syntaxcheck.rs` is a different check at a different call site, not a
replication of this set.) [[src/syntaxcheck/mod.rs:check_function]] [[src/ir/lower.rs:lower_function]]

Soundness rests on the borrow rule (`TYPE_RESOURCE_BORROW_INVALIDATE`,
§15.6): a borrowed resource cannot escape a callee, so a resource enters a
collection only inside the function that owns it, by direct insertion of a
`RES`-binding identifier — which is exactly what the syntactic scan detects.

## Walk: building the routing facts

`analyze_function` records every binding's declaration depth and order, then
walks the body collecting *routing facts*. [[src/escape.rs:Analyzer]]

- **`RES` parameters** are declared at depth 0 and entered as resources owned at
  function-entry depth.
- The body is walked at depth 0; each nested block (`IF` then/else, `MATCH`
  case, `FOR`/`FOR EACH`/`WHILE`/`DO UNTIL` body) increments depth by 1.
- **The trap body is walked at depth 1.** [[src/escape.rs:analyze_function]]

`declare(name, depth)` records, on first sight only, the binding's
`decl_depth` and a monotonically increasing `decl_order` index. Declaration
order is the deterministic tiebreak used by the float target selection.
[[src/escape.rs:declare]]

A **routing** is "a collection value carrying resource borrows flows into a
target", where target is a variable (`LET`/`MUT` bind, assignment) or `Returned`
(`RETURN <expr>`). [[src/escape.rs:Routing]] Each routing records:

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
  MapLiteral{entries}     -> scan_element on each value (keys ignored)
  Call(f, args) if is_insertion_builtin(f):
                             arg[0] is the collection being updated -> recurse
                               scan_collection_expr
                             arg[1..] are elements -> scan_element
  _                       -> ignore
```

[[src/escape.rs:scan_collection_expr]]

`scan_element` treats a `RES`-identifier as a direct insertion (push to
`res_elems`); anything else falls through to `scan_collection_expr` so a nested
list/map contributes its own reachable resources. [[src/escape.rs:scan_element]]
A bare `RETURN f` or `LET g = f` of a resource produces **no** routing, so it
never floats (it is an ordinary move).

### Insertion-builtin set

A call counts as a collection insertion when, after mapping qualified
`collections.*` names back to the bare op via `native_member_bare`, the bare name
is one of: [[src/escape.rs:is_insertion_builtin]]

```text
append  prepend  insert  set  mid  removeAt  filter  reduce
```

The qualified→bare mapping ensures a *freed* bare name in user code is never
mistaken for a collection insertion (only the `collections::` qualified op
counts). [[src/builtins/collections.rs:native_member_bare]] For an insertion
call, argument 0 is the collection flowing into the result and arguments 1.. are
candidate element insertions.

## Solve: fixpoint membership, then per-resource owner

`solve` first identifies **returned collections**: the `src_collections` of any
routing whose target is `Returned`. [[src/escape.rs:solve]]

It then computes, to a **fixpoint**, `membership[C]` = the set of resources
reachable from collection binding `C`, propagating along every routing edge:

```text
repeat until no change:
  for each routing R:
    incoming = R.res_elems ∪ ⋃{ membership[s] : s ∈ R.src_collections }
    if R.target = Var(name):  membership[name] ∪= incoming
    if R.target = Returned:   returned ∪= incoming
```

This closes membership over collection copy/append/nesting edges (`List`/`Map`
literals plus insertion builtins).

For each `RES` binding it then chooses an owner in two ordered phases, using the
resource's own `res_depth` and `res_order`:

**Phase 1 — returned-collection-before-resource forces a float.** Among returned
collections that contain the resource, consider only those declared **strictly
before** the resource (`decl_order(C) < res_order`); a collection whose order
`>= res_order` is skipped, because the collection must be live before the
resource so its owned-list exists when the resource is produced. This is the
special rule that **forces a Float even at the same scope depth**: the
obligation rides the collection's owned-list and transfers to the caller on
`RETURN`, instead of closing here. Pick the candidate with minimum
`(depth, order)`. [[src/escape.rs:solve]]

**Phase 2 — otherwise float to a strictly-outer collection.** Only if Phase 1
found nothing: among all collections containing the resource, keep only those
declared at a **strictly outer** scope (`decl_depth(C) < res_depth`); same-or-
inner scopes do not float. Pick the **outermost** — minimum `decl_depth`, then
minimum `decl_order` as the deterministic tiebreak. [[src/escape.rs:solve]]

If neither phase finds a target, the owner is `Local`.

```text
choose_owner(r):
  best = none
  # Phase 1: returned collection declared before r (same depth allowed)
  for C in returned_collections where r ∈ membership[C] and order(C) < order(r):
    best = min by (depth(C), order(C))
  # Phase 2: only if Phase 1 empty
  if best is none:
    for C where r ∈ membership[C] and depth(C) < depth(r):
      best = min by (depth(C), order(C))
  owner(r) = best ? Float(best) : Local
```

In both phases the running-best comparison keeps the current candidate when its
`(depth, order)` is `<=` the new one, so equal keys never replace an earlier
pick — declaration order is the final, deterministic tiebreak.

## Worked outcomes

| Pattern | Result | Why |
| --- | --- | --- |
| `RES f; LET xs = [f]` (same scope) | `f → Local` | same depth, `xs` not returned, not before-and-returned |
| `MUT xs=[]; WHILE { RES f; xs=append(xs,f) }` | `f → Float(xs)` | `xs` strictly outer (Phase 2) |
| `MUT xs=[]; RES f; xs=append(xs,f); RETURN xs` | `f → Float(xs)` | `xs` returned and declared before `f` (Phase 1, same depth) |
| `RES f; RETURN f` | `f → Local` | bare resource return is a move, no routing |
| outer `ys`; inner `{ RES f; xs=[f]; ys=xs }` | `f → Float(ys)` | membership reaches `ys`, the outermost (Phase 2) |

[[src/escape.rs:tests]]

## See Also

* ./mfb spec language resource-management — the source ownership/move/float contract (§15.6) this procedure implements
* ./mfb spec package resource-regions — how resource lifetime is (and is not) encoded in the Binary Representation
* ./mfb spec language memory-semantics — the ownership-tree model and scope-drop frees
* ./mfb spec architecture ir — where `analyze_function` runs during lowering
