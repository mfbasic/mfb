# bug-935: `collections::append(xs, [f])` on a `List OF RES T` fails the build with an unlocated internal error ("native collection list item must be fs.File, got List OF fs.File")

Last updated: 2026-10-06
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/resources/append-res-list-literal-rt

The list form of `collections::append` ("Return a list with one element, or every element of another
list") works for a `List OF RES fs::File` when the second argument is a **named** list
(`LET one AS List OF RES fs::File = [f]` then `append(xs, one)` prints `1`), and works for ordinary
lists with an inline literal (`append(xsInt, [2, 3])` prints `3`). With a `RES` list and an **inline
literal** second argument, the program passes the shape pass and IR verification but codegen aborts
the build with a message that has no rule code, no file and no line:

```
error: native collection list item must be fs.File, got List OF fs.File while lowering bind ys AS List OF RES fs.File
```

**Correct behavior:** `append(xs, [f])` compiles and behaves exactly like the named-list form (`len`
is 1; `f` floats into the result like any insertion).

References:

- `mfb man collections append` (the list form).
- `src/docs/spec/architecture/22_type-inference.md` "Call arguments — the parameter type IS the expected
  type" (rule 3: an ambiguous generic position yields no expected type) and "Structural Assignability"
  (both predicates peel `RES` before comparing).
- Related, distinct: bug-882 (same internal message family, `List OF Unknown` from `LET x = []`).
- Found in hardening batch B12 (architecture/22_type-inference, 23_escape-analysis).

## Failing Reproduction

```
IMPORT io
IMPORT fs
IMPORT collections
FUNC main() AS Nothing
  RES f AS fs::File = fs::openFile("/tmp/hardening/B12/data.txt")
  LET xs AS List OF RES fs::File = []
  LET ys AS List OF RES fs::File = collections::append(xs, [f])
  io::print(toString(len(ys)))
END FUNC
```

- Observed (`mfb build`): exit 1, the single line
  `error: native collection list item must be fs.File, got List OF fs.File while lowering bind ys AS List OF RES fs.File`.
- Expected: builds; prints `1`.

Contrast: replacing line 7 with `LET one AS List OF RES fs::File = [f]` +
`LET ys AS List OF RES fs::File = collections::append(xs, one)` builds and prints `1`. The same
internal error appears for `xs = collections::append(xs, [f])` inside a `WHILE`.

## Root Cause

`mfb build --ir` shows the lowered call:

```
{"kind": "call", "type": "List OF RES fs.File", "target": "collections.append",
 "args": [{"kind": "local", "name": "xs"},
          {"kind": "list", "type": "List OF fs.File", "values": [{"kind": "local", "name": "f"}]}]}
```

The inline literal is synthesized bottom-up (no agreed expected type at the generic position, rule 3)
from `f : fs.File`, so it is `List OF fs.File` — without the `RES` marker the receiving list carries.
`src/codegen/builtins/collections/gen_mutate.rs:collection_argument_as_list_slot` then decides
"whole list vs single item" with a raw `&item.type_ == list_type` (`List OF fs.File` vs
`List OF RES fs.File`), which fails, falls to the single-item check `item.type_ != *element_type`, and
returns the internal `Err`. The `at_start && item.type_ == list.type_` prepend guard just above has the
same RES-blind comparison. Shape/IR verification accept the call because their `compatible` peels `RES`.

## Goal

- The literal form builds and prints `1`; codegen's list-vs-item discrimination compares with `RES`
  peeled (or lowering gives the literal the receiving list's `List OF RES T` type).
- An argument codegen genuinely cannot lower is a located, coded diagnostic, never this bare `Err`.

### Non-goals (must NOT change)

- `append(xsInt, [2, 3])` and the named-list RES form keep working.
- `prepend(xs, <list>)` keeps its "expects a single item" rejection (but reached for the RES case too).

## Blast Radius

- `gen_mutate.rs:collection_argument_as_list_slot` — fixed by this bug.
- `gen_mutate.rs` prepend guard `item.type_ == list.type_` — same RES-blind equality; latent.
- Any other codegen-side `ParameterType ==` between an argument literal and a `List OF RES` slot — not
  surveyed.

## Resolution

New `codegen::engine::types::typed_same_list_ignoring_res(a, b)` returns true when
`a` and `b` are both `List`s whose elements are equal after `RES` is peeled
(`typed_list_element_type` already peels it). Every codegen site that tells "a whole
list" from "one element" by comparing an argument's type with the receiving list's
now uses it, so it agrees with the front end's `compatible`:

- `gen_mutate.rs:collection_argument_as_list_slot`, the failing site. It is shared
  by `append`, `insert` and `set`.
- `gen_mutate.rs:lower_collection_end_insert`'s prepend guard. It was latent;
  `prepend(xs, [f])` is still refused by the front end (`TYPE_UNKNOWN_VALUE` after
  the argument mismatch, unchanged, probe `/tmp/g935c`).
- `func_insert.rs`'s single-item guard. It was latent: `insert` takes `T` only, so
  the front end refuses a list.
- `builder_inplace_assign.rs`, the bulk in-place `append` arm, at both its commit
  test and its sublist check. `zs = collections::append(zs, [g])` in a loop now
  takes the in-place arm, as the named-list form does. Before, the commit test
  declined and the value path hit the same error.

The `Err` text itself remains for genuinely ill-typed input. It is unreachable from a
program the front end accepts, because codegen and `compatible` now agree on this
question. Locating every remaining internal codegen `Err` is not this bug's change.

Measured (macOS AArch64):

- Before (main at 468ff0f94): `mfb build` of the repro gives
  `error: native collection list item must be fs.File, got List OF fs.File while lowering bind ys …`,
  exit 1.
- After: the fixture prints `bind len=1 hello,`, `loop len=2 hello,hello,` and
  `pair len=3 world,hello,hello,`, exit 0.
- "Exactly like the named-list form": the same program with each literal bound
  first (`LET one AS List OF RES fs::File = [f]`, …) prints the same lines. Under
  `--debug` both report `alloc_calls 54`, `free_calls 43`, `live_bytes 33952`. The
  11 unfreed blocks belong to both forms: they are the iterated-`List OF RES` block
  leak recorded in bug-933's "Found while fixing".

Checks: `scripts/artifact-gate.sh … all` gave 62 diffs, exactly bug-976's base set
(61 `.ast` + `app-mouse-surface` `.nir`), with no `.ncode`/`.ncodesum`/`.nplan` diff.

## Golden impact

None. Only the new fixture.
