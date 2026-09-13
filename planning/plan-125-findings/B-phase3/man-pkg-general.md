### 1. Conversion overloads are undiscoverable
UNIT:      man-pkg:general
PAGE:      package-wide
CATEGORY:  discoverability
CLAIM:     The rendered unit presents only representative declarations such as "`general::toMoney(value AS String) AS Money`" and gives no conversion matrix or complete accepted-type lists.
VERDICT:   missing
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man general --all | rg '^`general::'` printed single-type declarations for every conversion. `src/codegen/builtins/general/mod.rs:262-270` lists the actual accepted types; `resolve_call` at lines 367-508 implements them. A scratch probe built and ran with `mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-general/probe`, printing `9.00`, `7`, `7`, `7.00`, `7.00`, `A`, `AB` for documented-undiscoverable calls including `toMoney(toByte(9))`, `toByte(money)`, `toInt(money)`, and `toString([Byte, Byte])`.
SUGGESTED: Add a package overview conversion table, or explicitly list every accepted source type and relevant conversion behavior on each conversion page. In particular, make `toMoney`, `toByte`, `toString`, and the numeric conversions discoverable beyond their representative declaration.

### 2. `isNumeric` advertises a guard that its sibling says is unsafe
UNIT:      man-pkg:general
PAGE:      isNumeric
CATEGORY:  consistency
CLAIM:     "`isNumeric` lets you branch instead of trap" and the page's “Use it to guard a conversion” example guards `toInt`.
VERDICT:   inconsistent
EVIDENCE:  The rendered `isNumeric` page says both `isNumeric("1.5")` is `TRUE` and recommends guarding `toInt`; rendered `toInt` says "`isNumeric` is not a safe guard for `toInt`." A scratch probe compiled and ran with `mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-general/probe`, printing `TRUE` followed by `toInt raised 77050003` for `"1.5"`.
SUGGESTED: State that `isNumeric` only guards conversions accepting decimal text, not `toInt`; replace the `toInt` guard example with `toFloat`, `toFixed`, or `toMoney`, and direct integer parsing to `TRAP` or a whole-number-specific validation path.

### 3. The sign-predicate family calls unsupported types “any numeric types”
UNIT:      man-pkg:general
PAGE:      package-wide
CATEGORY:  coverage
CLAIM:     "`It accepts any of the numeric types, not just Integer.`" on `isPositive` and `isNegative`; the same family framing applies to `isZero`.
VERDICT:   misleading
EVIDENCE:  `src/docs/spec/language/04_types.md:480` defines numeric as including `Money` and `Byte`, while `src/codegen/builtins/general/mod.rs:528-541` accepts only `Integer`, `Float`, and `Fixed`. `mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-general/probe` rejected `isPositive(amount AS Money)` with `TYPE_CALL_ARGUMENT_MISMATCH`, reporting expected `Integer, Float, or Fixed`.
SUGGESTED: Replace the generic phrase on all three pages with: “It accepts `Integer`, `Float`, or `Fixed`.”