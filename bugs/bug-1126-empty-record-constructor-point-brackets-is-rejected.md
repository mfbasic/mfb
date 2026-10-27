# bug-1126: `Point[]` (all-defaults record constructor) is rejected with an arity error

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Compiler (type checker)

Status: Open
Regression Test: —

bug-1021 (`19691be2c`) let a by-field record constructor name some fields and
give the rest their defaults, and documented on `mfb man types` that
"`Point[]` is the all-defaults `Point`". The empty form does not build:

```
TYPE Point
  x AS Integer
  y AS Integer
END TYPE

FUNC main() AS Nothing
  LET p = Point[]
END FUNC
```

→ `error[2-203-0024 TYPE_CONSTRUCTOR_ARITY_MISMATCH]: Constructor 'Point' has 0
argument(s), expected 2.` `Point[x := 5]` works (`y` is `0`).

**The correct behavior a fix produces:** `Point[]` builds and equals the
all-defaults `Point`, as the page says.

References:

- Found by the plan-125 B recheck (`types` topic), finding 2; probe
  `/tmp/plan-125-scratch/B-recheck/man-topic-types/proj_p3_defaults`.
