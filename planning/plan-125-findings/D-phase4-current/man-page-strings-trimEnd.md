### 1. "a new String, even when nothing was trimmed" is an unobservable identity claim
UNIT:      man-page:strings/trimEnd
CLAIM:     "value is not mutated; the result is a new String, even when nothing was trimmed."
VERDICT:   out-of-scope
EVIDENCE:  Probe `p_ws` (`/tmp/plan-125-scratch/D-phase4-current/man-page-strings-trimEnd/p_ws/src/main.mfb`) confirms `value` is unchanged after the call (every `v_*` byteLen printed matches its literal, e.g. `interior 9 -> 7`). But whether the result is "a new String" when nothing was trimmed is not observable by any MFBASIC program: a String is a value, and two variables never share one (`.ai/man-content.md` §4.1), so "new" versus "the same" has no developer-visible meaning. The clause only answers a question a reader with a pointer/identity mental model would ask, which is the §7 failure mode without a banned word. The first half of the sentence is the sharp edge §2 requires and should stay. (Minor; the identical sentence is on `strings::trim`, so fix both together.)
SUGGESTED: "value is not mutated; the call returns the trimmed text and leaves value as it was."

Everything else on the page checked true at HEAD:

- Both examples compile and run (`mfb build p_ex1` / `p_ex2`), printing `[  Hello]` and `[value]` exactly as implied; the lexer honours `\n` (`src/lexer/literals.rs`, the `'n' => value.push('\n')` arm).
- The whitespace set in `gen_trim.rs:emit_trim_end_backward` is byte-for-byte the 25 White_Space scalars the page lists, and the `p_ws` probe strips every one of them (`ascii 8 -> 1`, `nel_nbsp 5 -> 1`, `ogham 4 -> 1`, `quads 10 -> 1`, `sep 10 -> 1`, `mmsp_ideo 7 -> 1`, `mixed_trailing_mixed 12 -> 1`) while keeping the non-White_Space look-alikes U+200B, U+180E, U+FEFF, U+2060 (`4 -> 4` each) and a trailing `é` before a space (`eacute_kept 4 -> 3`, no split scalar).
- Interior whitespace and leading whitespace preserved (`interior 9 -> 7` yields `"  a b\nc"`), all-whitespace yields empty (`allws 5 -> 0`), empty yields empty (`empty 0 -> 0`), leading U+3000 kept (`multi_ideo_then_x 10 -> 4`).
- AttributedString overload: `strings.trimEnd` maps to `__astrings_trimEnd` (`src/codegen/builtins/strings/mod.rs`, the `("strings.trimEnd", "__astrings_trimEnd")` entry), and probe `p_attr` printed `[  hi there]` with markdown `  **hi** the**re**` from an input styled `  **hi** the**re   **`, so the trailing span is clipped by the same edit.
- Errors table: the lowering's only failure path is `emit_materialize_string_from_bytes`, matching the single `ErrOutOfMemory` row.
- No banned §4 vocabulary on the rendered page (only "mutated", "new", "copy"-free prose).
