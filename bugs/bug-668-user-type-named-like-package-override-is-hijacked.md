# bug-668: a user type named like a built-in package's override type is routed to that package's `toString`

Last updated: 2026-09-20
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (to add) `tests/syntax/general/tostring_user_type_named_color_invalid`

`toString(c)` where `c` is a user `TYPE Color` (with no user `toString`
override) is accepted by the type checker and then fails the build with
`error: NIR call target '#color_toString' does not resolve`. The same happens
for any user type whose bare name equals a registered package override's type
(`Url`, `Float2`, `Integer3`, …).

**The correct behavior:** a user type that the built-in rejects and that no
user override covers is a `TYPE_CALL_ARGUMENT_MISMATCH` at the call, exactly
as for a user type with any other name. A package override applies only to
that package's own type.

References:

- Found while landing plan-140-C (`toString(ENUM)`): a user `ENUM Color` was
  hijacked the same way; plan-140-C fixed the enum case (the built-in is
  authoritative) and recorded this as Correction C-C2.
- `src/docs/spec/language/18_builtin-functions.md` §18.3 — override
  resolution.

## Failing Reproduction

```
IMPORT io

TYPE Color
  r AS Integer
END TYPE

FUNC main AS Integer
  LET c AS Color = Color[1]
  io::print(toString(c))
  RETURN 0
END FUNC
```

- Observed (main at b6a10efbc): `error: NIR call target '#color_toString' does
  not resolve`.
- Expected: `TYPE_CALL_ARGUMENT_MISMATCH` on line 9, the same diagnostic a
  `TYPE Colour` gets.

## Root Cause

`registry::general_override_target` (`src/codegen/registry/mod.rs`) matches an
argument type against each override's descriptor `arg_type` by its BARE
spelling (`o.arg_type == spelled`) as well as the package-qualified one
(bug-480 Phase 4b). A user `Color` spells `Color`, so it matches the `color`
package's row. `ir::shape`'s general-call check (`shape.rs`, "A
package-provided override may accept what the built-in rejects") then accepts
the call, and `ir::lower`'s `package_override` routes it to `#color_toString`,
which exists only when `color` is imported.

## Goal

- The reproduction reports `TYPE_CALL_ARGUMENT_MISMATCH`; `toString(color::…)`,
  `toString(net::Url)` and the vector renderers keep working.

### Non-goals (must NOT change)

- Override dispatch for the packages' own (qualified) types.
- Do not "fix" it by renaming the user type in a test.

## Blast Radius

- `general_override_target`'s bare-spelling arm — fixed by this bug (it should
  match only the package-qualified identity, or a bare spelling only when that
  bare name resolves to the package's own type).
- `ir/shape.rs`, `ir/verify/compat.rs`, `ir/lower.rs` (typing and routing) —
  consumers of the same predicate; fixed through it.
- Enum arguments — already immune since plan-140-C (the built-in accepts them
  before any override is consulted).

## Phases

### Phase 1 — failing test + audit

- [ ] Add the reproduction as a syntax fixture; confirm today's build error.
- [ ] Find why bug-480 Phase 4b kept the bare comparison (which callers still
      pass a bare built-in spelling) before removing it.

Commit: —

### Phase 2 — the fix

- [ ] Restrict the match to the package's own type identity.

Commit: —

### Phase 3 — full validation

- [ ] `bash scripts/artifact-gate.sh target/release/mfb all` and
      `scripts/test-accept.sh target/debug/mfb target/accept-actual` → green.

Commit: —

## Summary

A name-keyed match on a type that should be identity-keyed. The risk is the
bare-spelling callers bug-480 left in place; the audit in Phase 1 settles that
first.
