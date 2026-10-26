### 1. ß does not lengthen the byte count, so "in both scalars and bytes" is not what the example shows
UNIT:      man-page:strings/upper
CLAIM:     "The German sharp s ß uppercases to SS, so upper can return a string that is longer than its input in both scalars and bytes."
VERDICT:   misleading
EVIDENCE:  Probe (`/tmp/plan-125-scratch/D-phase4-current/man-page-strings-upper/probe-proj`, built with `mfb build -q`), printing `len` and `strings::byteLen` of input vs output:
           `eszett` input "straße" len=6 bytes=7 → output "STRASSE" len=7 bytes=7 (scalars +1, bytes unchanged).
           Byte growth does exist but only for other scalars: "ŉ" 2 bytes → "ʼN" 3 bytes; "ᾳ" 3 bytes → "ΑΙ" 4 bytes. And it can shrink: "ﬁ" 3 bytes → "FI" 2 bytes. The sentence hangs "longer … in both scalars and bytes" on the ß case, which is byte-neutral, and never says the byte length can also go down.
SUGGESTED: "The German sharp s ß uppercases to SS, so the result can have more scalars than the input. The byte length can go up or down too: ŉ becomes ʼN (one byte more), while the ligature ﬁ becomes FI (one byte fewer). Never assume len or byteLen is preserved across a case mapping."

### 2. Parameter description says empty is accepted but not what comes back
UNIT:      man-page:strings/upper
CLAIM:     "The string to uppercase. Any String is accepted, including the empty string."
VERDICT:   incomplete
EVIDENCE:  `.ai/man-content.md` §2 requires each parameter to state what happens at empty. Probe `empty` line: `strings::upper("")` → `[]` len=0 bytes=0. The page never states that the empty string maps to the empty string; the reader has to infer it from "every scalar … is mapped".
SUGGESTED: "The string to uppercase. Any String is accepted; the empty string returns the empty string."

### 3. The ErrOutOfMemory sentence is attached to the AttributedString paragraph
UNIT:      man-page:strings/upper
CLAIM:     "… so the overlay cannot be remapped. ErrOutOfMemory means the program ran out of memory."
VERDICT:   misleading
EVIDENCE:  Rendered page (`mfb man strings upper`): the only error sentence is the last sentence of the paragraph that begins "value may also be an astrings::AttributedString", so it reads as a condition specific to the AttributedString overload. `src/codegen/builtins/strings/gen_case_map.rs:lower_strings_case_map_out` raises `ErrOutOfMemory` on both the ASCII quick path and the Unicode path for a plain String; it is not specific to the overload. Verified the overload itself drops attributes as claimed: `p2` probe, `getAttributes(a,0)` has 1 entry before and 0 after `upper`, `toMarkdown` goes from `**hello** ß` to `HELLO SS`, and the original is unchanged.
SUGGESTED: Move the sentence into its own short paragraph before the AttributedString paragraph: "ErrOutOfMemory is raised only when the program runs out of memory while building the result; no input String causes any other error."

All other sentences verified true at HEAD. Both page examples were copied verbatim into `ex1-proj` and `ex2-proj`, built with `mfb build -q`, and printed `HELLO` / `ABC-123` and `STRASSE` / `6` / `7`, matching the page. Probe results backing the remaining claims: ASCII and punctuation pass through (`HELLO, WORLD! 123`); no Turkish tailoring (`"i ı"` → `I I`); combining sequences stay decomposed (`e`+U+0301 → `E`+U+0301, len=2); emoji pass through; `µ` → `Μ`; titlecase `ǅ` → `Ǆ`; the input binding is unchanged after the call (`orig-after [abc]`). Malformed UTF-8 input could not be constructed: `encoding::utf8Decode([104,255,105])` raises `7-705-0003 invalid utf-8`, so the lowering's replacement-character path is not developer-reachable by that route and the page's silence on it is not a defect. No banned memory vocabulary and no borrow/ownership mental model found on the page. No files in the repository were edited.
