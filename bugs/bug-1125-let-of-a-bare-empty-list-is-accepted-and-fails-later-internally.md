# bug-1125: `LET xs = []` with no type is accepted, then fails later with an internal error

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Diagnostics (type inference)

Status: Open
Regression Test: —

`mfb man variable` and `mfb man types` say an empty list needs its type spelled.
The compiler accepts the declaration anyway, infers `List OF Unknown`, and only
fails when the list is used, with an internal error instead of a diagnostic:

```
IMPORT io
IMPORT collections

FUNC main() AS Nothing
  LET xs = []
  LET ys = collections::append(xs, 1)
  io::print(toString(len(ys)))
END FUNC
```

`mfb build` → `error: native collection list item must be Unknown, got Integer while lowering bind ys AS List OF Unknown`.

**The correct behavior a fix produces:** a bare `[]` with no annotation and no
expected type is rejected at the `LET` with a real diagnostic that asks for the
type (`LET xs AS List OF Integer = []`).

References:

- Found by the plan-125 A recheck (iteration 3, `variable` topic), finding 6;
  repro `/tmp/plan-125-scratch/A-recheck-iter3/man-topic-variable/proj_p6b`.
