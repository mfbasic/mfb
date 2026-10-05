# bug-935: `collections::append(xs, [f])` on a `List OF RES T` fails the build with an unlocated internal error ("native collection list item must be fs.File, got List OF fs.File")

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/resources/append-res-list-literal-rt (new; name suggested)

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
