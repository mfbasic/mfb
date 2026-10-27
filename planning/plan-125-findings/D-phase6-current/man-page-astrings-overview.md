### 1. `a.text` compiles and runs
UNIT:      man-page:astrings/overview
CLAIM:     "It is opaque — it exposes no user-visible fields (a.text does not compile), cannot be built with a record literal (AttributedString[...]), and cannot be WITH-updated."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/D-phase6-current/man-page-astrings-overview/c_field.mfb` built with `mfb build -q proj_c_field` and ran:
```
IMPORT astrings
IMPORT io
FUNC main() AS Nothing
  LET a AS AttributedString = astrings::addAttribute(astrings::fromString("xy"), 0, 1, astrings::bold())
  io::print(a.text)
  io::print(toString(a.text = "xy"))
END FUNC
```
prints `xy` then `TRUE`. The field access is accepted and yields the visible text. Only an explicit binding fails, and not with a field error: `LET t AS String = a.text` (c_text_rec.mfb) gives `TYPE_UNKNOWN_VALUE: Initializer for binding t does not have a known type`. Worse, an unknown field is not diagnosed at all: `io::print(a.foo)` (c_foo.mfb) aborts the build with the internal message `error: native code record 'AttributedString' has no field 'foo' while lowering eval call io.print`. By contrast `i.text` on an Integer (c_text_int.mfb) gives the proper `TYPE_FIELD_ACCESS_REQUIRES_RECORD`. The record-literal and WITH halves of the sentence are true: `AttributedString[text := "x"]` (c_lit.mfb) gives `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`, and `WITH a { text := "y" }` (c_with.mfb) gives `TYPE_READ_ONLY_RECORD_UPDATE`.
SUGGESTED: This is a compiler bug, not a wording problem: field access on `AttributedString` should be rejected with `TYPE_FIELD_ACCESS_REQUIRES_RECORD` (or the read-only-record diagnostic) in the checker, and `a.foo` must never reach lowering. Until it is fixed the page claim is false at HEAD; the main thread should file/fix the bug and keep the sentence, or drop the parenthetical "(a.text does not compile)" so the page does not assert something the binary contradicts.

### 2. Reader list omits six of the fourteen overloads
UNIT:      man-page:astrings/overview
CLAIM:     "The readers include byteLen, contains, count, startsWith, endsWith, find, graphemes, and split;"
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/strings/mod.rs:is_tier_a_query` lists fourteen readers: byteLen, contains, count, endsWith, endsWithAny, find, graphemes, graphemesCount, split, startsWith, startsWithAny, toBytes, toScalars, graphemeAt. Probe p1.mfb confirmed the omitted ones accept an AttributedString: `strings::startsWithAny(a, ["a"])` printed `TRUE`, `strings::graphemesCount(a)` printed `2`, `strings::graphemeAt(a, 0)` printed `a`, and `strings::toScalars(a)` type-checked as `List OF Scalar`. The page's transform list, by contrast, is complete against `TIER_B_TRANSFORMS` (the "trim family" covers trimChars, "four padding functions" covers the four pad members).
SUGGESTED: "The readers are byteLen, contains, count, startsWith, startsWithAny, endsWith, endsWithAny, find, graphemes, graphemesCount, graphemeAt, split, toBytes, and toScalars;"

### 3. Mixing `&` with a String fails with an internal symbol in the message
UNIT:      man-page:astrings/overview
CLAIM:     "a & b joins two AttributedString values, text and styling together; there is no mixing with a plain String."
VERDICT:   misleading
EVIDENCE:  The claim is true in substance. `"y" & a` (c_mix2.mfb) is rejected cleanly: `TYPE_BINARY_OPERATOR_MISMATCH: Operator & requires String operands, got String and AttributedString`. But `a & "y"` (c_mix.mfb) is rejected with `error: TYPE_CALL_ARGUMENT_MISMATCH: Argument 2 for #astrings_concat has type String, expected AttributedString.`, with no source location and leaking a mangled helper name a developer cannot look up. Also `a & b` with b italic (p1.mfb) printed `**ab***cd*`, confirming the join keeps both stylings.
SUGGESTED: Page wording can stand. The diagnostic for `AttributedString & String` is a compiler defect (should be the same `TYPE_BINARY_OPERATOR_MISMATCH` with a location, naming `&`, not `#astrings_concat`); the main thread should file it. Optionally make the sentence sharper: "a & b joins two AttributedString values, text and styling together; joining one with a plain String in either order is a compile error."

Everything else on the page checked out against the binary. The example (ex.mfb) compiled and printed exactly the four lines the page shows. p1.mfb verified: `repeat` keeps styling on every copy (`**ababab**`), `padLeft` padding is unstyled (`---**ab**`), `upper`/`lower`/`caseFold`/`normalizeNfc` drop styling (`AB`, `ab`, `ab`, `ab`), `trim` keeps styling on the kept text, `io::print`/`io::write` accept an AttributedString directly, and a default `MUT d AS AttributedString` has empty text and empty markdown. c_mut.mfb and c_pass.mfb confirmed assignment and passing copy (restyling the copy left the original unstyled). c_cmp.mfb, c_map.mfb and c_set.mfb confirmed `=`, Map key, and Set element are all rejected with `TYPE_REQUIRES_COMPARABLE`. c_coerce.mfb confirmed `LET s AS String = a` is rejected, so no implicit coercion. No banned memory vocabulary and no compiler-internal mental model appears in the prose.
