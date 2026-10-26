Probes are complete. Every sentence on the page checked out against the binary except one omitted sharp edge, which I found with the last probe.

### 1. Page says trimming never splits a scalar, but it can split a grapheme
UNIT:      man-page:strings/trimStart
CLAIM:     "Multi-byte whitespace scalars are matched whole, so trimming never splits a scalar."
VERDICT:   incomplete
EVIDENCE:  The sentence is true, but the page never states the adjacent sharp edge that item 5 of this pass asks for: whitespace is removed scalar by scalar, so a leading space that is the base of a grapheme cluster is removed and its combining marks stay behind at the front of the result. Probe `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-trimStart/graph.mfb`, built with `mfb build -q graph` and run:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET v AS String = " " & strings::fromScalars([`\u{301}`]) & "x"
  LET t AS String = strings::trimStart(v)
  io::print("in bytes=" & toString(strings::byteLen(v)) & " out bytes=" & toString(strings::byteLen(t)))
  io::print("in graphemes=" & toString(strings::graphemesCount(v)) & " out graphemes=" & toString(strings::graphemesCount(t)))
  RETURN 0
END FUNC
```

prints

```
in bytes=4 out bytes=3
in graphemes=2 out graphemes=2
```

The space (base of the cluster "space + U+0301") is removed and the result begins with a bare U+0301. The lowering confirms this is by design: `gen_trim.rs:lower_strings_trim_window` advances by `emit_unicode_whitespace_branch`'s scalar width and never consults grapheme boundaries. The page says "by Unicode scalar, not by byte" but never says "not by grapheme", and the only grapheme-adjacent sentence ("never splits a scalar") reads as reassurance that nothing is split.
SUGGESTED: Append to that paragraph: "Trimming works on scalars, not graphemes: a leading space followed by a combining mark loses the space, and the combining mark stays at the front of the result."

Everything else on the page is verified true at HEAD. For the audit record: the two examples were copied verbatim into `ex1/src/main.mfb` and `ex2/src/main.mfb` under the scratch directory, built with `mfb build -q`, and printed `[Hello  ]` and `[wide　]` (trailing U+3000 kept), matching the prose. The whitespace-set probe `probe.mfb` trimmed every one of the 25 listed White_Space scalars (U+0009 through U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000, U+2005, U+200A, U+2028, U+2029, U+202F, U+205F, U+3000) and left U+200B, U+180E, U+FEFF, U+0008 and U+001F in place, which matches the branch table in `emit_unicode_whitespace_branch` (src/codegen/string/unicode_props/mod.rs). The same probe showed the empty string and an all-whitespace string both yield the empty string, interior and trailing spaces and line breaks survive byte for byte, and the original binding is unchanged after the call. The AttributedString probe `astr.mfb` bolded "hello" at scalars 3 to 7 of "   hello world  ", trimmed it, and printed `**hello** world  ` with the span correctly shifted, confirming the overload sentence and `__astrings_trimStart` in src/codegen/builtins/astrings/helper_trim_start.rs. The Errors table's single row is derived from `errors: vec!["ErrOutOfMemory"]` in func_trim_start.rs and its "Allocation failed." cell is the sanctioned carve-out 2. No banned vocabulary, no compiler-internal sentences, and no borrow or ownership framing appear on the rendered page.
