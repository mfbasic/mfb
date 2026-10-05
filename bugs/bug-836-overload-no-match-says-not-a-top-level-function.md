# bug-836: a call that matches no user overload reports `SYMBOL_UNKNOWN_IDENTIFIER: Callable `f` is not a top-level function`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (misleading diagnostic)

Status: Open
Regression Test: tests/syntax/functions/func_overload_call_no_match_invalid (to add)

When a name has several `FUNC` declarations (an overload set) and a call matches none of
them exactly, the build fails with
`error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved / Callable `f` is not a top-level function.`
But `f` *is* a top-level function, declared right above the call. The message neither
says that no overload matched nor lists the candidates. That is what the
value-position analogue `TYPE_OVERLOAD_NO_MATCH` does ("Both name the declared
overloads"). Two common triggers both produce this message:

- omitting a defaulted argument in an overload set, which the spec says is not allowed;
- passing an integer literal where the only numeric overload takes `Byte`, which the
  spec says gets no literal coercion.

The single correct behavior: a resolution failure against a user overload set reports a
diagnostic that names the call's argument types and the declared overloads, for example
`TYPE_OVERLOAD_NO_MATCH` or a call-specific code. It does not claim the function does not
exist.

References:

- `src/docs/spec/language/06_functions.md`: "if no overload matches exactly, the call is
  a resolution error"; "within an overload set every parameter — including one declared
  with a default — must be supplied explicitly"; "an integer literal does not become
  `Byte` merely because an overload takes `Byte`".

## Failing Reproduction

```basic
IMPORT io
FUNC f(x AS Integer, y AS Integer = 1) AS Integer
  RETURN 1
END FUNC
FUNC f(x AS String) AS Integer
  RETURN 2
END FUNC
FUNC main() AS Nothing
  io::print(toString(f(1)))
END FUNC
```
- Observed: `error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved` / `Callable `f` is not a top-level function.`
- Expected: a no-matching-overload diagnostic naming `f(Integer)` and the two declarations.

Same message for `FUNC f(x AS Byte)` + `FUNC f(x AS String)` called as `f(1)`.
(Measured in /tmp/hardening/B01/t/fe31471 and fe28567.)

## Root Cause

The monomorphizer resolves user overloads to mangled names. When nothing matches, it
leaves the bare callee name `f`. That name is no longer a visible function symbol, so
`src/resolver/resolution.rs:resolve_callable` falls into the generic
`SYMBOL_UNKNOWN_IDENTIFIER` "is not a top-level function" branch.

## Goal

- An unmatched user-overload call yields an overload-specific diagnostic listing the
  candidates.

### Non-goals

- Resolution semantics stay as they are: exact match, no defaults inside overload sets.

## Validation Plan

- A syntax fixture with both triggers. Then the full suite. Existing goldens with this
  message (tests/syntax/functions/func_visibility_top_level_invalid,
  tests/syntax/collections/collections-cutover-invalid) must be inspected. Only lines that
  are overload no-match cases may change.

## Summary

This changes diagnostic quality only. The risk is in classifying which unresolved
callees are overload sets.
