# bug-1039: a template call whose argument is a direct imported-package call infers `T` as `Unknown`

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (proposed) an rt-behavior fixture with the project below, printing `a11`

`ident(pA::make())`, where `ident OF T(x AS T) AS T` is a user template and `pA::make()`
returns the package's exported type, does not infer `T`. The build fails with three
errors, one naming the instantiation `ident$Unknown`. Binding the call to a typed `LET`
first and passing the binding works.

**Correct behavior:** `T` is `pA.Thing`; the program prints `a11`.

## Failing Reproduction

Measured 2026-10-10 with main's compiler (`/tmp/main-dbg-target/release/mfb`, main at
a236fdd44), `mfb build -q .` in `/tmp/b911/infer`.

Package `pA`:
```
EXPORT TYPE Thing
  n AS Integer
END TYPE
EXPORT FUNC make() AS Thing
  RETURN Thing[11]
END FUNC
```
`src/a.mfb`:
```
FUNC ident OF T(x AS T) AS T
  RETURN x
END FUNC
FUNC fromA() AS String
  RETURN "a"
END FUNC
```
`src/z.mfb`:
```
IMPORT pA
FUNC fromZ() AS Integer
  LET t AS pA::Thing = ident(pA::make())
  RETURN t.n
END FUNC
```
`src/main.mfb` prints `fromA() & toString(fromZ())`.

- Observed:
  ```
  ./src/a.mfb:2 error[2-203-0043 TYPE_UNKNOWN_VALUE]: … RETURN value does not have a known type.
  ./src/z.mfb:3 error[2-203-0043 TYPE_UNKNOWN_VALUE]: … Initializer for binding `t` does not have a known type.
  ./src/a.mfb:1 error[2-203-0033 TYPE_FUNC_REQUIRES_RETURN_TYPE]: … FUNC `ident$Unknown` must declare an `AS` return type.
  ```
- Expected: builds; prints `a11`.

## What is known

Not traced. Whether the two-file layout matters was not tested. Related: bug-1022
(template calls and union variants).

Found by the bug-911 agent (batch 20).
