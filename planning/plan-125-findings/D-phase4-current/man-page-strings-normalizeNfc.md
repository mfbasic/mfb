Both page examples compile and print what the page implies (`TRUE`; `5` then `4`). The probes against the release binary turned up the following.

### 1. "Composed" scalars are not always carried through unchanged
UNIT:      man-page:strings/normalizeNfc
CLAIM:     "Scalars already in composed form, and scalars with no canonical composition, are carried through unchanged."
VERDICT:   wrong
EVIDENCE:  probe2 (`/tmp/plan-125-scratch/D-phase4-current/man-page-strings-normalizeNfc/probe2.mfb`, strings built with `strings::fromScalars` so the editor could not pre-normalize them) prints:
```
angstrom singleton U+212B: in 8491 | out 197 | len 1->1 bytes 3->2
ohm singleton U+2126: in 8486 | out 937 | len 1->1 bytes 3->2
kelvin U+212A: in 8490 | out 75 | len 1->1 bytes 3->1
excl U+0958 precomposed: in 2392 | out 2325 2364 | len 1->2 bytes 3->6
CJK compat U+F900: in 63744 | out 35912 | len 1->1 bytes 3->3
```
U+212B ANGSTROM SIGN is a single composed-looking scalar and comes out as U+00C5; U+212A KELVIN SIGN becomes ASCII `K`; U+0958 is a precomposed letter that NFC *decomposes* and never recomposes (composition exclusion); U+F900 is a CJK compatibility ideograph that NFC replaces with U+8C48 despite the word "compatibility" in its name. probe3 (literal inputs, constant-folded at compile time) gives the same answers, so both paths agree.
SUGGESTED: "A scalar that is already the preferred composed form, such as `U+00E9`, is carried through unchanged. A few scalars that look composed are replaced: singletons such as `U+212B` ANGSTROM SIGN become their canonical equivalent (`U+00C5`), `U+212A` KELVIN SIGN becomes `K`, the CJK compatibility ideographs become their unified ideograph, and the composition-excluded letters such as `U+0958` are split into base plus mark and stay split."

### 2. Length change is described as shrink-only; it can grow
UNIT:      man-page:strings/normalizeNfc
CLAIM:     "Normalization can change the length of the string in both scalars and bytes, since a base scalar plus its combining marks may collapse into one scalar."
VERDICT:   incomplete
EVIDENCE:  probe2 output above: `U+0958` goes from 1 scalar / 3 bytes to 2 scalars / 6 bytes; `e U+0344` goes from 2 scalars / 3 bytes to 2 scalars / 4 bytes (U+0344 splits into U+0308 U+0301, the first composes onto `e`); the singletons shrink bytes without changing scalar count. The second example's caption "Normalization can shorten the scalar count" reinforces the shrink-only reading. A developer sizing a buffer or a fixed-width column from `len(value)` will be wrong.
SUGGESTED: "Normalization can change the length of the string in both scalars and bytes, in either direction: a base scalar plus its combining marks usually collapses into one scalar, but a composition-excluded scalar such as `U+0958` splits into two, and a singleton such as `U+212B` keeps its scalar count while its byte count shrinks. Measure the result, not the input."

### 3. ErrOutOfMemory is in the table but its condition is never stated
UNIT:      man-page:strings/normalizeNfc
CLAIM:     Errors table row "77010001 ErrOutOfMemory Allocation failed." with no sentence in the Description naming when it is raised.
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/strings/func_normalize_nfc.rs:register` lists `errors: vec!["ErrOutOfMemory"]`; the `DESC` constant in the same file contains no mention of the error. §2 of `.ai/man-content.md` requires "every raisable error and the condition that raises it, consistent with the auto-derived Errors table". Sibling `strings::fromScalars` states it ("The only error it can raise is ErrOutOfMemory, when the result string cannot be created; no input value is ever rejected"), so the omission is inconsistent within the package.
SUGGESTED: Add to the Description: "The only error it can raise is `ErrOutOfMemory`, when the result string cannot be created. No input value is ever rejected."

### 4. "rather than from the table" is an implementation detail
UNIT:      man-page:strings/normalizeNfc
CLAIM:     "Hangul is composed algorithmically rather than from the table."
VERDICT:   out-of-scope
EVIDENCE:  "the table" refers to the generated composition table the lowering consults (`emit_hangul_composition_attempt` then `compose_try_tables` in `func_normalize_nfc.rs:lower_out`). A developer at the terminal has not been told about any table and gains nothing from the contrast; the developer-visible fact is only that Hangul jamo sequences compose, which probe2 confirms (`U+1100 U+1161` → `U+AC00`, `U+AC00 U+1168` → `U+AC01`).
SUGGESTED: "Hangul jamo sequences compose into precomposed syllables as well." (or drop the sentence; the preceding "wherever a canonical composition exists" already covers it)

### 5. Unicode version not stated
UNIT:      man-page:strings/normalizeNfc
CLAIM:     "The transformation is deterministic and locale-independent."
VERDICT:   incomplete
EVIDENCE:  Which scalars compose depends on the Unicode version of the tables; `src/codegen/memory/data/data_objects/unicode_tables.rs` pins them to Unicode 16.0.0 (comment "pinned to Unicode 16.0.0", and `src/docs/spec/unicode/01_tables-and-algorithms.md:49`). The page never says which version, so a developer comparing against another system's NFC output cannot know whether a mismatch is a bug.
SUGGESTED: "The transformation is deterministic and locale-independent, and follows the Unicode 16.0.0 tables."

No banned-vocabulary hits: `./scripts/man-census.sh --memory-scope strings` reports no line for this page. The AttributedString claim is confirmed by aprobe (`**Cafe**́ bold` in, `Café bold` out with the bold span gone, and the original left as it was).
