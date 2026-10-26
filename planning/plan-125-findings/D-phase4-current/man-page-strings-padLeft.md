### 1. An oversized width that still fits in 64 bits raises ErrOutOfMemory, not ErrInvalidArgument
UNIT:      man-page:strings/padLeft
CLAIM:     "A negative width raises the same error, as does a result size that cannot be represented in 64 bits." and "ErrOutOfMemory means the program ran out of memory."
VERDICT:   incomplete
EVIDENCE:  Probe `errs.mfb` (TRAP wrapper around each call), built with the release binary:
```
huge            padLeft("ab", 9223372036854775807, "😀")  -> err=77050002 Argument value is not valid...
hugeNoPadNeeded padLeft("ab", 9223372036854775807, "-")   -> err=77010001 Allocation failed.
```
The lowering (`gen_pad.rs:pad_measure`, `emit_checked_size_multiply` / `emit_checked_size_add` branching to `invalid`) only raises `ErrInvalidArgument` when `padCount * padLen + valueLen` overflows; a width that merely demands more memory than exists reaches `emit_arena_alloc_call` and raises `ErrOutOfMemory`. So which error an absurd `width` produces depends on the byte length of `padChar`, and the page's "ran out of memory" sentence reads as though only a genuinely exhausted program sees that code.
SUGGESTED: "A negative `width` raises `ErrInvalidArgument`, as does a result whose byte size cannot be represented in 64 bits. A `width` that is representable but larger than available memory raises `ErrOutOfMemory` instead, so a wildly large `width` can surface as either error depending on the byte length of `padChar`."

### 2. A single visible character that is more than one scalar is rejected as padChar, and the page does not say so
UNIT:      man-page:strings/padLeft
CLAIM:     "Must be exactly one Unicode scalar value." / "it must be exactly one well-formed Unicode scalar value — neither empty nor more than one scalar"
VERDICT:   incomplete
EVIDENCE:  Probe `errs.mfb`:
```
twoScalarGrapheme padLeft("ab", 5, "e\u0301")   -> err=77050002
emojiZWJ          padLeft("ab", 5, "👨‍👩‍👧")      -> err=77050002
```
and probe `edges.mfb`: `padLeft("x", 3, "\u0301")` succeeds and prints `[́́x]` (a bare combining mark is accepted). The page spends a paragraph on scalar-vs-grapheme for `width` but never applies the same sharp edge to `padChar`, where it bites harder: a decomposed accented letter or a family emoji looks like one character and is refused, while a lone combining mark looks like nothing and is accepted. `gen_pad.rs:pad_measure` requires `emit_scalar_count_loop` == 1 then a full decode/re-encode round-trip.
SUGGESTED: Append to the `padChar` paragraph: "One scalar is not one visible character: a decomposed accented letter (`"e"` followed by U+0301) or an emoji sequence joined with U+200D is several scalars and is rejected, while a lone combining mark is one scalar and is accepted."

### 3. "Neither argument" on a three-parameter function
UNIT:      man-page:strings/padLeft
CLAIM:     "Neither argument is mutated."
VERDICT:   misleading
EVIDENCE:  Declaration on the same page: `strings::padLeft(value AS String, width AS Integer, [padChar AS String])`. Probe `edges.mfb` confirms `value` is unchanged (`orig=[ab] padded=[---ab]`) and probe `astr.mfb` confirms the AttributedString input is unchanged (`orig still=**hi**`), so the fact is true, but "neither" names two of three.
SUGGESTED: "No argument is mutated."

### 4. "the sharp edge of the pair" names no pair
UNIT:      man-page:strings/padLeft
CLAIM:     "Scalar count is not screen width, and this is the sharp edge of the pair."
VERDICT:   misleading
EVIDENCE:  Rendered page: the sentence precedes any mention of `padLeftToWidth`, so "the pair" has no antecedent on first read. The reader has to finish the paragraph to learn the pair is `padLeft` / `padLeftToWidth`.
SUGGESTED: "Scalar count is not screen width, and that is what separates `padLeft` from `strings::padLeftToWidth`."

### 5. "You still get a new String back" describes identity a developer cannot observe
UNIT:      man-page:strings/padLeft
CLAIM:     "You still get a new `String` back in that case, and `value` itself is never changed."
VERDICT:   out-of-scope
EVIDENCE:  Nothing in the language lets a program distinguish "a new String equal to value" from "value"; the only observable fact is that `value` is unchanged, which the second clause already states. "New ... back" teaches an allocation/identity model (§3, §4 spirit) without using a banned word. The lowering (`gen_pad.rs:lower_strings_pad`, `mark_fresh_string`) does allocate, but that is implementation.
SUGGESTED: "In that case the result equals `value`, and `value` itself is unchanged."

Everything else on the page checks out against the binary. Both examples compile and print exactly what the prose implies (`[   42]`, `00042`, `hello`, `😀😀x`). Width 0 and width equal to the length return the input unchanged, an empty `value` is padded to `width`, `padLeft("日本", 4, "-")` prints `--日本`, negative width and empty or two-scalar `padChar` raise 77050002, and the AttributedString overload pads the text and shifts the bold span past the padding (`toMarkdown` goes from `**hi**` to `\*\*\***hi**`, with the default space `padChar` also working). Probes live under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-padLeft/{ex1,ex2,edges,errs,astr}.mfb`.
