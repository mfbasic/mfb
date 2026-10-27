### 1. "bare `mid` name" is not callable bare
UNIT:      man-page:strings/mid
CLAIM:     "The bare mid name is also defined for lists; see mfb man collections mid."
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase4/man-page-strings-mid/p_bare2.bas` (`IMPORT collections`, then `mid(xs, 1, 2)` on a `List OF Integer`) fails to compile: `error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: Callable 'mid' is not a top-level function.` The same on a String (`p_bare.bas`, `mid("hello", 1, 3)`) fails identically. Only the qualified `collections::mid(xs, 1, 2)` compiles and runs (prints `2`). A developer reading "bare mid name" will type an unqualified call, which is rejected; the shared name is a fact about the two *qualified* members, not a bare spelling.
SUGGESTED: "`collections::mid` slices a list the same way; see `mfb man collections mid`."

Everything else on the page verified against the binary at HEAD. Probes under `/tmp/plan-125-scratch/D-phase4/man-page-strings-mid`, each compiled with `mfb build` and run:

- The three examples print `ell` / `[]`, `😀é`, and `out of range`, as the page implies.
- `mid("hello",-1,2)`, `mid("hello",1,-1)`, `mid("hello",5,1)`, `mid("hello",6,0)`, `mid("",0,1)`, and `mid("hello",1,9223372036854775807)` (64-bit overflow) all raise `77050001 ErrIndexOutOfRange`, matching the parameter ranges, the "does not clamp" paragraph, the overflow sentence, and the single Errors row (lowering: `lower_mid` in `src/codegen/collection/search/builder_search.rs`, `invalid_range` branches).
- `mid("",0,0)` and `mid("hello",5,0)` return `""`; `mid("hello",0,5)` returns `hello`. `strings::left("hello",10)` and `strings::right("hello",10)` return `hello` uncut, confirming the clamp contrast.
- `"e\u0301x"` has `len` 3 and 2 graphemes; `mid(s,0,1)` returns bare `e`, confirming the "can cut a cluster in half" sentence.
- AttributedString overload: `strings::mid(bold-on-"h" "hello", 1, 3)` yields `ell`, `toMarkdown` gives `**e**ll` (span remapped), and `strings::mid(a, 3, 3)` raises `77050001`, matching the last paragraph (helper `__astrings_mid` in `src/codegen/builtins/astrings/helper_mid.rs` delegates to the String overload).
- No banned memory vocabulary or ownership-model sentences on the page; "value is not mutated; the result is a new String" uses only permitted words.
