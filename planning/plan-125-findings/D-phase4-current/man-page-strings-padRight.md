### 1. "the pair" is never named on this page
UNIT:      man-page:strings/padRight
CLAIM:     "Scalar count is not screen width, and this is the sharp edge of the pair."
VERDICT:   misleading
EVIDENCE:  `mfb man strings padRight` rendered in full: the word `padLeft` appears nowhere on the page, and the only functions named before this sentence are `padRight` itself. The sentence is lifted from a shared pad-family draft (same wording in `src/codegen/builtins/strings/func_pad_right.rs:DESC`); on a leaf page "the pair" has no antecedent. A reader may take it to mean padRight/padRightToWidth, which is the opposite of what the paragraph goes on to say.
SUGGESTED: "Scalar count is not screen width, and that is the sharp edge of this function."

### 2. The copy-count sentence is false for a value already at or over width
UNIT:      man-page:strings/padRight
CLAIM:     "The number of copies appended is width minus the current scalar length of value."
VERDICT:   incomplete
EVIDENCE:  Probe `p_probe` (`tryIt("longer", "abcdef", 3, "-")` printed `longer [abcdef]`; `tryIt("equal", "abc", 3, "-")` printed `equal [abc]`). The lowering computes `max(0, width - scalarLen)` (`gen_pad.rs:pad_measure`, the `strings_pad_no_pad` branch). The clamp is only stated three paragraphs later; the first paragraph a reader sees states a formula that goes negative.
SUGGESTED: "The number of copies appended is width minus the current scalar length of value, or none when that difference is not positive."

### 3. A huge but representable width raises ErrOutOfMemory, not ErrInvalidArgument
UNIT:      man-page:strings/padRight
CLAIM:     "A negative width raises the same error, as does a result size that cannot be represented in 64 bits."
VERDICT:   incomplete
EVIDENCE:  Probe `p_probe`: `padRight("ab", 9223372036854775807, "-")` printed `ERR 77010001 Allocation failed.` (result size 2^63 bytes fits in 64 bits, so the check passes and the allocation fails). Probe `p_probe2`: `padRight("ab", 9223372036854775807, "日")` printed `ERR 77050002` (3 bytes per copy overflows 64 bits). Lowering: `emit_checked_size_multiply`/`emit_checked_size_add` branch to `invalid` in `gen_pad.rs:pad_measure`. The sentence is true, but "64 bits" is a byte-arithmetic detail the developer cannot reason about from the page, and the developer-visible fact — a huge width fails either way, with one of two codes — is split across two sentences three paragraphs apart with no link between them.
SUGGESTED: "A negative width raises the same error. A width far too large to build a result for raises either ErrInvalidArgument (when the result size cannot be represented) or ErrOutOfMemory (when it can be represented but cannot be obtained)."

### 4. Padding appended to an AttributedString is unstyled — the page does not say so
UNIT:      man-page:strings/padRight
CLAIM:     "value may also be an AttributedString: it returns an AttributedString whose text is transformed exactly as the String overload's and whose attribute spans are remapped by the same edit."
VERDICT:   incomplete
EVIDENCE:  Probe `p_probe2`: `a = fromString("ab")`, `addAttribute(a, 1, 1, bold())`, `padRight(a, 5, "-")`, `toMarkdown` printed `a**b**---`: the existing span stays on `b` and the three appended scalars carry no attributes, even though the last styled scalar is adjacent to them. "Remapped by the same edit" does not tell a reader whether a trailing style extends over the padding, which is the one thing they will ask on this overload.
SUGGESTED: "value may also be an AttributedString: the text is padded exactly as for a String, every existing attribute range stays on the same scalars, and the appended padding carries no attributes."

### 5. Six-columns claim verified only indirectly
UNIT:      man-page:strings/padRight
CLAIM:     "padRight("日本", 4, "-") is four scalars but six columns"
VERDICT:   UNVERIFIED (consistent with evidence; not a defect)
EVIDENCE:  `term::measureText` raises ErrUnsupported without an active Console TUI (`mfb man term measureText`), which the sandbox has no tty for. Indirect check in `p_probe2`: `padRightToWidth("日本", 6, "-")` printed `日本--` and `padRightToWidth("日本", 4, "-")` printed `日本`, so the column measure of `日本--` is six and of `日本` is four; `padRight("日本", 4, "-")` printed `日本--`. The claim holds against padRightToWidth's measure; the measureText number itself was not run.
SUGGESTED: no wording change; main thread may run `term::measureText("日本--")` under a tty if it wants the direct number.

Everything else on the page checked out against the binary. Both examples compiled and ran from `p_ex1`/`p_ex2` and printed `[42   ]`, `42000`, `name......value`, `id........7`, matching the page. Verified by `p_probe`: width 0 never pads; empty value pads to width; equal/over-width value returned unchanged and never truncated; the original variable is unchanged after the call; negative width, empty padChar, two-scalar padChar and a decomposed `e`+U+0301 padChar all raise 77050002; a composed `é` and a CJK or emoji padChar each count one scalar per copy; a combining sequence in value counts two scalars; the default padChar is a space. No banned memory vocabulary and no borrow-model sentence found on the rendered page; "copy", "mutated" and "value" are the only model words used.
