Review complete. Findings follow.

### 1. Two See-also targets do not render
UNIT:      man-topic:unicode
PAGE:      unicode (package.md, See also)
CATEGORY:  discoverability
CLAIM:     "- `mfb man general find`" and "- `mfb man general mid`"
VERDICT:   wrong
EVIDENCE:  `mfb man general find` prints `error: unknown general function \`find\``; `mfb man general mid` prints `error: unknown general function \`mid\``. `mfb man general` lists no find or mid; both live in strings (`mfb man strings find`, `mfb man strings mid` render). `grep -rn "general find\|general mid" src/docs src/codegen/builtins` hits only `src/docs/man/unicode/package.md:55-56`. §9.1 requires every cross-reference to render.
SUGGESTED: Replace with `mfb man strings find` and `mfb man strings mid`. The body's "search, and substring indexes" sentence should name those two functions so the links are motivated.

### 2. No sentence says that `=` and `<` compare raw scalars, with no normalization or folding
UNIT:      man-topic:unicode
PAGE:      unicode (Description)
CATEGORY:  coverage
CLAIM:     The page describes normalizeNfc and caseFold but never says what string equality and ordering do by themselves. The most common Unicode question a developer arrives with ("why is `"e\u{301}" = "é"` FALSE?") is unanswered, and the See-also to `types string` is captioned only "literals and \u{HEX} escapes", so the reader is not sent to the section that does answer it.
VERDICT:   missing
EVIDENCE:  Probe (proj/build/proj.out): `eq without nfc: FALSE`, `eq with nfc: TRUE`. `mfb man types string` section "Comparison and ordering" states: "Two Strings compare with =/<> and order with <, >, <=, >= lexicographically by Unicode scalar value — deterministic across targets, not locale collation, and not grapheme-aware." The unicode topic has no equivalent sentence and no link to it.
SUGGESTED: Add after the normalizeNfc/caseFold paragraph: "String `=`, `<>` and ordering compare Unicode scalar values directly, with no normalization, case folding or locale collation: `"e\u{301}" = "é"` is FALSE until both sides are passed through strings::normalizeNfc, and a caseless comparison needs strings::caseFold on both sides. See `mfb man types string` (Comparison and ordering)." Widen the See-also caption for `types string` to cover comparison.

### 3. The topic never names the Scalar type or how to get from a String to its scalars
UNIT:      man-topic:unicode
PAGE:      unicode (Description)
CATEGORY:  coverage
CLAIM:     "String length, search, and substring indexes are zero-based Unicode scalar indexes." The page tells the reader everything is counted in scalars but never says that a scalar is a value they can hold (`Scalar`), nor how to obtain one (`strings::toScalars`, `toScalar`, `toInt`) or classify it (`strings::isLetter` etc.). "Unicode whitespace" is mentioned as a behavior with no function named for it.
VERDICT:   missing
EVIDENCE:  `grep -c Scalar src/docs/man/unicode/package.md` matches only the See-also caption. `mfb man strings` overview names "the Unicode-scalar seam (toScalars, fromScalars, and the Scalar classifiers isLetter, isDigit, isWhitespace, isUpper, isLower)"; `mfb man types string` section "Conversions and the scalar seam" lists toInt(Scalar)/toScalar(Integer). Neither is reachable from the unicode topic's body.
SUGGESTED: One short paragraph: "A single scalar is the `Scalar` primitive (written `` `中` ``). strings::toScalars splits a String into them and strings::fromScalars rebuilds it; toInt and toScalar convert between a Scalar and its code point. strings::isWhitespace, strings::trim and friends use the Unicode White_Space property." Add `mfb man strings toScalars` and `mfb man strings isWhitespace` to See also.

### 4. "MFBASIC strings are UTF-8 text" with no word on how bytes that are not UTF-8 are handled, or on UTF-16/32
UNIT:      man-topic:unicode
PAGE:      unicode (Description)
CATEGORY:  coverage
CLAIM:     "MFBASIC strings are UTF-8 text." A topic titled "Unicode behavior" never says that strings are always well-formed, what happens to malformed input bytes, or that the encoding package holds the UTF-8/16/32 codecs.
VERDICT:   missing
EVIDENCE:  `mfb man encoding utf8Decode`: "Because MFBASIC strings are always well-formed UTF-8, the input is validated in full before the string is produced … each raises ErrInvalidFormat." `mfb man encoding` lists utf8Decode/utf8Encode/utf16Decode/utf16Encode/utf32Decode/utf32Encode. The unicode topic does not mention encoding at all (`grep -c encoding src/docs/man/unicode/package.md` is 0).
SUGGESTED: Add: "A String is always well-formed UTF-8. Bytes arrive as List OF Byte and become text through encoding::utf8Decode, which raises ErrInvalidFormat on a malformed sequence rather than substituting a replacement character; strings::toBytes and encoding::utf8Encode go the other way, and encoding also provides the UTF-16 and UTF-32 codecs." Link `mfb man encoding utf8Decode`.

### 5. The topic hedges on which Unicode version is embedded while the function pages commit to one
UNIT:      man-topic:unicode
PAGE:      unicode (Description)
CATEGORY:  consistency
CLAIM:     "…follow the Unicode data embedded in the compiler/runtime."
VERDICT:   inconsistent
EVIDENCE:  `mfb man strings --all | grep -n "Unicode 16"` shows five function pages (isLetter, isUpper, isLower, lineBreaks, upper/lower family) stating "Unicode 16.0"; `mfb spec unicode --all` cites Unicode 16.0.0 throughout. The overview topic that owns the concept gives no version, so a developer cannot learn from it which emoji or scripts the classifiers know.
SUGGESTED: "…follow the Unicode 16.0 data embedded in the compiler, so results do not depend on the host system's Unicode tables or locale."

### 6. Regex is the other scalar-indexed surface and is not reachable from this topic
UNIT:      man-topic:unicode
PAGE:      unicode (See also)
CATEGORY:  discoverability
CLAIM:     The See-also list omits regex even though its indexes follow the rule the topic exists to teach.
VERDICT:   missing
EVIDENCE:  `mfb man regex` lines 46-49: "Every position and index a regex function accepts or reports is a zero-based Unicode scalar index — never a byte offset and never a grapheme-cluster index — consistent with len and the strings package."
SUGGESTED: Add "regex::" to the indexes sentence ("…indexes, including every regex:: position, are zero-based Unicode scalar indexes") and `mfb man regex` to See also.

Verified as correct and not reported: scalar counting (`len=11`, `byteLen=27`, `graphemes=7` on the probe string), caseFold/upper/lower changing length (`Straße` to 7 scalars, `İ` lowering to 2), NFC collapsing `e\u{301}` to 1 scalar, padLeftToWidth treating 日本語 as 6 columns without the terminal surface, the measureText surface-active caveat matching `mfb man term measureText`, and the utf8proc licensing paragraph matching `LICENSE` lines 26-75. Probe program and binary are under /tmp/plan-125-scratch/B-recheck/man-topic-unicode/proj. No repository file was modified.
