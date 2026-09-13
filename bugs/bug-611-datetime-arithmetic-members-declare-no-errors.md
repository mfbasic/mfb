# bug-611: `datetime` arithmetic members declare no errors, and which inputs raise is unmeasured

Last updated: 2026-09-13
Effort: small (<1h)
Severity: LOW
Class: Documentation (registry error declaration)

Status: Open
Regression Test: none yet — see Phase 1

Found while fixing bug-520 (S8). Every `datetime` descriptor declared `errors: vec![]`.
bug-520 probed and filled in the members that raise on bad arguments or out-of-range
host instants: `date`, `time`, `toIso(dt, digits)`, `parse`, `parseIso`, `fixedOffset`
(both), `localOffset`, `offsetAt`, `toLocal`, `inZone` and `civil`. The rest still
list nothing, and nobody has measured which of them can raise.

## Failing Reproduction

```
grep -l 'errors: vec!\[\]' src/codegen/builtins/datetime/func_*.rs
```

The members most likely to raise are the ones doing checked `Integer` arithmetic on
caller values, which fails `ErrOverflow` (`77050010`):

- `add`, `subtract`, `plus`, `minus`, `negate`, `between`
- `addDays`, `addMonths`, `startOfDay` (via `civil`: `ErrOverflow`, and for a Local
  zone `ErrInvalidArgument`)
- `toMillis`, `toNanos`, `fromMillis`, `instant`, `duration`

Each is a **guess** until probed. For example, `civil` with a UTC year of `3·10^14`
raises 77050010 (measured in bug-520), so `addDays` from such a value very likely
does too.

## Fix Design

For each member: read the body, probe the boundary inputs (Integer max/min, huge
day/month counts, a Local zone far from the epoch), and list exactly the codes seen
in its descriptor `errors`, the way bug-520's S8 table did. Descriptor `errors` feed
the `.ir` goldens; regenerate and inspect.

## Phases

### Phase 1 — probe and declare

- [ ] Probe each member above and record its raising inputs and codes here.
- [ ] Fill in `errors` for every member that raises; leave `vec![]` only where the probe
      shows none.

Acceptance: `mfb man datetime <member>` shows an Errors table for every member that
raises, and nothing else changes.
Commit: —
