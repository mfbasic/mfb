### 1. "Never fails" contradicts the page's own Errors table
UNIT:      man-page:strings/stripPrefix
CLAIM:     "The function is total and never fails."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/strings/func_strip_prefix.rs:register` declares `errors: vec!["ErrOutOfMemory"]`, and the rendered page (`mfb man strings stripPrefix`) shows a one-row Errors table for `ErrOutOfMemory` and ends the description with "`ErrOutOfMemory` means the program ran out of memory." The same page says the function never fails and, two paragraphs later, names the error it can fail with. "Total" is also a maths/compiler word, not a developer word.
SUGGESTED: "No input is rejected: an empty `value`, an empty `prefix`, a `prefix` longer than `value` and a non-matching `prefix` all succeed. The only error is `ErrOutOfMemory`, raised when the program runs out of memory." — and delete the stray `ErrOutOfMemory` sentence from the AttributedString paragraph (see finding 3).

### 2. "A new String back, even on the unchanged path" teaches a copy-vs-shared identity model
UNIT:      man-page:strings/stripPrefix
CLAIM:     "Neither operand is modified, and you always get a new String back, even on the unchanged path."
VERDICT:   out-of-scope
EVIDENCE:  Probe `p_basic` (`io::print(r)` then `io::print(v)` after `LET r = strings::stripPrefix(v, "foo")`) printed `foobar` then `foofoobar`, and `strings::stripPrefix("abc", "zz") = "abc"` printed `TRUE`. Those are the only developer-visible facts. Whether the unchanged result is "new" or the same storage as `value` is not observable in MFBASIC — two variables never share one value (§4.1) — so "a new String … even on the unchanged path" only carries meaning to a reader thinking about allocation and identity. This is the §7 case: no banned word, but a fresh-vs-shared mental model. `gen_strip.rs:lower_strings_strip` does call `emit_materialize_string_from_bytes` on both paths, so it is true, and still not a developer fact. "Operand" is also the wrong word for a function argument.
SUGGESTED: "Neither argument is changed. The result is a value of its own; when there is nothing to strip it is equal to `value`."

### 3. The `ErrOutOfMemory` sentence is attached to the AttributedString paragraph
UNIT:      man-page:strings/stripPrefix
CLAIM:     "value may also be an AttributedString: it returns an AttributedString whose text is transformed exactly as the String overload's and whose attribute spans are remapped by the same edit. ErrOutOfMemory means the program ran out of memory."
VERDICT:   misleading
EVIDENCE:  Rendered page, last description paragraph. The error sentence sits as the second sentence of the paragraph about the `AttributedString` form, so it reads as if out-of-memory is specific to that form. `errors: vec!["ErrOutOfMemory"]` in `func_strip_prefix.rs:register` is declared on the `String` implementation. Also "the String overload's" — "overload" is compiler vocabulary; the page should just say "the String form".
SUGGESTED: Move the error sentence into the paragraph proposed in finding 1, and rewrite this one: "`value` may also be an `AttributedString`. The result is then an `AttributedString` whose text is stripped exactly as described above, with the attribute spans moved to match (see finding 4 for what happens to spans on the removed text)."

### 4. Spans on the removed prefix are dropped or cut; the page does not say so
UNIT:      man-page:strings/stripPrefix
CLAIM:     "… whose attribute spans are remapped by the same edit."
VERDICT:   incomplete
EVIDENCE:  Probe `p_spans`: `"foofoobar"` with bold on scalars 0–3 and italic on 1–2, then `strings::stripPrefix(a2, "foo")`. `astrings::toMarkdown` printed `**f*****oo*****f**oobar` before and `**f**oobar` after. The italic span (entirely inside the removed `foo`) is gone, and the bold span 0–3 was cut to the one surviving scalar. A non-matching prefix (`"zzz"`) returned the spans untouched. "Remapped" does not tell a developer that attributes on the removed text are discarded and a straddling span is truncated, which is the sharp edge of this form (`src/codegen/builtins/astrings/helper_strip_prefix.rs`: `__astrings_windowSpans(…, n - kept, n - 1)`).
SUGGESTED: "Attribute spans that lie entirely on the removed prefix are dropped, a span that straddles the cut keeps only its part on the remaining text, and every other span shifts left by the number of scalars removed."

NO other findings. Both examples compiled and ran under `mfb build` (projects `p_ex1`, `p_ex2`): the first printed `example.com` twice, the second printed `foobar`, as the page implies. Probe `p_basic` confirmed: repeated prefix strips one copy (`foobar`), empty prefix returns `abc`, empty value returns empty, prefix longer than value returns `ab`, `"ABC"`/`"abc"` is left unchanged (no case folding), `"éx"` with prefix `"e"` is left unchanged (no normalization). Probe `p_comb` confirmed stripping `"e"` from `"e\u{0301}x"` leaves a 3-byte string of 2 graphemes beginning with the lone combining accent, exactly as the page states.
