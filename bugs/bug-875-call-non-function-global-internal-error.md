# bug-875: calling a non-function top-level binding is not `SYMBOL_NOT_CALLABLE` — statement form dies in the linker

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-error/... (new) — name suggested

`a(3)` where `a` is a top-level `LET a = 1` is not reported as `SYMBOL_NOT_CALLABLE`. As a bare
expression statement the build fails deep in native emission with
`error: native code internal relocation target 'a' is not defined` (no rule, no location); as an
initializer (`LET q AS Integer = a(3)`) it is only the generic `TYPE_UNKNOWN_VALUE` "Initializer for
binding `q` does not have a known type", which never says why.

Correct behavior: `error[2-201-0012 SYMBOL_NOT_CALLABLE]` "... `a` is not callable", as the same
call through a *local* binding already reports.

References:

- src/docs/spec/diagnostics/01_rule-codes.md registry row `2-201-0012 SYMBOL_NOT_CALLABLE` "symbol cannot be called".
- Found during B05 diagnostics sampling.

## Failing Reproduction

```
LET a = 1
FUNC main() AS Nothing
  a(3)
END FUNC
```

- Observed: `error: native code internal relocation target 'a' is not defined` (build rc=1).
- Expected: `main.mfb:3 error[2-201-0012 SYMBOL_NOT_CALLABLE]: symbol cannot be called`.

Contrast (measured, same program shape): `LET b = 2` local then `LET y = b(2)` →
`SYMBOL_NOT_CALLABLE` "Local binding or parameter `b` is not callable."; `math::pi()` →
`SYMBOL_NOT_CALLABLE` "Package constant `math.pi` is not callable."

## Root Cause

`src/ir/verify/calls.rs:check_call_arity` emits `SYMBOL_NOT_CALLABLE` for a package constant and
for a *local* of known non-FUNC type. For a name that is neither a local nor a declared function it
only handles a global of `FUNC` type (bug-198) and otherwise `return`s silently — a global of any
other type falls through, and codegen later emits a direct call to a symbol `a` that does not exist.

## Goal

- A call whose target is a top-level binding of a known non-FUNC type is `SYMBOL_NOT_CALLABLE`, in statement and expression position.

### Non-goals (must NOT change)

- Calling a top-level binding of FUNC type stays legal (bug-198).

## Blast Radius

- `check_call_arity`'s `self.globals.get(target)` arm — the fix site.
