# bug-358: constant-folded `toString(Float)` ignores the documented default precision, so a literal and a runtime value of the same Float print differently

Last updated: 2026-07-22
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (compile-time / runtime divergence)

Status: Resolved (2026-07-22) — the fold now renders the runtime default
Regression Test: tests/rt-behavior (new) — `toString(<float literal>)` and `toString(<same value at runtime>)` produce identical text

`toString(value AS Float, precision AS Byte = 2)` is documented to default to **two**
digits after the decimal point, and that is exactly what the runtime helper does.
But when the argument is a compile-time constant the fold produces the *full*
shortest representation instead, so the same value renders two different ways
depending on whether the compiler could see it.

The single correct behavior a fix produces: `toString(f)` yields the same text for a
given Float whether or not the argument was foldable — whichever default the language
settles on.

References:

- `src/docs/man/builtins/general/toString.md` — `toString(value AS Float, precision AS
  Byte = 2) AS String`, "defaults to `2`".
- `src/target/shared/code/builder_strings.rs:751` — the runtime default,
  `move_immediate(&scratch8, "Byte", "2")`.
- Found while fixing bug-304 (`json::stringify` precision loss); it is why the first
  attempt at that fix failed its own round-trip check.

## Failing Reproduction

```basic
IMPORT io

FUNC identity(x AS Float) AS Float
  RETURN x
END FUNC

FUNC main AS Integer
  io::print("literal =" & toString(3.141592653589793))
  io::print("runtime =" & toString(identity(3.141592653589793)))
  RETURN 0
END FUNC
```

- Observed:
  ```
  literal =3.141592653589793
  runtime =3.14
  ```
- Expected: both identical.

Further runtime cases confirming the default really is two places (these are correct
per the man page): `toString(identity(0.1))` → `0.10`, `toString(identity(1.0/3.0))`
→ `0.33`, `toString(identity(2.5))` → `2.50`.

## Root Cause

Two independent renderers implement `toString(Float)`:

- the runtime helper, which reads the precision slot — defaulted to `2` at
  `builder_strings.rs:751` when no precision argument is supplied;
- the constant folder, which formats the literal without consulting that default.

Nothing keeps the two in agreement, so the divergence is invisible until the same
value takes both paths.

## Goal

- One default, honored by both paths.

### Non-goals (must NOT change)

- The explicit two-argument form `toString(f, nb)`, which is unambiguous and correct
  on both paths.
- `Money`, whose 2-place default is semantically right for currency.

## Blast Radius

- The constant folder's `toString(Float)` arm, or the runtime default — whichever the
  resolution picks.
- **Deciding which way to converge is the real work here, and it is a language
  decision, not a mechanical one.** Two decimal places is the documented default but
  is lossy and surprising for a general-purpose float (`0.000000000000123` prints as
  `0.00`); shortest-round-trip is what most languages do and what `json::stringify`
  needs, but changing the default would move every `.run` golden that prints a Float
  and is a breaking change to documented behavior. A third option is to keep the
  documented default and fix only the folder, which is the smallest change and
  restores consistency without a language change.
- `Fixed` shares the same defaulting code path and should be checked for the same
  divergence.

## Fix Design

Recommend converging the folder onto the documented runtime default first (smallest
change, restores consistency, no golden churn beyond folded call sites), and treating
"should the default be shortest-round-trip?" as a separate language question. bug-304
does not depend on the answer: it now searches for the shortest round-tripping
precision explicitly rather than relying on any default.

## Phases

### Phase 1 — failing test
- [x] rt-behavior fixture printing a literal and a runtime Float of the same value:
      `tests/rt-behavior/conversions/bug358_tostring_default_precision` (15
      folded/runtime pairs across Float and Fixed, plus an explicit-precision
      guard). Proven to fail against the pre-fix compiler and pass post-fix.
### Phase 2 — the fix
- [x] Make the folder honor the documented default. `numeric::default_to_string_text`
      renders a folded `Float` with Rust's exact `{:.2}` (the same correctly-rounded
      `%.2f`, ties-to-even, that `float_format.rs` computes) and a folded `Fixed` by
      converting through `fixed_raw_from_decimal` and mirroring
      `emit_fixed_to_string_value` at precision 2 (half-away-from-zero pre-round,
      truncating ×10 digit steps, including the signed overflow-guard compare the
      minimum Fixed exercises). All four copies of the fold
      (`code/type_utils.rs`, `code/builder_value_semantics.rs`, `plan/symbols.rs`,
      `validate.rs`) now call it; `numeric::expanded_literal_text` (whose only
      callers they were) is removed — scientific notation flows through the
      conversions themselves, preserving plan-28-B.
### Phase 3 — validation
- [x] Full suite green; `Fixed` had the same divergence (confirmed by reproduction)
      and is fixed and regression-tested by the same fixture.

## Validation Plan

- Regression: literal-vs-runtime equality for several Floats.
- Doc sync: `toString.md` if the default changes.

## Summary

Two renderers, one documented default, no agreement between them. Low-risk to fix in
the folder; the broader question of what the default *should* be is worth deciding
deliberately rather than by accident.

## Resolution (2026-07-22)

Reproduced exactly as reported (`literal =3.141592653589793` vs `runtime =3.14`), and
`Fixed` had the same divergence (`toString(0.666F)` folded to `0.666` where the
runtime prints `0.67`). Converged the folder onto the documented runtime default of
two decimal places, per this report's recommendation; the runtime and the man page
are unchanged.

The fold turned out to be a raw source-text passthrough duplicated in four places
(`static_primitive_text_with_constants`, `CodeBuilder::static_primitive_text`,
and the two `native_primitive_text` copies in `plan/symbols.rs` and `validate.rs`);
all four now delegate `Float`/`Fixed` to a single new renderer,
`numeric::default_to_string_text`, which reproduces the runtime formatters
byte-for-byte (verified against runtime output for ties, carries, scientific
notation, sub-ULP values, and the minimum `Fixed`). Four committed tests encoded the
old fold text and were updated as the intended behavior change, each cross-checked
against the runtime path:

- `tests/rt-behavior/general/fixed-min-literal` and
  `tests/rt-behavior/codegen/bug367_negative_fixed_literal` goldens:
  `-2147483648.0` → `-2147483648.00`.
- `tests/rt-behavior/lexical/lexical-literals`: literal-typing assertions like
  `toString(1e3) = "1000"` → `"1000.00"`; `toString(1e-3)` now passes an explicit
  precision so the sub-cent digits stay visible.
- `tests/acceptance/src/primitives.mfb`: `f=3.5` → `f=3.50` in the mixed-type
  concatenation case.
- `tests/syntax/testing/testing-run-valid` golden: the testing framework's own
  failure detail previously mixed the two renderers (`expected 3.0, got 2.00`);
  it now reads `expected 3.00, got 2.00`.

Unrelated find while validating: `rules::tests::unknown_rule_name_trips_the_debug_assert`
asserts a `debug_assert!` panic and so could never pass under `cargo test --release`;
it is now gated `#[cfg(debug_assertions)]`.
