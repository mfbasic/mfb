# bug-801: a `List OF RES` stored inside another collection by a call is freed at its own scope exit

Last updated: 2026-10-03
Effort: small
Severity: HIGH
Class: Memory (use after free → SIGSEGV)

Status: Fixed (2026-10-03, bug-staging-3)
Regression Test: `tests/runtime/rt_res_list_kept_by_call.rs`; unit tests
`record_ownership::owned_list_ownership_tests::{a_collection_appended_into_another_is_kept,
a_callee_that_keeps_its_parameter_keeps_the_argument, a_recursive_callee_keeps_its_argument}`

Found while gating bug-755 in `bug-staging-4`: `rt-behavior/resources/bug643-collection-helpers-keep-caller-handles-rt`
segfaulted (exit 139) at `collections::get(collections::get(dwins, 0), 0)`.

## Reproduction (main `1242b4983`, its release binary)

```basic
IMPORT io
IMPORT fs
IMPORT collections

FUNC wrap(RES f AS fs::File) AS List OF List OF RES fs::File
  MUT out AS List OF List OF RES fs::File = []
  LET piece AS List OF RES fs::File = [f]
  out = collections::append(out, piece)
  RETURN out
END FUNC

FUNC main AS Integer
  RES d AS fs::File = fs::open("project.json", "read")
  LET w = wrap(d)
  LET inner = collections::get(w, 0)
  io::print("len=" & toString(len(inner)))
  RES pulled AS fs::File = collections::get(inner, 0)
  io::print("got")
  RETURN 0
END FUNC
```

Prints `len=96` (a freed, reused block), then SIGSEGV (`./build/nest.out; echo $?` → 139).
The same happens when `piece` is handed to a user `FUNC box(xs …)` that appends `xs`
and returns the outer list.

## Root cause

`record_ownership::owning_collections` frees a `RES`-marked collection's block at
scope exit only when nothing else can hold it. `CollectionUses::visit_value` treated
every bare-local call argument as a use ("an argument is a USE, not a second
owner"). That holds for `len(c)` and for the receiver of `collections::append(c, x)`,
but not for the appended ELEMENT when it is itself a collection:
`collections::append(out, piece)` stores `piece`'s block inside `out`. A user function
that appends its parameter keeps the caller's argument the same way.

bug-753's partial fix widened the fresh-block sources to `__collections_slice`
results, so the `piece` that `collections::window`'s body appends to its result hit
the same hole. That is how the bug643 fixture started to crash.

## Fix

`CollectionUses` gains a `kept` set, asked only of collections (a resource element
passed into `append` is the float the owning list exists for, and stays a use):

- every non-receiver argument of a `collections.` member is kept;
- for a callee with a body in the program, the caller's argument is kept when the
  callee's own walk lets that parameter escape or keeps it (memoized; a recursive
  callee answers "kept").

Spec: `memory/04_arenas.md` (the owned-collection conditions).

Verified: `rt_res_list_kept_by_call` fails 2/3 on main's binary and passes 3/3 with
the fix. `rt_res_list_from_call_leak`, `rt_scope_drop_leaks` (154),
`rt_debug_soak` (49), `rt_native_write_overload_call_argument`, `rt_res_rebind_alias`
and `rt_res_state_inplace_mutation` pass, and the `resources` + `collections`
acceptance fixtures pass (119).
