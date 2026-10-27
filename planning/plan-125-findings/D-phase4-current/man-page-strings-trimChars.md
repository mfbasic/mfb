### 1. Grapheme-splitting sharp edge is not stated
UNIT:      man-page:strings/trimChars
CLAIM:     "Membership is tested on whole Unicode scalars rather than bytes, so a multi-byte scalar listed in chars matches correctly and trimming never splits a scalar."
VERDICT:   incomplete
EVIDENCE:  The sentence is true, but the page never says the converse: trimming operates below the grapheme level and can split a grapheme cluster, leaving a bare combining mark. Probe (built with `mfb init` + `mfb build -q`, source at /tmp/plan-125-scratch/D-phase4-current/man-page-strings-trimChars/probe.mfb):
```
io::print("[" & strings::trimChars("e\u0301a", "e") & "]")   ' decomposed é = e + U+0301
io::print("[" & strings::trimChars("éa", "e") & "]")         ' precomposed U+00E9
```
printed
```
[́a]      <- base 'e' removed, orphan U+0301 left at the front
[éa]     <- precomposed scalar untouched, as the page says
```
Lowering confirms per-scalar stepping with no cluster check: `src/codegen/builtins/strings/func_trim_chars.rs:window` advances one lead byte plus continuation bytes and tests each such scalar via `gen_strings_support.rs:emit_chars_set_contains_branch`. The standard (`.ai/man-content.md` §2, "Sharp edges, stated explicitly: ... Unicode scalar vs. grapheme") requires the page to say which, and the consequence. The same probe also shows a decomposed scalar in `chars` is two set members, not one character.
SUGGESTED: Append to that paragraph: "Scalars, not graphemes: a base letter listed in `chars` is removed even when a combining mark follows it, and the mark is then left at the edge of the result. A decomposed character in `chars` (such as `e` plus U+0301) contributes two set members, not one. To work with user-perceived characters, see `strings::graphemes`."

All other sentences verified true at HEAD. Both page examples compiled and ran, printing `hello` / `Hello` and `a-b` / `hello`. The probe also confirmed: empty `value` and all-member `value` give `""`; empty `chars` returns `value` unchanged; ordering and repetition in `chars` are irrelevant (`"abcab"` with `"ba"` and `"bbbaaa"` both give `c`); interior members are preserved; multi-byte scalars (`é`, `日本`, `🙂`) are matched whole; comparison is case-literal (`"XxhelloxX"` with `"x"` unchanged); the original binding is not mutated (`zzqzz q`); and the `AttributedString` overload compiles and returns the trimmed text. The Errors table lists only `ErrOutOfMemory`, matching the descriptor's `errors: vec!["ErrOutOfMemory"]`. No banned memory vocabulary appears on the rendered page outside the derived "Allocation failed." error-message cell, which is carve-out 2.
