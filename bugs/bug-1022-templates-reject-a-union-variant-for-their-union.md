# bug-1022: a template call (`collections::merge`, any user `FUNC f OF T`) rejects a union variant where its `T` is the union, though the builtin collection members now accept it

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (language surface; follows owner decisions on bug-988/989)

Status: Open
Regression Test: (proposed) tests/rt-behavior/types/union-variant-template-call-rt and a syntax fixture for the reverse orders

## Cause (coordinator, bug-staging-24)

- OWNER RULING 2026-10-18: template calls follow the SAME rules as builtin generic calls. The principle (owner): the final destination is the bucket: a value is fitted into the declared type it ends up in (a collection argument's element type for `append`-style members; the accumulator the callback declares for `reduce`, which is not an exception but the same rule); a member fits a union bucket (wrapped; a collection of members is converted), a union never fits a member bucket, and `T` is never widened to make a call fit.
- Applied to templates: a collection argument fixes `T` in ANY position (so `addTo(aShape, squares)` is an error and `addTo(aSquare, shapes)` / `addAll(shapes, squares)` work); an empty `[]` fixes nothing and `T` comes from the other arguments (`addAll([], shapes)` keeps working, as `user-generic-unknown-refine-rt` / bug-442 pins).
- Where: the template-instantiation inference in `src/monomorph/` (its own unifier, unchanged by bug-988/989). bs24-agent-h measured that the builtin binder as coded binds left-to-right, which differs from the principle only when a bare value precedes a collection (no builtin has that shape) and lacks the empty-list refinement.
- Fix: ONE binding rule both use: collection arguments bind `T` first, empty collections bind nothing, then bare values must fit (wrapped into a union `T`); templates call it instead of their own unifier. The doc's examples above that contradict this (`pick(aShape, squares)` vs `addTo`) follow the principle: the collection decides.
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
