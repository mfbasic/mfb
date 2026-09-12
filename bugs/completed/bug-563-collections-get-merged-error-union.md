# bug-563: `collections::get`'s two overloads declare a merged error union, and the map form carries the list form's parameter prose

Last updated: 2026-09-12
Effort: small-to-medium (the audit is the work, as it was for bug-553)
Severity: MEDIUM (documentation correctness; same class as bug-553)
Class: Registry metadata / documentation correctness

Status: **FIXED** (`ceeefcf24`) for `get`. The package census below found two
more instances and one renderer gap; each is recorded with its evidence.
Regression Test: `collections_get_declares_errors_per_overload_not_merged`
(`src/codegen/builtins/tests/collections.rs`).

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

## Outcome

`get` fixed in `ceeefcf24`: list overload `["ErrIndexOutOfRange"]`, map overload
`["ErrNotFound"]`, and the map key has its own description.

Verified `get` has no other raise route before narrowing — neither
`lower_list_get` nor `lower_map_get` reaches plan-17's float observation
boundary, and `materialize_owned_element` raises nothing. That check mattered:
see `set` below, where it changes the answer.

Artifact gate **1428 / 1594 / 2003, 0 diffs** — narrowing these declarations
moves no emitted code. Note what that zero also says: **no fixture anywhere
TRAPs a map `get` expecting `ErrIndexOutOfRange`**, so the containment is real
but the coverage behind it is thin.

## The package census — do not stop at `get`

Every multi-overload member in `collections`, each derived from its lowering by
enumerating `raise_error("collections.<member>", …)` plus the helpers each arm
calls:

| member | verdict |
|---|---|
| `get` | **was wrong, fixed** — two shape-branched paths, one error each |
| `find` | **HONEST, left alone** — both overloads are list forms taking a `start`, so `ErrIndexOutOfRange` and `ErrNotFound` are both reachable on both |
| `findLastIndex` | **HONEST, left alone** — same reason, one shared helper raising both |
| `set` | **wrong, BLOCKED** — see below |
| `sum` | **wrong, BLOCKED** — see below |
| `append`, `contains`, `getOr` | declare no errors; not in scope here |

### `set` — wrong, and the obvious fix is the dangerous one

The map branch (`func_set.rs:271`) is remove-key + build-singleton + merge. It
contains **no raise at all**, and neither `lower_map_remove_key` nor
`lower_collection_values` raises. So its declared `["ErrIndexOutOfRange"]` names
an error it cannot raise.

**But `errors: vec![]` would be a MISCOMPILE.** Both `set` overloads call
`observe_float_vr` on the key and the value (plan-17 observation boundary), and
that reaches `raise_error_bare("ErrFloatNaN")` / `ErrFloatInf` /
`ErrFloatOverflow` via `emit_float_result_check_fp`. So map `set` is **not**
infallible — declaring it so would let `inline_builtin_is_infallible` delete a
live handler, which is exactly the dead-handler miscompile that hit
`strings::left`/`right`/`padLeft`/`padRight`.

This is bug-553's documented trap in its purest form: *a raise gated on a Rust
`if` over a lowering parameter, reached through a helper no call site names.*

The right declaration for `set` therefore depends on a question this bug cannot
settle: **do plan-17 observation-boundary raises belong in a member's declared
`errors`?** They are systematically undeclared across the whole tree today, and
`raise_error_bare`'s own declaration check is a `debug_assert!` that never runs
in release, so nothing has ever caught it. Answering it one way rewrites the
declarations of every member taking a `Float`; answering it the other way makes
the current silence correct and needs saying out loud. Recorded, not guessed.

### `sum` — the Float overload cannot raise what it declares

All three overloads declare `["ErrOverflow"]`. Derived from the arms
(`func_sum.rs:214-231`):

- `Integer` -> `emit_checked_integer_add` -> `raise_error_bare("ErrOverflow")`. **Correct.**
- `Fixed` -> same helper. **Correct.**
- `Float` -> a plain `abi::float_add_d` with **no check and no raise**. **Wrong.**

Blocked for the same reason as `set`, and it carries a sharper question with it:
a `Float` sum that overflows returns **Inf**, and nothing re-checks it —
`float_arith_node` only re-observes `Binary`/`Unary` NIR nodes, and a builtin
`Call` is neither. So a non-finite `Float` can escape the plan-17 boundary
through `collections::sum`. That is a **correctness** question, not a
documentation one, and it should be settled before `sum`'s declaration is
touched in either direction.

### A renderer gap this exposed — filed separately

`mfb man collections get`'s **Parameters** table renders only overload 1
(`value AS List OF T`, `index AS Integer`), while the Synopsis lists both
signatures. So the corrected map-key description is right in the descriptor and
**invisible on the page**: a reader sees overload 2 in the synopsis and then a
parameter table describing overload 1's types and prose as if they were its own.

bug-558 gave the Errors table an overload column; the Parameters table never got
the equivalent. Not fixed here — this bug's non-goals forbid touching the
renderer, and it affects every multi-overload member's page, so it wants its own
change and its own containment evidence.
