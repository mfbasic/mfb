# bug-870: a return-type overload (`encoding::utf8Encode`) loses its expected type under an inline TRAP — `LET x AS List OF Integer = encoding::utf8Encode(s) TRAP(e)` is TYPE_OVERLOAD_AMBIGUOUS

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/encoding/utf8encode-return-overload-inline-trap-rt (to add)

`encoding::utf8Encode` is selected by the call's expected type (`List OF Byte` or
`List OF Integer`). An annotated `LET` supplies it. With an inline `TRAP` on the same
`LET` the expected type is not propagated, and the call is rejected as ambiguous for
**both** annotations. Inline TRAP on a total call is otherwise legal
(`LET b AS String = encoding::base64Encode(strings::toBytes("ab")) TRAP(e) … END TRAP`
builds and runs).

Correct behavior: the inline-TRAP form resolves the overload from the `LET` annotation,
exactly as the plain form does.

References:

- src/docs/spec/stdlib/08_encoding.md §Unicode transforms: "With an expected type of
  `List OF Byte` it returns the raw bytes; with `List OF Integer` it returns the identical
  `0..255` values … An unannotated call is the compile-time `TYPE_OVERLOAD_AMBIGUOUS`
  error (resolved in the monomorphizer by the call's expected type)."

## Failing Reproduction

`/tmp/hardening/B04/rto/src/main.mfb` (first version):

```
IMPORT io
IMPORT encoding

FUNC main() AS Nothing
  LET a AS List OF Integer = encoding::utf8Encode("A")
  io::print(toString(a))
  LET b AS List OF Integer = encoding::utf8Encode("B") TRAP(e)
    RECOVER []
  END TRAP
  io::print(toString(b))
END FUNC
```

```
/tmp/hardening/mfb build -q /tmp/hardening/B04/rto
```

- Observed: `rto/src/main.mfb:7 error[2-203-0101 TYPE_OVERLOAD_AMBIGUOUS]: return-type overload cannot be resolved without an expected type`. Line 5 (no TRAP) is accepted. Changing line 7's annotation to `List OF Byte` gives the same error.
- Expected: both lines compile.

## Root Cause

Hypothesis: the monomorphizer's expected-type propagation for an annotated `LET`
(bug-197's fix, "monomorph assign rhs expected type") looks at the binding's initializer
node directly. With an inline TRAP the initializer is wrapped in the TRAP expression node,
and the wrapper does not forward the expected type to its scrutinee. Confirm in the
monomorphizer's LET/assign expected-type path for the inline-TRAP node kind.

## Goal

- Both TRAP forms (`List OF Byte`, `List OF Integer`) compile and select the matching overload.

### Non-goals (must NOT change)

- An unannotated `LET x = encoding::utf8Encode(s) TRAP(e)` stays ambiguous.

## Blast Radius

- Every registry member with return-type overloads, under inline TRAP. Also check `MUT x AS T = … TRAP` and `x = … TRAP` assignment forms.
