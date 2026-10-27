### 1. "negative width or count anywhere in the package raises ErrInvalidArgument" is false for `mid`
UNIT:      man-page:strings/overview
CLAIM:     "The pad character, when supplied, must be exactly one Unicode scalar value; an empty or longer padChar, like a negative width or count anywhere in the package, raises ErrInvalidArgument."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-overview/p1/src/main.mfb` (each call wrapped in a FUNC with a function-level TRAP that returns the error name), built with the release `mfb build` and run:
```
mid 0,-1: ErrIndexOutOfRange
mid -1,1: ErrIndexOutOfRange
find start -1: ErrIndexOutOfRange
graphemeAt -1: ErrIndexOutOfRange
left -1: ErrInvalidArgument
right -1: ErrInvalidArgument
repeat -1: ErrInvalidArgument
padLeft -1: ErrInvalidArgument        (p3)
padLeftToWidth -1: ErrInvalidArgument (p3)
padRight -1: ErrInvalidArgument       (p3)
padRightToWidth -1: ErrInvalidArgument(p3)
```
`strings::mid` with a negative count raises ErrIndexOutOfRange, and its own page's Errors table lists only ErrIndexOutOfRange. "Anywhere in the package" is therefore untrue, and a developer trapping on ErrInvalidArgument for a bad `mid` count will miss it.
SUGGESTED: "The pad character, when supplied, must be exactly one Unicode scalar value; an empty or longer padChar raises ErrInvalidArgument, as does a negative width for the four padding members, a negative count for left, right and repeat. mid is the exception: a negative start or count is an out-of-range window and raises ErrIndexOutOfRange, like a negative start to find or a negative index to graphemeAt."

### 2. mid's clamp-vs-raise paragraph omits the negative case
UNIT:      man-page:strings/overview
CLAIM:     "mid never clamps: a window that starts or runs past the end raises ErrIndexOutOfRange."
VERDICT:   incomplete
EVIDENCE:  Same p1 run: `mid -1,1: ErrIndexOutOfRange`, `mid 0,-1: ErrIndexOutOfRange`, versus `left -1: ErrInvalidArgument`, `right -1: ErrInvalidArgument`. This is exactly the clamp/raise sharp edge `.ai/man-content.md` §2 names for this package, and the page states it only for the past-the-end direction; the negative direction raises a different error from `left`/`right`, and the page's only statement about negatives (finding 1) points the reader at the wrong one.
SUGGESTED: "mid never clamps: a window that starts or runs past the end, or that has a negative start or count, raises ErrIndexOutOfRange — never ErrInvalidArgument, unlike a negative count to left or right."

### 3. lineBreaks does not return bare scalar positions
UNIT:      man-page:strings/overview
CLAIM:     "lineBreaks returns where text may wrap, by the Unicode line-breaking rules, as scalar positions that feed straight into mid and left."
VERDICT:   misleading
EVIDENCE:  `mfb man strings lineBreaks` declaration: `strings::lineBreaks(text AS String) AS List OF strings::LineBreak`, each with fields `index` and `mandatory`. Probe p5 (`/tmp/plan-125-scratch/D-phase4-current/man-page-strings-overview/p5/src/main.mfb`):
```
breaks n=2
break0 idx=3 mandatory=FALSE left=[ab ] mid=[cd]
```
The positions feed `left`/`mid` only after reading `.index`; nothing "feeds straight". The sentence also drops the `mandatory` flag and the always-reported end-of-text break, both of which a wrapping loop needs. A reader writing `strings::left(text, b)` from this sentence gets a type error.
SUGGESTED: "lineBreaks returns a List OF strings::LineBreak, one per place a line may start under the Unicode line-breaking rules, each with an index in scalar positions (so strings::left(text, b.index) is the text before it) and a mandatory flag for breaks the text forces, such as after a line feed; the end of the text is always the last entry."

Everything else on the page checked out against the binary: left/right clamp (p1), find raising ErrNotFound and ErrIndexOutOfRange at start > length and accepting start = length (p1), the full empty-needle table including the four regex:: parallels, trimChars "" and join "" (p2, all 17 lines matched the page), the five Scalar predicates on letters, digits, White_Space including tab/newline/CR/NBSP and not ZWSP, Nl/No rejection (p3), padChar validation (p3), scalar-vs-column padding divergence on CJK, emoji and a combining sequence (p3/p5), padToWidth working without `term::on()` while `term::measureText` raised ErrUnsupported in the same non-TUI run, and the two sharing `measure_text_of` in `src/codegen/builtins/term/func_measure_text.rs` (`lower_measure_text` vs `lower_measure_text_raw`), scalar round trip (p1), `IMPORT strings` with no manifest dependency (every probe's generated `project.json` has none), and the astrings paragraph (p4: `repeat` of `**a**b` gives `**a**b**a**b**a**b`, `replace` of the bold `a` gives unstyled `zz`, padding is unstyled, upper/lower/caseFold/normalizeNfc drop styling). No banned memory vocabulary and no ownership-model sentence found on the rendered page. One cross-page note for the main thread: the overview's `repeat` claim is correct, but the sibling `strings/repeat` page says attribute spans are "remapped by the same edit", which p4 shows is the wrong description for repeat.
