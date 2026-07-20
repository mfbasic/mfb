# bug-367: a negative `Fixed` literal silently stores the f64 bit pattern of its magnitude

Last updated: 2026-07-19
Effort: small
Severity: **HIGH** (silent wrong values in a core numeric type — no error, no
warning, no crash; the program just computes the wrong number)
Class: Compiler / IR lowering

Status: FIXED (2026-07-19)
Regression Test: `tests/rt-behavior/codegen/bug367_negative_fixed_literal`,
verified to print the corrupt values on the pre-fix compiler. `Fixed` record
fields are also exercised by `record-field-arithmetic-rt` and
`math-record-field-args-rt`.

## Reproduction

Two lines. No records, no arithmetic, no `math::`.

```
LET a AS Fixed = -1.25
io::print(toString(a))
```

```
-1074528256.00        ' expected -1.25
```

The positive form (`LET a AS Fixed = 1.25`) was always correct, which is why this
survived. Every negative `Fixed` literal was affected, in every context: `LET`,
`MUT`, positional and named record construction, and as a call argument.

`-1074528256.0` is exactly the f64 bit pattern of `1.25`
(`0xBFF4000000000000` = -4615063718147915776) read as a Q32.32 raw
(-4615063718147915776 / 2^32).

## Root cause

`ir::lower`'s `Expression::Unary` arm propagated the expected type into a negated
numeric literal **only for `Money`**:

```rust
let money_literal_negation = operator == "-"
    && expected == Some("Money")
    && matches!(operand.as_ref(), Expression::Number(_));
```

with the comment: *"Only the Money case propagates expected — Fixed/Byte keep
their existing (Float/Integer-const) operand shape so their goldens are
unchanged."*

That is the whole bug, and the comment states the reasoning that caused it:
**goldens were kept stable at the cost of correctness.** Without the
propagation the operand stayed a `Float` const under a `Float`-typed unary minus,
so the bind lowered as

```json
{ "op": "bind", "type": "Fixed",
  "value": { "kind": "unary", "type": "Float",
             "operand": { "kind": "const", "type": "Float", "value": "1.25" } } }
```

Codegen then materialized an f64, negated it, and stored those bits into a Q32.32
slot. Nothing along the way had a reason to complain.

bug-07's fold rescued exactly one input — the min `Fixed` (`-2147483648.0`),
whose positive magnitude overflows the raw — which is why *that* boundary worked
and had a passing test while every ordinary negative literal was corrupt.

## Fix

Extend the expected-type propagation to `Fixed` alongside `Money`
(`exact_literal_negation`), so the operand lowers as a const of the binding's own
type and the node is annotated to match. The min-`Fixed` boundary keeps trapping
on negation (bug-07's case) and `Float`/`Integer`/`Byte` literals are untouched.

## Blast radius

One golden changed: `rt-error/operators/unary-fixed-negation-overflow-rt`'s `.ir`,
by a single line — the `Unary`-over-`Float`-const shape becomes the `Fixed` const
that bug-07's fold was always meant to produce. **That test's runtime behavior is
unchanged**: its `build.log` golden, which records the `ErrOverflow` the test
exists to assert, is byte-identical before and after. Everything else in the
suite is byte-identical (1207 goldens).

## Found by

Adding record-field arithmetic tests for Integer/Float/Fixed/Money at the user's
request (2026-07-19). The `Fixed` column of the matrix printed
`1074528256.00` where `1.25` was expected.

**The lesson recorded in AGENTS.md.** This was initially triaged as
"out of scope, file it for later" because fixing it would churn a golden. That
was wrong, and the user said so plainly: leaving a known bug in place is always
wrong, and a silent wrong value is the worst class of all — nothing downstream
will ever report it. AGENTS.md now carries "Never leave a bug in place" and "The
other half of this rule: once proven, you MUST fix it"; `.ai/compiler.md` carries
"A Bug You Find Is a Bug You Fix".
