### 1. Overview's toString source list omits Money and enum values
UNIT:      man-pkg:general
PAGE:      package-wide (overview Description)
CATEGORY:  overview-mismatch
CLAIM:     "toString takes Integer, Float, Fixed, Boolean, String, Byte, Scalar, or List OF Byte (Float and Fixed also take an optional Byte second argument)."
VERDICT:   wrong
EVIDENCE:  Probe `toString(1.5m)` / `toString(1.5m, toByte(3))` / `toString(1.129m)` compiled and printed `1.50` / `1.500` / `1.13` (`mfb build -q proj && ./proj/build/proj.out`). `src/codegen/builtins/general/mod.rs:376-382` lists the precision form as `(Float|Fixed|Money, Byte)`. The toString page itself documents the enum form ("An enum value renders as its member's name") and the toMoney page's own example calls `toString(total)` on a Money, so the overview contradicts two of its sibling pages. (Side note for the main thread: the compiler's TYPE_CALL_ARGUMENT_MISMATCH text for toString also omits Money — "expected Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, Scalar, List OF Byte, or an enum" — which is a diagnostic bug worth filing, not a doc fix.)
SUGGESTED: "toString takes Integer, Float, Fixed, Money, Boolean, String, Byte, Scalar, List OF Byte, or an enum value (Float, Fixed and Money also take an optional Byte second argument)."

### 2. toString page never says how Money renders
UNIT:      man-pkg:general
PAGE:      general toString
CATEGORY:  coverage
CLAIM:     The Description covers Integer, Float ("two decimal places by default"), Boolean, enum and List OF Byte, and says nothing about Money. "How do I print a Money?" is a question a developer arrives at this unit with, and the toMoney page answers it only implicitly by calling toString in its example.
VERDICT:   missing
EVIDENCE:  Probes: `toString(1.12345m)` → `1.12`; `toString(1.129m)` → `1.13`; `toString(1.12345m, toByte(5))` → `1.12345`; `toString(-1.5m, toByte(0))` → `-2`. So Money renders with two decimals by default, rounded, and `precision` extends it up to its five places — none of which is on the page. `mfb man money` has no text-rendering function either (`money::convert/getRounding/round/scale/setRounding/toRate`), so this page is the only place it can live.
SUGGESTED: Add a Money sentence beside the Float one: "A Money also renders with two decimal places by default, rounding the rest away — toString(1.129m) is "1.13"; pass precision (up to 5) to show more." State whether the rounding follows money::setRounding (I did not probe that; the main thread should before writing it).

### 3. isZero implies it accepts Money; it does not
UNIT:      man-pkg:general
PAGE:      general isZero
CATEGORY:  consistency
CLAIM:     "For Integer, Fixed and Money the question has no such ambiguity."
VERDICT:   misleading
EVIDENCE:  Probe `isZero(0m)` fails to compile: "Call to `isZero` has argument type(s) (Money), expected Integer, Float, or Fixed." The sibling pages isPositive and isNegative both say "It accepts Integer, Float, or Fixed." and isZero is the only one of the three that never states its accepted types, then names Money in passing. A developer reading the three as a set (which all three pages tell them to do) will try isZero on a Money.
SUGGESTED: Add the same sentence the siblings carry — "It accepts Integer, Float, or Fixed." — and change the Float remark to "For Integer and Fixed the question has no such ambiguity."

### 4. Overview's toInt list omits the enum form the page documents
UNIT:      man-pkg:general
PAGE:      package-wide (overview Description)
CATEGORY:  overview-mismatch
CLAIM:     "toInt takes String (with an optional Integer base), Byte, Float, Fixed, Money, or Scalar."
VERDICT:   inconsistent
EVIDENCE:  The toInt page has a full paragraph and an example on "From an enum value you get the member's position", and the compiler's own list at `src/codegen/builtins/general/mod.rs:266` is "String[, Integer], Byte, Float, Fixed, Money, Scalar, or an enum". The overview promises "its page lists every form" and then gives a list that is itself short by one.
SUGGESTED: "...Money, Scalar, or an enum value."

### 5. toScalar page omits the Byte form the overview promises
UNIT:      man-pkg:general
PAGE:      general toScalar
CATEGORY:  overview-mismatch
CLAIM:     Overview: "toScalar takes Integer, String, or Byte." The toScalar page describes only "From an Integer…" and "From a String…" and its Parameters row says "A codepoint … or a String holding exactly one scalar."
VERDICT:   inconsistent
EVIDENCE:  Probe `toScalar(toByte(65))` compiled and printed `A`, so the Byte form exists and the overview is right; the page is the one that is incomplete. The overview's contract is "its page lists every form", which this page breaks.
SUGGESTED: Add to the page: "From a Byte it takes the byte's value as the codepoint, so toScalar(toByte(65)) is also A; a Byte is always in range, so that form never raises." Update the Parameters row to mention Byte.

### 6. toFloat page does not describe its Fixed and Money forms
UNIT:      man-pkg:general
PAGE:      general toFloat
CATEGORY:  overview-mismatch
CLAIM:     Overview: "toFloat takes String, Integer, Fixed, or Money." The page has paragraphs for "From a String" and "From an Integer" only.
VERDICT:   missing
EVIDENCE:  Probe `toFloat(1.5m)` and `toFloat(1.5F)` both compile and print `1.5000` at precision 4. Compiler list at `mod.rs:267` confirms the four forms. The toFixed and toMoney pages each name every form they accept, so toFloat is the odd one out, and a developer who reads the overview's "its page lists every form" cannot learn from this page whether a Money→Float conversion is exact or can raise.
SUGGESTED: Add one sentence after the Integer paragraph: "From a Fixed or a Money it converts the number; every Fixed and Money fits a Float, so those forms never raise, though a Money with five decimal places may not be exactly representable."

Checked and found consistent (no finding): the overview's lists for toByte, toFixed, toMoney and toScalar against `mod.rs:266-269` and probes (`toMoney(toByte(3))` → `3.00`; `toFloat(toByte(3))`, `toFixed(toByte(3))`, `toMoney(toScalar(65))`, `toFloat(toScalar(65))` all correctly rejected); isNumeric as a guard for the decimal conversions (`isNumeric("1e5")` TRUE and `toFixed("1e5")`, `toMoney("1e5")` both succeed; `isNumeric(" 12")` FALSE and `toFloat(" 12")` raises 7-705-0003, so the guard is never looser than the conversion); isPositive/isNegative "Integer, Float, or Fixed" (Money and Byte rejected for all three predicates); the error page's field names and the Errors tables on every page against their Description text; the Functions table covers all 18 descriptors in `src/codegen/builtins/general/`; `mfb man general types` correctly reports no public types.
