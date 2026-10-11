# bug-1022: a template call (`collections::merge`, any user `FUNC f OF T`) rejects a union variant where its `T` is the union, though the builtin collection members now accept it

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (language surface; follows owner decisions on bug-988/989)

Status: Open
Regression Test: (proposed) tests/rt-behavior/types/union-variant-template-call-rt and a syntax fixture for the reverse orders

bug-988 and bug-989 (owner decisions, 2026-10-10) settled the union rules for the builtin
collection members: a member fits a union bucket (it is wrapped; a collection of members is
converted), a union never fits a member bucket, and the collection argument fixes `T`. Template
functions go through monomorphization's own unifier, which was not changed, so they still
reject every variant-for-union call.

**Correct behavior:** a template call follows the same rules as a builtin generic call.
- `T` first bound from a collection argument is fixed by it (`both(shapes, squares)`: `T` is
  `Shape`, the `List OF Square` is converted; `both(squares, shapes)` stays an error).
- `T` first bound from a bare value may be rebound to the union a later argument brings
  (`pick(aSquare, shapes)`: `T` is `Shape`, the `Square` is wrapped), as `reduce` does.
- The reverse direction stays a compile error in every form.

## Failing Reproduction

Measured by the coordinator on 2026-10-10, main 2f0052332 (`/tmp/main-dbg-target/release/mfb`).
`Shape` is `UNION Shape  Square  Circle`.

```
FUNC pick OF T(x AS T, xs AS List OF T) AS Integer
  RETURN len(xs)
END FUNC

FUNC both OF T(a AS List OF T, b AS List OF T) AS Integer
  RETURN len(a) + len(b)
END FUNC
```

| call | result |
|---|---|
| `collections::merge(shapeMap, squareMap, TRUE)` | `TYPE_CALL_ARGUMENT_MISMATCH`: "cannot infer template arguments from `Map OF String TO Square`" |
| `pick(Square[side := 2.0], shapes)` | `TYPE_CALL_ARGUMENT_MISMATCH`: "cannot infer template arguments from `List OF Shape`" |
| `both(shapes, squares)` | `TYPE_CALL_ARGUMENT_MISMATCH`: "cannot infer template arguments from `List OF Square`" |
| control: `collections::append(shapes, squares)` (builtin) | builds; prints `2` |

Expected: the three template calls build; `merge` and `both` convert the variant collection,
`pick` wraps the `Square`. `both(squares, shapes)`, `merge(squareMap, shapeMap, TRUE)` and
`pick(aShape, squares)` stay `TYPE_CALL_ARGUMENT_MISMATCH`.

## Related

- bug-988, bug-989, bug-990 (the builtin side, fixed). Reported by bs18-agent-j; reproduced by
  the coordinator.
