### 1. "A negative … count anywhere in the package raises ErrInvalidArgument" is false for mid, find and graphemeAt
UNIT:      man-page:strings/overview
CLAIM:     "The pad character, when supplied, must be exactly one Unicode scalar value; an empty or longer padChar, like a negative width or count anywhere in the package, raises ErrInvalidArgument."
VERDICT:   wrong
EVIDENCE:  Probe `p1.mfb` (built via `mfb init` + `mfb build`, run from `proj_p1/build/*.out`) printed:
```
mid(hello,-1,1) raises 77050001
mid(hello,1,-1) raises 77050001
find(hello,l,-1) raises 77050001
graphemeAt(hello,-1) raises 77050001
left(hello,-1) raises 77050002
right(hello,-1) raises 77050002
padLeft(x,-3) raises 77050002
padLeftToWidth(x,-3) raises 77050002
repeat(x,-1) raises 77050002
```
77050001 is ErrIndexOutOfRange, 77050002 is ErrInvalidArgument. `mid`'s `count` parameter is a count, and a negative one raises ErrIndexOutOfRange (`src/codegen/builtins/strings/func_mid.rs`, `errors: vec!["ErrIndexOutOfRange"]`, and its desc: "both must be 0 or greater … Violating that raises ErrIndexOutOfRange"). The overview's own earlier paragraph says "mid never clamps … raises ErrIndexOutOfRange" but only for windows past the end, so the "anywhere in the package" generalisation directly contradicts the binary for mid, and also for negative `start`/`index` positions in find and graphemeAt.
SUGGESTED: "The pad character, when supplied, must be exactly one Unicode scalar value; an empty or longer padChar raises ErrInvalidArgument, as does a negative width for the four padding members, a negative count for left, right and repeat, and a negative times for repeat. A negative position or count given to mid, find or graphemeAt is out of range and raises ErrIndexOutOfRange instead."

### 2. Zero-width padChar restriction on the ToWidth members is omitted
UNIT:      man-page:strings/overview
CLAIM:     "The pad character, when supplied, must be exactly one Unicode scalar value; an empty or longer padChar … raises ErrInvalidArgument."
VERDICT:   incomplete
EVIDENCE:  Probe `p5.mfb`:
```
padLeftToWidth(x,4,U+0301) raises 77050002
padLeft(x,4,U+0301) scalars 4
```
A single combining mark is exactly one scalar and is accepted by `padLeft`, but rejected by `padLeftToWidth` with ErrInvalidArgument because it occupies zero columns. The `padLeftToWidth` page states this ("it must occupy at least one column: a zero-width padChar … is rejected with ErrInvalidArgument"); the overview's sentence, which claims to state the package-wide rule for padChar, says the one-scalar test is the whole rule. A reader who passes a combining mark or ZWJ to the ToWidth members gets a raise the overview says will not happen.
SUGGESTED: Append: "padLeftToWidth and padRightToWidth additionally require the pad character to occupy at least one column; a zero-width scalar such as a combining mark is rejected with ErrInvalidArgument."

### 3. Width padding can undershoot with a two-column padChar; the overview says it "pads to a count of terminal columns"
UNIT:      man-page:strings/overview
CLAIM:     "padLeftToWidth and padRightToWidth pad to a count of terminal columns, the measure displayWidth reports — right for anything that has to line up on screen."
VERDICT:   incomplete
EVIDENCE:  Probe `p6.mfb`:
```
padLeftToWidth(x,4,日) = [日x] width 3
```
With a two-column pad character and an odd gap, the result is one column short of the requested width. The `padLeftToWidth` page documents this ("The result never exceeds columns … the last column is left empty"); the overview, which is the paragraph that sells these members as "right for anything that has to line up on screen", does not, and a table padded with a wide padChar will not line up.
SUGGESTED: Append after "the measure displayWidth reports": "They never exceed the requested width; with a two-column pad character and an odd gap, the result stops one column short."

### 4. Errors table omits ErrOutOfMemory, which repeat, split, join and the ToWidth members raise
UNIT:      man-page:strings/overview
CLAIM:     Errors table lists only `ErrIndexOutOfRange`, `ErrInvalidArgument`, `ErrNotFound`.
VERDICT:   incomplete
EVIDENCE:  Probe `p7.mfb`:
```
repeat(xy,2^62) raises 77010001
padLeftToWidth(x,2^62) raises 77010001
```
77010001 is ErrOutOfMemory. The raise is `builder.raise_error_bare("ErrOutOfMemory")` in `func_repeat.rs:212`, `func_split.rs:258,260`, `func_join.rs:184,192`, and the pad-to-width helper; none of the four descriptors lists it in `errors: vec![…]`, so the derived table on this page and on the four member pages omits it. The convention elsewhere is to declare it: `grep -rn 'ErrOutOfMemory' src/codegen/builtins/ | grep -c 'errors: vec'` reports 72 descriptors that do. The `padLeftToWidth` page's prose even names the error and code while its own derived table does not show it. Per `.ai/man-content.md` §0 this is a descriptor bug, not a prose fix: the `errors` vec on `repeat`, `split`, `join`, `padLeftToWidth` and `padRightToWidth` should declare `ErrOutOfMemory`, which fixes this page's table by derivation.
SUGGESTED: Add `"ErrOutOfMemory"` to the `errors` vec of the five descriptors above (code change, prose unchanged). Optionally add one sentence to the description: "A repeat count or padding width so large the result cannot be built raises ErrOutOfMemory."

### 5. `isUpper` / `isLower` are described as general-category tests without saying what that excludes
UNIT:      man-page:strings/overview
CLAIM:     "the five isX(Scalar) predicates classify a single scalar — isLetter, isDigit, isUpper and isLower by its Unicode general category"
VERDICT:   misleading
EVIDENCE:  Probe `p3.mfb`:
```
isUpper Ⅷ = FALSE
isLower ª = FALSE
isDigit ² = FALSE
isDigit ٣ = TRUE
```
The results are consistent with a general-category test (Ⅷ is Nl, ª is Lo, ² is No), so the sentence is true. But a developer reading "isUpper" expects the Unicode Uppercase property, which includes Ⅷ, and "isDigit" expects superscript digits to count. The sentence states the mechanism ("general category") without stating the observable consequence, so the reader has no way to predict these FALSEs without knowing Unicode's category tables. The sharp edge is on the wrong side of the page: the individual member pages are where it belongs in full, but the overview presents this as the package's contract.
SUGGESTED: "… isLetter, isDigit, isUpper and isLower by its Unicode general category — so isDigit accepts only decimal digits (not superscripts or fractions) and isUpper/isLower accept only cased letters (not Roman-numeral or ordinal-indicator scalars) — and isWhitespace by the Unicode White_Space property, which also covers tab, newline and carriage return."

### 6. "Only a negative count raises" for left/right is verified; "starts or runs past the end" for mid under-describes the accepted boundary
UNIT:      man-page:strings/overview
CLAIM:     "mid never clamps: a window that starts or runs past the end raises ErrIndexOutOfRange."
VERDICT:   incomplete
EVIDENCE:  Probe `p1.mfb`:
```
mid(hello,3,5) raises 77050001
mid(hello,6,0) raises 77050001
```
and the `mid` page: "start may equal the scalar length of value, which with a count of 0 selects the empty slice at the end." A start equal to the length does not "start past the end" and is accepted; the overview reader cannot tell whether `start = len` is allowed, which is exactly the inclusive/exclusive question §2.1 says the prose must answer. The find sentence on the same page gets this right ("0 through the scalar length inclusive"); mid should match.
SUGGESTED: "mid never clamps: start may be anywhere from 0 through the scalar length inclusive, and start + count must not exceed that length; anything else raises ErrIndexOutOfRange."

The rest of the page verified true against the binary. Probes `p2.mfb` (empty-needle rule for every named member plus `regex::find/count/split/replace`, `trimChars("hello","")`, `join(["a","b"],"")`), `p3.mfb` (toScalars/fromScalars round trip on `"a😀é日\t\n"` = TRUE, isWhitespace on tab/newline/CR/U+00A0 TRUE and U+200B FALSE, scalar-not-grapheme indexing on `e\u{301}`, byteLen/toBytes = 5 for `"a😀"`, caseFold("Straße") = caseFold("STRASSE") TRUE while lower() gives FALSE, `trim` leaves its argument unchanged, count("aaaa","aa") = 2) and `p4.mfb` (AttributedString overloads: `repeat` keeps bold on every copy, `upper`/`lower`/`caseFold`/`normalizeNfc` return no attributes, `padLeft` padding and `replace` replacement carry no attributes while kept text keeps them, `contains`/`find`/`byteLen` answer as for the plain String) each printed exactly what the page states. No banned memory vocabulary appears on the rendered page, and no sentence teaches a borrow/ownership model. The page carries no example block, so there was nothing to compile for item 4 of the brief.
