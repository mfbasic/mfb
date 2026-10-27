### 1. The overview never points at the guide topics that explain its own vocabulary
UNIT:      man-pkg:strings
PAGE:      package-wide (overview)
CATEGORY:  discoverability
CLAIM:     The overview's "See also" is exactly one entry: "mfb man strings types — the package's types". It uses "scalar length", "Scalar primitive", "zero-based Unicode scalar values" and "Unicode normalization" throughout and never says where a developer learns what a Scalar is, how to write one, or how to get a string's scalar length.
VERDICT:   missing
EVIDENCE:  `mfb man unicode` ("Unicode behavior, indexes, normalization, and licensing") cites strings:: 12 times (`grep -c strings src/docs/man/unicode/*.md` → 12) and links `mfb man strings`, `mfb man types string` and `mfb man general len` in its own See also; `grep -n "mfb man unicode\|mfb man types string\|mfb man general" all.txt` over the full `mfb man strings --all` render returns nothing. The only place in the whole unit that names the scalar-length builtin is the byteLen page ("use the bare len builtin"; `grep -c "len builtin\|mfb man general len" all.txt` → 1), and `mfb man general len` exists. Other package overviews do cite guide pages (`grep -ln "mfb man types" src/codegen/builtins/*/mod.rs` → collections, money), so there is precedent.
SUGGESTED: Add one sentence to MODULE_DESC near the index/count paragraph: "A string's scalar length is len(value) (mfb man general len); what a String and a Scalar are, how to write them, and why every index here is a scalar index is covered in mfb man types string and mfb man unicode." The renderer will then list those pages under See also.

### 2. padLeftToWidth / padRightToWidth contradict their siblings (and the binary) on the huge-width error
UNIT:      man-pkg:strings
PAGE:      strings::padLeftToWidth, strings::padRightToWidth (versus strings::padLeft, strings::padRight)
CATEGORY:  consistency
CLAIM:     "A columns so large that the padded result cannot be built fails with ErrOutOfMemory (77010001) rather than producing a wrong answer." The padLeft/padRight pages say the opposite split for the same situation: "raises ErrInvalidArgument when the size of the result cannot even be represented, and ErrOutOfMemory when it can be represented but there is not enough memory for it — so which of the two a wildly large width raises depends on the byte length of padChar."
VERDICT:   inconsistent
EVIDENCE:  Probe /tmp/plan-125-scratch/B-recheck/man-pkg-strings/p_padw (built with `mfb build -q`, run `build/p_padw.out`) calling strings::padRightToWidth("x", cols, pc) under TRAP printed:
           max_space 77010001 (cols = 9223372036854775807, padChar " ")
           max_emoji 77050002 (same cols, padChar "😀")
           1T_space 77010001
           1T_emoji 77010001
           so the ToWidth members behave exactly as padLeft/padRight describe: ErrInvalidArgument when the byte size overflows, ErrOutOfMemory otherwise. The same probe against padLeft("x", 9223372036854775807, "😀") also printed 77050002. The ToWidth pages' Errors tables already list both codes, but the prose attributes ErrInvalidArgument only to negative columns and a bad padChar.
SUGGESTED: Replace the quoted sentence on both ToWidth pages with the padLeft/padRight wording: "A columns far too large to build a result for raises ErrInvalidArgument when the size of the result cannot even be represented, and ErrOutOfMemory when it can be represented but there is not enough memory for it — which of the two depends on the byte length of padChar."

### 3. lineBreaks is the one text-taking member that says nothing about AttributedString
UNIT:      man-pkg:strings
PAGE:      strings::lineBreaks
CATEGORY:  overview-mismatch
CLAIM:     Overview: "Many strings:: functions also accept an AttributedString as their first argument, the text they work on; each function's own page documents that overload." The lineBreaks page has no sentence either way, so a developer wrapping styled text (the obvious use with canvas::measureText, which the page recommends) cannot tell from this unit whether it is allowed.
VERDICT:   missing
EVIDENCE:  `grep -ln AttributedString src/codegen/builtins/strings/func_*.rs` lists 33 files; func_line_breaks.rs is not among them. Probe /tmp/plan-125-scratch/B-recheck/man-pkg-strings/p_lb_as passing an astrings::fromString value to strings::lineBreaks fails to compile: `error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH] ... has argument type(s) (AttributedString), expected String`. Every other member whose first argument is text states its overload; startsWith even states the negative case for its second argument ("prefix must be a plain String; pass toString(p)").
SUGGESTED: Add to lineBreaks' desc: "text must be a plain String; for an AttributedString pass toString(a) — the break positions are the same, since styling does not change the text." (Confirm the toString route is the intended one before wording it.)

### 4. The Unicode version is stated on five pages and absent from the rest, including the overview
UNIT:      man-pkg:strings
PAGE:      package-wide
CATEGORY:  consistency
CLAIM:     isDigit, isLower, isUpper say "Classification follows the Unicode 16.0 general categories"; normalizeNfc says "follows the Unicode 16.0 tables"; lineBreaks says "for Unicode 16.0". isLetter says only "follows the Unicode general categories"; isWhitespace, caseFold, upper, lower, trim/trimStart/trimEnd, graphemes/graphemeAt/graphemesCount and the overview give no version at all.
VERDICT:   inconsistent
EVIDENCE:  `grep -n "Unicode 16" all.txt` over the full render → 5 hits (lines 1385, 1567, 1657, 2047, 2426), all on the pages named above. The data really is one pinned version for the whole package: `src/unicode/range_tables.rs` ("Unicode 16.0.0 general-category runs"), `src/unicode/casefold.rs` (vendored CaseFolding-16.0.0.txt), and `mfb man unicode` states "follow the Unicode 16.0 data embedded in the compiler". A developer asking "which Unicode version does isLetter / caseFold / graphemes use?" gets an answer on some sibling pages and silence on the one they opened.
SUGGESTED: State it once in the overview ("Every classifier, case mapping, normalization, grapheme and line-break rule in this package follows Unicode 16.0; see mfb man unicode") and either drop the per-page repetitions or add the same clause to isLetter, isWhitespace, caseFold, upper, lower and graphemes so the siblings agree.

What I checked and found clean: every one of the 41 registered public functions is named in the overview's first paragraph and the two internal-only descriptors (genCat, lineBreakClass) are correctly hidden; the clamp-versus-raise contract for left/right/mid, the empty-needle rule, the find/contains guard caveat, the two padding units, the attribute-drop rule for the four case/normalization members, and the LineBreak record's index/mandatory semantics are each stated the same way on the overview, the function pages and the types page.
