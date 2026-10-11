# bug-1034: `RES n AS Integer STATE Cur` reports two errors; only `TYPE_STATE_REQUIRES_RESOURCE` should be reported

Last updated: 2026-10-10
Effort: small (<1h)
Severity: LOW
Class: Other (diagnostic)

Status: Not a bug (owner decision, 2026-10-10: both errors are valid; `Integer` is not a resource, so `RES` and `STATE` are each wrong on their own). Nothing to change.
Regression Test: (proposed) tests/syntax/resources/res-non-resource-with-state-one-error-invalid

A local `RES` binding whose type is not a resource and which carries a `STATE` clause is reported
twice on the same line: `TYPE_RES_REQUIRES_RESOURCE` (the `RES` keyword needs a resource type) and,
since bug-998, `TYPE_STATE_REQUIRES_RESOURCE` (the `STATE` clause needs a resource type).

**Owner decision (2026-10-10), final: keep both errors.** The owner first chose
`TYPE_STATE_REQUIRES_RESOURCE` alone, then, on seeing what `TYPE_RES_REQUIRES_RESOURCE` says, ruled
that both are valid. The text below is the report as filed.

**Correct behavior:** one error, `TYPE_STATE_REQUIRES_RESOURCE`, at the binding. Without a `STATE`
clause, `RES n AS Integer = 1` keeps reporting `TYPE_RES_REQUIRES_RESOURCE` as today.

## Failing Reproduction

Measured by the coordinator on 2026-10-10, macOS aarch64, `mfb build -ast -ir`, with batch 19's
staging compiler (`bug-staging-19` at 5580627c3, which has bug-998):

```
IMPORT io

TYPE Cur
  n AS Integer
END TYPE

FUNC main AS Integer
  RES n AS Integer STATE Cur = 1
  io::print(toString(n))
  RETURN 0
END FUNC
```

- Observed: two errors at line 8:
  `error[2-203-0083 TYPE_RES_REQUIRES_RESOURCE]: RES binds only resource types` — "binding `n` is
  declared `RES` but `Integer` is not a resource type; use `LET`/`MUT`." and
  `error[2-203-0145 TYPE_STATE_REQUIRES_RESOURCE]: STATE can only be attached to a resource` —
  "binding `n` attaches `STATE Cur` to `Integer`, which is not a resource; remove the STATE clause."
- On main at 3fa35d543 (before bug-998): only the first.
- Expected: only the second.

## Notes

- bug-998's agent (bs19-agent-a) left both in place because removing an existing diagnostic went
  beyond that bug's doc. The parked earlier attempt (bs18-agent-l) suppressed
  `TYPE_RES_REQUIRES_RESOURCE` here; that hunk was dropped, not reviewed.
- A user who follows the one remaining message literally ("remove the STATE clause") is left with
  `RES n AS Integer = 1` and then gets `TYPE_RES_REQUIRES_RESOURCE`. Consider wording the
  `TYPE_STATE_REQUIRES_RESOURCE` detail for a `RES` binding so it names both fixes (this is the
  coordinator's suggestion, not part of the owner's decision).
- Check the sibling positions for the same pair: a `RES` record field (`field AS RES Integer STATE
  Cur`, see bug-1031), a `RES` return, a top-level `RES` binding.
