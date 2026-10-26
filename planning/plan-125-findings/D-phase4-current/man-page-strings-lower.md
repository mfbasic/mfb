### 1. Final-sigma behaviour is a sharp edge the page leaves unstated
UNIT:      man-page:strings/lower
CLAIM:     "The mapping is applied per Unicode scalar value across the whole string, applying the Unicode full lowercase mapping."
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-lower/p_probe/src/main.mfb` printed `οδοσ` for `strings::lower("ΟΔΟΣ")` and `TRUE` for `= "οδοσ"`. The lowering (`src/codegen/builtins/strings/gen_case_map.rs:lower_strings_case_map_out`, table lookup via `gen_strings_support.rs:emit_case_map_lookup`) has no Final_Sigma context rule. Every mainstream runtime a developer is likely to have used (Rust `to_lowercase`, Python, Java, ICU) yields `οδος` with final ς here, so this is exactly the "sharp edge stated explicitly" case §2 of `.ai/man-content.md` demands. The page's "per scalar" wording technically implies it but never says what a reader will actually hit.
SUGGESTED: Add after the Turkish sentence: "Nor is Unicode's final-sigma context rule applied: Greek capital sigma always lowercases to σ, never to word-final ς, so `strings::lower("ΟΔΟΣ")` is `"οδοσ"`."

### 2. The type is spelled in a form that does not compile
UNIT:      man-page:strings/lower
CLAIM:     "value may also be an astrings::AttributedString: it returns an AttributedString whose text is transformed as above"
VERDICT:   misleading
EVIDENCE:  `LET a AS astrings::AttributedString = ...` fails to compile: `error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: Built-in package 'astrings' does not export 'astrings.AttributedString'` (probe `p_attr`, first version). The type is the unqualified built-in `AttributedString`; `LET a AS AttributedString` compiles and `strings::lower(a)` printed `hello world`. The `astrings` overview and `astrings::addAttribute`'s Declaration both spell it `AttributedString`. Note: `grep -rhoc 'an \`astrings::AttributedString\`' src/codegen/builtins/strings/*.rs` sums to 34 sibling descriptors with the same spelling, so the fix is tree-wide, but this page is in scope.
SUGGESTED: "`value` may also be an `AttributedString` (see `mfb man astrings`): the result is an `AttributedString` whose text is transformed as above…"

### 3. The stated reason for dropping attributes reads as conditional; the drop is unconditional
UNIT:      man-page:strings/lower
CLAIM:     "but attributes are dropped — the mapping changes the scalar count within a span, so the overlay cannot be remapped."
VERDICT:   misleading
EVIDENCE:  Probe `p_attr` (final version): `addAttribute(fromString("HELLO World"), 0, 4, bold())` renders via `toMarkdown` as `**HELLO** World`; after `strings::lower` it renders as `hello world` with no bold, even though the scalar count did not change (pure ASCII, 11 → 11). A reader could take the "because the count changes" clause to mean styling survives when the length is preserved. It never does.
SUGGESTED: "but **all attributes are dropped**, even when the text length is unchanged — because a case mapping can change the scalar count within a span, the overlay is never carried over."

Everything else on the page checked out against the binary. Both Examples compiled with `mfb build` (plain, not `--app`, whose bundle emits nothing on stdout) and printed `hello` / `abc-123` and `1` / `2` as the page implies. The probe also confirmed: empty string returns empty (`[]`), the argument is not mutated (`ABC abc`), `"I"` lowercases to `"i"` with no Turkish tailoring, `"İ"` equals `"i\u0307"`, `A + U+0301` stays two scalars (`2`), and `ßé€` passes through unchanged. The single Errors row (`ErrOutOfMemory`) matches the descriptor's `errors: vec!["ErrOutOfMemory"]` in `src/codegen/builtins/strings/func_lower.rs:register`. No banned memory vocabulary and no borrow/ownership framing found; "copied through unchanged" and "a new String" use only permitted words.
