### 1. Parameter row omits the AttributedString acceptance the description relies on
UNIT:      man-page:strings/trim
CLAIM:     "The string to trim. Any `String` is accepted, including the empty string and a string that is entirely whitespace."
VERDICT:   incomplete
EVIDENCE:  The Declaration and the Parameters table (derived from `src/codegen/builtins/strings/func_trim.rs:register`, a single `ParameterType::String` implementation) say `value` is a `String`, and the one hand-written parameter sentence repeats "Any `String`". Only the final paragraph of the Description reveals that an `AttributedString` is also accepted. The overload is real: `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-trim/asp` (`astrings::fromString("  hello world  ")`, bold over inclusive scalars 2–6, then `strings::trim`) printed `[**hello** world]`, `[hello world]`, and with bold over 0–3 printed `[**he**llo world]`, so the span is remapped and clipped as the page says. A reader scanning the table never learns that. Minor, but the parameter description is the one editable place that can say it.
SUGGESTED: "The string to trim: a `String`, or an `AttributedString` (see below). Any value is accepted, including the empty string and a string that is entirely whitespace."

Everything else on the page checked out against the binary. For the record, the probes that back that up:

- **Examples.** Both page examples, copied verbatim into `ex1p` and `ex2p` under the scratch directory and built with `mfb build -q`, printed `Hello` and then `a b` / `wide`, exactly as the page implies.
- **Whitespace set.** `probep` and `vtp` trimmed every one of the 25 scalars the page lists (U+0009–U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000) from both ends of `x`, and left U+200B, U+FEFF, U+180E and U+2060 untouched (`byteLen` stayed 7). That matches the forward predicate in `src/codegen/string/unicode_props/mod.rs:emit_unicode_whitespace_branch` and the backward scan in `src/codegen/builtins/strings/gen_trim.rs:emit_trim_end_backward`, and it is exactly Unicode `White_Space`.
- **Edges.** `"  a \t　 b  "` trimmed to `a \t　 b` (interior preserved byte for byte, including a multi-byte scalar), an all-whitespace string and the empty string both trimmed to empty, and a mixed run of VT/FF/space/U+0085/U+00A0 came off cleanly.
- **No mutation.** `mutp` trimmed a `MUT` binding holding `"  x  "` and printed the original as `[  x  ]` afterwards.
- **Vocabulary and scope.** No banned word and no implementation vocabulary on the rendered page; "new `String`", "not mutated", "copy" usage all stay within the §4 set, and no sentence teaches a borrow or ownership model.
