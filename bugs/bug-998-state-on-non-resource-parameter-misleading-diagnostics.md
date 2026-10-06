# bug-998: `STATE` on a non-resource parameter is not rejected where it is declared; the user sees unrelated `TYPE_UNKNOWN_VALUE` / `TYPE_STATE_MISMATCH` errors instead

Last updated: 2026-10-06
Effort: small (<1h)
Severity: LOW
Class: Other (misleading diagnostic)

Status: Open
Regression Test: (proposed) tests/syntax/resources/param-state-on-non-resource-invalid

A `STATE` clause is only meaningful on a resource (`mfb man types`: a resource carries "its own
state with a trailing STATE S"). On a parameter whose type is not a resource, nothing rejects
the clause at the declaration. Instead the parameter gets a type no expression can use, and
the user sees errors that do not name the cause.

**Owner decision (2026-10-06): add a dedicated rule**, `TYPE_STATE_REQUIRES_RESOURCE` (an error,
code assigned in the registry), reported once, located at the parameter's `STATE` clause:
"STATE can only be attached to a resource". Register it in the diagnostics spec (rule-codes and the
error-codes Constant Registry) in the same change, per `.ai/compiler.md`. Do not reuse
`TYPE_STATE_INVALID`: that code means "STATE must be a copyable, defaultable data type".

The question as filed:

**Correct behavior (proposed):** one located error at the parameter, saying that `STATE` needs
a resource type. The recommendation is to reuse `TYPE_STATE_INVALID`, the existing code for an
invalid `STATE` use (bug-975 reuses it for `t.state.x` on a worker without `STATE`), rather
than adding a new rule. **Owner decision needed:** reuse `TYPE_STATE_INVALID`, or add a
dedicated rule.

## Failing Reproduction

Measured by the coordinator on 2026-10-06. "main" is 468ff0f94 (`/tmp/main-dbg-target/release/mfb`),
where a parameter needed the `RES` spelling. "bs17-agent-e" is bug-976's compiler, where `RES` on
a parameter is gone and `x AS Integer STATE Cur` parses. The result is the same on both.

```
IMPORT io

TYPE Cur
  n AS Integer
END TYPE

FUNC f(x AS Integer STATE Cur) AS Integer     ' main: FUNC f(RES x AS Integer STATE Cur)
  RETURN x + 1
END FUNC

FUNC main AS Integer
  io::print(toString(f(41)))
  RETURN 0
END FUNC
```

- With the call: `st/src/main.mfb:12 error[2-203-0129 TYPE_STATE_MISMATCH] … Argument 1 for 'f'
  is declared 'STATE Cur' but carries no STATE; a parameter cannot attach one — declare
  'STATE Cur' on the owning binding.` That advice is impossible to follow for an `Integer`.
- Without the call: `st/src/main.mfb:8 error[2-203-0043 TYPE_UNKNOWN_VALUE]: … RETURN value does
  not have a known type.` on `RETURN x + 1`.

Expected: a single error at line 7 (the parameter) saying that `STATE` requires a resource type.

## Related

- Found by bs17-agent-e while fixing bug-976 (`RES` removed from parameter lists). That change
  makes the `x AS Integer STATE Cur` spelling parse, so the gap is easier to reach than before.
  The diagnostics themselves are unchanged from main.
