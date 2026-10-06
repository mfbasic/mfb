# bug-933: a `RES` added to an outer list through `collections::take`/`drop`/`flatten` is closed at the inner scope exit; the outer list keeps a dead handle (reported as ErrResourceMoved)

Last updated: 2026-10-06
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/resources/float-through-repackaging-collections-rt; `ir::resource_escape::tests::{a_resource_floats_through_a_repackaging_member, an_element_read_put_into_a_container_floats_its_source, a_scalar_or_element_result_is_not_a_float_target}`

A resource produced in an inner scope and put into an outer-scope `List OF RES` floats to the
list's scope (language resource-management §15.6 rule 2: "Adding it to a collection whose scope is
further out moves it out"). That holds when the outermost call of the assigned expression is an
insertion builtin (`xs = collections::append(xs, f)`, also `append(take(xs, 10), f)`, `filter(...)`,
`reduce(...)`). It does **not** hold when the insertion is wrapped by a re-packaging member:
`xs = collections::take(collections::append(xs, f), 10)`, `collections::drop(collections::append(xs, f), 0)`,
`collections::flatten([xs, [f]])`. Then the inner scope closes `f` on exit (the process's open-fd count
drops back to the baseline) while `xs` still holds the handle, and the next `fs::readLine` through
`xs` fails. The failure also carries the wrong code: `ErrResourceMoved` (77030009, "moved to another
thread by thread::transfer"), whereas the same dead handle after an explicit `fs::close` reports
`ErrResourceClosed` (77030004).

The program is valid (it compiles with no diagnostic), so this is a silent lifetime bug: a handle the
program legitimately holds is dead. The fixture `resources/inline-trap-producer-float-rt` records that
the same early-close shape on a resource *union* (`http::Stream`) SIGSEGV'd reading STATE through the
closed record; this repro uses `fs::File`, which is guarded.

**Correct behavior:** the resource floats to `xs`'s scope regardless of the re-packaging call around the
insertion; after the inner block `f` is still open (fd count +1), the read succeeds (`hello,`), and
`xs`'s scope closes it exactly once. A handle closed by scope cleanup, if it is ever observed, reports
`ErrResourceClosed`, not `ErrResourceMoved`.

References:

- `src/docs/spec/language/15_resource-management.md` §15 rule 2 and §15.6 (float to the outer collection).
- `src/docs/spec/architecture/23_escape-analysis.md` "Scanning a collection-valued expression" (only
  insertion builtins are recursed; `_ -> ignore`) and "A resource that escapes by a route the scan does
  not model is still closed exactly once, by its owner" — exactly once holds, but too early.
- Related, distinct: bugs/completed/bug-643-collections-chunks-and-window-close-the-callers-resource-handles.md
  (introduced `call_result_holds_only_argument_resources`, the adoption skip that also misses this route).
- Found in hardening batch B12 (architecture/23_escape-analysis).

## Failing Reproduction

`/tmp/hardening/B12/data.txt` contains `hello`.

```
IMPORT io
IMPORT fs
IMPORT collections
FUNC fds() AS Integer
  RETURN len(fs::listDirectory("/dev/fd"))
END FUNC
FUNC rd(xs AS List OF RES fs::File) AS String
  MUT out AS String = ""
  FOR EACH h IN xs
    out = out & fs::readLine(h) & ","
  NEXT
  RETURN out
  TRAP(e)
    RETURN out & "ERR" & toString(e.code)
  END TRAP
END FUNC
FUNC main() AS Nothing
  LET base AS Integer = fds()
  MUT xs AS List OF RES fs::File = []
  IF TRUE THEN
    RES f AS fs::File = fs::openFile("/tmp/hardening/B12/data.txt")
    xs = collections::take(collections::append(xs, f), 10)
  END IF
  io::print("open after block=" & toString(fds() - base) & " len=" & toString(len(xs)))
  io::print(rd(xs))
END FUNC
```

`mfb build` + run (macOS aarch64, /tmp/hardening/mfb at HEAD):

- Observed:
  ```
  open after block=0 len=1
  ERR77030009
  ```
- Expected:
  ```
  open after block=1 len=1
  hello,
  ```

Measured matrix (only the assigned expression varies):

| `xs = …` | open after block | read |
| --- | --- | --- |
| `collections::take(collections::append(xs, f), 10)` | 0 | ERR77030009 ✗ |
| `collections::drop(collections::append(xs, f), 0)` | 0 | ERR77030009 ✗ |
| `collections::flatten([xs, [f]])` | 0 | ERR77030009 ✗ |
| `collections::append(collections::take(xs, 10), f)` | 1 | hello, ✓ |
| `collections::filter(collections::append(xs, f), LAMBDA(h AS fs::File) -> TRUE)` | 1 | hello, ✓ |
| `collections::reduce([f], xs, LAMBDA(acc …, h …) -> collections::append(acc, h))` | 1 | hello, ✓ |
| user `FUNC myAppend(xs, RES f) = collections::append(xs, f)`; `xs = myAppend(xs, f)` in a loop | 2 (2 iters) | ✓ |

Contrast (code): `RES f`; `LET xs = [f]`; a callee `fs::close(f)`; reading through `xs` reports
`ERR77030004` (ErrResourceClosed) — so only the scope-exit close leaves the "moved" code.

## Root Cause

Two layers both miss this route:

1. `src/ir/resource_escape.rs:scan_collection_expr` recurses only into `is_insertion_builtin` calls
   (`append prepend insert set mid removeAt filter reduce`); any other call is `_ -> ignore`. For
   `take(append(xs, f), 10)` the outer `take` is ignored, so neither `f` (element) nor `xs`
   (source collection) produces a routing into `xs`, and `f` is decided `Local` to the `IF` block.
2. The runtime owner-word raise that rescues the user-function case (`myAppend`, adopted through
   `call_assigned_owner_containers`) is skipped here on purpose:
   `src/codegen/cleanup/owned/builder_owned_cleanup.rs:call_result_holds_only_argument_resources`
   classifies `take`/`drop`/`flatten` as members that "only re-package their arguments' handles"
   (bug-643), so `adopts_from_call` is false and the assignment raises nothing. The block's static
   close then fires with the owner word still naming the block.

The ErrResourceMoved code suggests the scope-exit close path marks the record with the moved state
(hypothesis; the explicit `fs::close` path sets the closed state). Confirm by reading the resource
close emission that runs at scope exit versus `fs::close`.

## Goal

- Every case in the matrix prints `open after block=1` and `hello,`; the handle is closed once when
  `xs`'s scope exits (fd count back to baseline after the enclosing function returns).
- A scope-closed handle read through a surviving view reports `ErrResourceClosed` (77030004).

### Non-goals (must NOT change)

- bug-643's guarantee: `chunks`/`window` of a caller's resource list must not close the caller's
  handles (the fix must not re-enable adoption that double-owns argument handles).
- Same-scope `RES f; LET xs = [f]` stays `Local` (the worked-outcomes table in 23_escape-analysis).

## Blast Radius

- `resource_escape.rs:scan_collection_expr` — the `_ -> ignore` arm for non-insertion `collections`
  members whose result carries their argument's elements (`take`, `drop`, `flatten`, likely `mid`
  already handled, `distinct`, `chunks`/`window`, `zip`, `partition`, `merge`, `values`): fixed by this bug.
- `builder_owned_cleanup.rs:call_result_holds_only_argument_resources` — same list; must stay consistent.
- Not measured: `Map OF K TO RES V` through `collections::merge` / `removeKey`; resource unions through
  the same wrappers (fixture comment says the union variant SIGSEGVs).

## Resolution

`ir::resource_escape` now models every `collections` member whose result can
carry its arguments' values. A new `collections_carry(callee)` classifies a
member from its registry signature. `Container` means some implementation returns a
non-variable type that mentions a type variable a parameter also mentions (`take`,
`drop`, `flatten`, `merge`, `values`, `zip`, `transform`, …). `Element` means the
return is a bare type variable (`get`). `Nothing` covers everything else. It accepts
both spellings a call reaches the analysis in: native `collections.X` and
instantiated `#collections_X$…`. `take` is the second kind. The first cut keyed only
on `collections.`, and the fixture stayed red because of that. It now uses the
existing `builder_owned_cleanup::collections_call_member`.

- `scan_collection_expr` scans every argument of a `Container` call as an element
  position, which covers `flatten([xs, [f]])`'s nested handle.
- `scan_element` scans the arguments of an `Element` call
  (`xs = [collections::get(collections::append(xs, f), 0)]`). This was a sibling
  route the doc did not list, and it failed the same way.
- A `Container`/`Element` call bound as a whole scalar or `RES` binding routes
  nothing new. That keeps `n = collections::findIndex(ys, …)` and
  `RES g = collections::get(xs, 0)` (a borrow) from becoming float targets
  (`a_scalar_or_element_result_is_not_a_float_target`).

The codegen adoption skip (`call_result_holds_only_argument_resources`, bug-643) is
unchanged. Once the static decision is `Float`, the handle rides `xs`'s owned-list,
so no runtime owner raise is needed. bug-643's own fixture
(`bug643-collection-helpers-keep-caller-handles-rt`) is unchanged.

**Why the code was `ErrResourceMoved`.** It was a read of a freed record, not a
mis-set flag. `fs.File` is in `resource_record_freed_at_drop`, so the inner block's
close also freed the record. The debugger shows this on the repro built with main at
-O0: at `fs::close` the record at `x0` reads `01 00 … 03 00 …`; at the later
`fs::readLine` through `xs`, the same address reads `00 00 … 70 00 … e2 9c 84 c8 …`
(the entropy scrub). A random closed word with bit 1 set reports "moved". So goal 2
("a scope-closed handle read through a surviving view reports ErrResourceClosed")
cannot be met by a flag while the record is freed. It is met by having no surviving
view, which this fix provides for every route the scan now models.

Measured (macOS AArch64, `mfb build` + run of the fixture):

- Before (main at 468ff0f94): every row `open=0 ERR77030009`.
- After: `take/drop/flatten/merge/values/get open=1 hello,`,
  `loop open=2 hello,hello,`, `after open=0`. The handle is open after the block
  and closed by the end.
- `transform(append(xs, f), LAMBDA(h) -> h)` already worked before and still does.

Checks: `scripts/test-accept.sh` over the 110 fixtures that name a `RES` collection
or live in `resources/` gave only the 24 `.ast` mismatches bug-976 predicted (no
`build.log`/`.run`). `scripts/artifact-gate.sh … all`: 62 diffs, exactly bug-976's
61 `.ast` + 1 `.nir` (`app-mouse-surface`, its class 4); no `.ncode`/`.ncodesum`/`.nplan`
diff. `cargo test --bin mfb resource`: 217 passed.

Spec: `architecture/23_escape-analysis.md` ("Scanning a collection-valued
expression", new "Re-packaging members").

## Found while fixing (not this bug)

A `List OF RES T` local that is iterated (`FOR EACH h IN xs`) or read
(`collections::get(xs, 0)`) never frees its own block. `record_ownership`'s
`owning_collections` deliberately refuses such a list ("a FOR EACH iterable"
counts as holding the block elsewhere, and so does "no element is read back out"),
and accepts the bounded leak. Measured with main, `--debug`, 10 vs 100 calls of a
function that does `RES f = open; xs = append(xs, f); FOR EACH h IN xs …`: 10
unfreed allocations vs 100, 2400 B vs 24000 B live. One 232-byte list block per
call, shown by the debugger's `_mfb_arena_alloc`/`_mfb_arena_free` trace. When `f`
floats into the iterated list, the drain then reclaims nothing either, so the
`fs.File` record and its 16 KiB read buffer also stay (16,736 B per call). This is a
pre-existing design trade-off in the ownership proof, separate from this bug.
Reported to the coordinator for its own ticket.

## Golden impact

None. The fixture is new, and the artifact gate shows no diff outside bug-976's
base set.
