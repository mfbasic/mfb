# bug-820: a Float-suffixed literal is accepted into a Fixed/Money slot and stored as its f64 bits

Last updated: 2026-10-04
Effort: medium (diagnostic change; needs the full suite)
Severity: MEDIUM
Class: Correctness (silent wrong value)

Status: Open
Regression Test: proposed `tests/syntax/types/float-suffixed-literal-into-fixed-invalid`
plus a MATCH-pattern case.

Found while fixing bug-816 (bug-staging-10).

## Failing Reproduction

```
IMPORT io
FUNC g(x AS Fixed) AS String
  RETURN toString(toInt(x * 100F))
END FUNC
FUNC main() AS Nothing
  LET f AS Fixed = 1.5f
  LET m AS Money = 5.0f
  io::print(toString(f = 1.5F) & " " & toString(m = 5.0m))
  io::print(g(1.5f))
END FUNC
```

Measured at 2bbf36113 and after bug-816 (`mfb build`, macOS arm64): prints
`FALSE FALSE`, then `g(1.5f)` raises ErrOverflow. The `-ir` shows
`"name": "f", "type": "Fixed", "value": { "kind": "const", "type": "Float", "value": "1.5" }`:
the slot holds the f64 bit pattern of 1.5. The same happens for a call argument,
a record field, a `RETURN`, a list element, and a `MATCH` pattern over a Fixed or
Money scrutinee (`CASE 1.5f` never matches). Unsuffixed literals are fine in
every context (they are lowered with the expected type).

## Root Cause

`ir/verify/compat.rs:expression_compatible` accepts any `Float`/`Integer`
constant into a `Fixed` or `Money` slot. That rule predates suffixes, when every
such constant was an unsuffixed literal that lowering had already retyped. Since
plan-28-B, `ir::lower::lower_expression_with_expected` keeps a suffixed literal's
own type ("the suffix always wins"), so a `Float` constant reaching a Fixed/Money
slot is now exactly a suffixed literal, and nothing converts it.

## Goal

Per `mfb spec language lexical-structure` ("`LET x AS Float = 2F` is a type
error"), `LET x AS Fixed = 1.5f` and `LET m AS Money = 5.0f` are type errors in
every slot kind, including `MATCH` patterns. No Float bits ever reach a Fixed or
Money carrier.

## Fix Design

Narrow the `Const` exemption in `expression_compatible` (and the negated-literal
one below it) to the forms lowering can still produce unretyped, after checking
every slot kind lowers its literal with an expected type. Confirm with an `-ir`
sweep that no existing fixture relies on the exemption, then run the full suite,
since this changes which programs compile.

## Blast Radius

Compile-time acceptance of suffixed literals in Fixed/Money slots (binding, call
argument, field, return, collection element, MATCH pattern), and decoded package
IR carrying such constants.
