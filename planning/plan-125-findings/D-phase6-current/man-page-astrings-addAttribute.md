### 1. No See also row derives, and the sibling cross-references are missing
UNIT:      man-page:astrings/addAttribute
CLAIM:     "when their starts are equal, the one added later wins (see getAttributes)."
VERDICT:   incomplete
EVIDENCE:  Rendered page (`mfb man astrings addAttribute`) has no "See also" section. `src/cli/man.rs:render_see_also` builds the rows from `referenced_functions(function.desc, …)`, which only matches `pkg::name` spellings; the desc writes `getAttributes` unqualified, and the only qualified reference (`astrings::bold()`) sits in a parameter desc, which is not scanned. Compare `mfb man strings mid`, which renders four See also rows. The page also never names `astrings::removeAttribute` or `astrings::clearAttributes`, the siblings a developer reaches for after adding a span (`mfb man astrings` lists both).
SUGGESTED: "… when their starts are equal, the one added later wins (see `astrings::getAttributes`). To take a span back, use `astrings::removeAttribute`; to drop every attribute over a range, use `astrings::clearAttributes`." Qualifying the names makes the See also rows derive.

### 2. "scalar" is never pinned to Unicode scalar, so the grapheme sharp edge is unstated
UNIT:      man-page:astrings/addAttribute
CLAIM:     "The first scalar index of the inclusive range (0-based); zero or more and no greater than endIndex."
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase6-current/man-page-astrings-addAttribute/probe/src/main.mfb` on `fromString("e\u{0301}x")` (two graphemes, three scalars): `len(strings::toScalars(...))` printed `3`; `addAttribute(g, 0, 2, bold())` printed `ok 0,2`; `addAttribute(g, 0, 3, bold())` printed `raise 0,3 -> 77050001 attribute range out of bounds`. Indices count Unicode scalars, not graphemes, and a combining mark takes its own index. The page says "scalar" throughout but never "Unicode scalar" and never warns that one visible character can occupy two indices; §2 of `.ai/man-content.md` lists "Unicode scalar vs. grapheme" as a sharp edge that must be stated.
SUGGESTED: In the `start` desc: "The first Unicode scalar index of the inclusive range (0-based, the same indexing as `strings::mid`, so a character written with a combining mark spans two indices); zero or more and no greater than `endIndex`."

### 3. "guard construction" names the wrong operation
UNIT:      man-page:astrings/addAttribute
CLAIM:     "even 0, 0 is out of bounds on an empty AttributedString, so guard construction from text that might be empty."
VERDICT:   misleading
EVIDENCE:  Probe: `fromString("")` succeeds (no raise), and only the subsequent `addAttribute(e, 0, 0, bold())` printed `raise 0,0 -> 77050001 attribute range out of bounds`. Construction never fails; it is the `addAttribute` call that must be guarded.
SUGGESTED: "… even `0, 0` is out of bounds on an empty `AttributedString`, so check the text is non-empty before calling `addAttribute` on it."

Everything else checked out against HEAD. The example compiled with `mfb build -q ex` and ran with exit 0 and no output, as the page implies. The probe confirmed: the original value is left unchanged (0 attributes at index 2 before, 1 after); the range is inclusive (`0, 4` covers index 4 and not 5; `5, 5` is accepted; `0, 10` on 11 scalars is accepted and `0, 11` raises); higher start wins over a later-added lower start (fontSize 20 at index 3 beats fontSize 30 added later from 0); equal starts resolve to the later addition (index 0 reads 30); `start < 0` or `endIndex < start` raises 77050002 "invalid attribute range" and `endIndex >= length` raises 77050001 "attribute range out of bounds", matching `helper_validate_range.rs:__astrings_validateRange`. The banned-word scan over the rendered page hit only "Allocation failed." in the derived Errors table, which is the §4.4 carve-out.
