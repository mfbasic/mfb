# bug-820: a Float-suffixed literal is accepted into a Fixed/Money slot and stored as its f64 bits

Last updated: 2026-10-04
Effort: medium (diagnostic change; needs the full suite)
Severity: MEDIUM
Class: Correctness (silent wrong value)

Status: Fixed
Regression Test: `tests/rt-behavior/conversions/float-suffixed-literal-into-fixed`,
`tests/syntax/types/float-suffixed-literal-into-money-invalid`,
`tests/syntax/types/float-suffixed-literal-fixed-range-invalid`

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

(Superseded for `Fixed` — see Resolution: the spec makes a `Float` literal in a
`Fixed` slot a conversion, not a type error. The `Money` half stands.)

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

## Resolution

RED fixtures in the commit before the fix; fix in the commit that archives this
document.

**Decision, from the spec.** The Goal above (a type error in every slot) was
not what the spec says for `Fixed`. `mfb spec architecture type-inference`,
"Literal Coercion", states it directly: "a Float `2f` into a `Fixed` slot still
coerces via the `E=Fixed ∧ A=Float ∧ expr=Number` rule" (written with the
suffixes, plan-28-B §4.3). Lexical structure's "`LET x AS Float = 2F` is a type
error" is the other direction (no coercion widens toward `Float`). For `Money`,
`mfb spec language types` (`Money`) says "there is **no implicit conversion**
into or out of `Money` — every crossing is an explicit `toMoney(...)`", a
`Money` literal comes only from an expected type on an *untyped* literal or an
`m` suffix, and the sibling `5.0F` is already rejected in a `Money` slot; the
architecture pseudocode's `E=Money ∧ A=Float` arm was written for the untyped
literal and is corrected. So:

- `Fixed` slot: a `Float` literal is converted as `toFixed(Float)` converts
  (`mfb spec language types`, "Promotion failures and equality"): the binary64
  value, scaled by 2^32, rounded to nearest, ties away from zero, range-checked
  on that value. `ir::lower` does it at compile time
  (`numeric::fixed_constant_from_float_literal`), in every slot that lowers its
  literal with an expected type, and the negated form takes its lowered
  literal's type.
- `Money` slot: `5.0f`/`-5.0f` is a type error through each slot's existing
  mismatch rule (no new diagnostic). `ir::shape::expression_compatible` excludes
  an `f`-suffixed literal from the `Money` arms; `ir::verify`'s
  `expression_compatible` no longer admits any `Float` const into a `Fixed` or
  `Money` slot (lowering never leaves one there legitimately, so such a const in
  package IR is f64 bits about to be read as a carrier).

Also fixed: `numeric::fixed_conversion_from_decimal` classified a whole part
too long for i128 (`1e400`, `1e400F`, unsuffixed `1e400` in a Fixed slot) as
malformed, so the verifier skipped `TYPE_FIXED_LITERAL_OVERFLOW` and lowering
failed with a location-less "invalid Fixed constant". It is out of range. The
coverage unit test `fixed_raw_from_decimal_rejects_malformed_values` (915e66983)
asserted the malformed classification; that one line is corrected — spec
§4.12 makes an unrepresentable literal a `TYPE_*_LITERAL_*` error, and the
parent's own output for `LET x AS Fixed = 1e400F` has no diagnostic at all.

Found while fixing: constructor arguments, `WITH` updates and `MATCH` literal
patterns were never literal-range-checked (`Box[300]` stored 300 in a `Byte`
field and printed 300; an out-of-range Fixed/Money/Integer field failed in
lowering with no location). Fixed in the following commits, with
`tests/syntax/types/literal-range-field-and-match-invalid`.

Evidence (`/tmp/b820/matrix`, 13 slot kinds × Fixed/Money × 10 literal forms,
built and run with the parent and the fix): every `f` literal in a Fixed slot
now prints its value (`g(1.5f)` 1073217536.00 → 1.50, `CASE 1.5f` nomatch →
match, `-1.5f` internal `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE` → -1.50,
`-2147483648.0f` → the minimum); every `f` literal in a Money slot is the slot's
mismatch error (it was a wrong value such as 46094342186137.03, or the internal
error when negated); unsuffixed, `F` and `m` forms are unchanged. The runtime
fixture's output is byte-identical on macOS arm64, 2226 (aarch64 glibc), 2229
(riscv64 musl) and 206 (Windows x86-64).

Reach: every project under `tests/` and `examples/` (1634) built with both
compilers (`mfb build -q`, and again with `-ast -ir`): no build log, AST or IR
differs except the new fixtures. All 1145 man-page examples compiled with
`scripts/man-run-examples.sh` (each package, each guide topic with `--topic`)
under both compilers: identical logs (1144 build; the one failure,
`variable` guide #2, is a deliberate shadowing-error example). No existing
golden moves: the IR of every fixture is unchanged and no codegen source
changed. `cargo test --release --bin mfb -- numeric:: ir:: spec man`: 1334
passed.
