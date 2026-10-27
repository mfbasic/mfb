### 1. `isDigit` accepts digits that `toInt`/`toFloat` reject, and the page sends readers from one to the other without saying so
UNIT:      man-page:strings/isDigit
CLAIM:     "Note also that a digit test is not a number parser — use `toInt` or `toFloat` to convert text to a number."
VERDICT:   misleading
EVIDENCE:  The page's own earlier sentence promises `isDigit` "accepts ASCII `0`–`9` and the decimal digits of other scripts, such as the Arabic-Indic and Devanagari digits" (verified TRUE for U+0660, U+0966, U+FF11, U+1D7CE, U+16130, U+16D70, U+1E4F0, U+1E5F1 by probe below). But the parser it points to does not accept them. Probe `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-isDigit/p_probe/src/main.mfb`, built with `mfb build -q p_probe`:
```
IMPORT io
IMPORT strings
FUNC main() AS Integer
  LET s AS String = strings::fromScalars([toScalar(1633), toScalar(1634)])   ' "١٢" Arabic-Indic 1,2
  io::print("isDigit(first) -> " & toString(strings::isDigit(toScalar(1633))))
  LET n AS Integer = toInt(s) TRAP(e)
    io::print("toInt raised")
    RECOVER -1
  END TRAP
  io::print("toInt -> " & toString(n))
  RETURN 0
END FUNC
```
Output:
```
isDigit(first) -> TRUE
toInt raised
toInt -> -1
```
A developer who reads this page's sentence as "validate with `isDigit`, then convert with `toInt`" writes code that passes the check and then raises. The page lists the exact scripts that trigger this and never says the converter only takes ASCII. Lowering read: `__strings_isDigit` in `src/codegen/builtins/strings/helper_scalar_seam.rs` (`cp < 128` fast path, else `strings::genCat(cp) = "Nd"`).
SUGGESTED: "Note also that a digit test is not a number parser, and the two do not agree on what a digit is: `toInt` and `toFloat` read only the ASCII digits `0`–`9`, so a string made of Arabic-Indic or Devanagari digits passes `isDigit` on every scalar and still fails to convert. Use `toInt` or `toFloat` to turn text into a number; use `isDigit` only to classify."

### 2. The Unicode version behind the classification is not stated
UNIT:      man-page:strings/isDigit
CLAIM:     "Classification follows the Unicode general categories, and is deterministic and locale-independent."
VERDICT:   incomplete
EVIDENCE:  Which Unicode version decides `Nd` is developer-visible: digits added in Unicode 15.0 (Nag Mundari U+1E4F0) and 16.0 (Garay U+16130, Kirat Rai U+16D70, Sunuwar U+1E5F1) all return TRUE (probe above, same project, earlier revision; every line printed `-> TRUE`). The table is vendored from Unicode 16.0.0 per `third_party/unicode/README.md` line 3 (`grep -n "Unicode 16.0.0" third_party/unicode/README.md`). `mfb man unicode` does not name a version either (`mfb man unicode | grep -i -n "16\.0\|version"` prints nothing), so there is nowhere on the man surface a developer can learn whether a newly encoded script's digits count. Minor, but the sentence invites the question and the page cannot answer it.
SUGGESTED: "Classification follows the Unicode 16.0 general categories, and is deterministic and locale-independent."

Everything else on the page verified. Both examples compiled with `mfb build -q` from `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-isDigit/p_ex1` and `p_ex2` and printed `TRUE` / `FALSE` and `3`, matching the page. Boundary probe (same scratch dir, `p_probe`, earlier revision) printed TRUE for U+0030, U+0039, U+0660, U+0966, U+FF11, U+1D7CE, U+1D7D8 and FALSE for U+002F, U+003A, U+216B (Nl), U+00B2, U+00BD, U+2460 (No), U+0000, U+FFFF, U+10FFFF, confirming "exactly `Nd`, no wider" and the Nl/No exclusions. "Total … never fails" holds: the descriptor's `errors` vec is empty in `src/codegen/builtins/strings/func_is_digit.rs` and the only raise seen (U+D800) came from `toScalar` constructing the argument, not from `isDigit`. No banned memory vocabulary, no compiler-internal vocabulary, intro is distinct from the description's first line, and `strings::toScalars` in See also renders.
