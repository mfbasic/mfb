### 1. `count` parameter description omits the negative-count outcome
UNIT:      man-page:strings/right
CLAIM:     "The number of trailing Unicode scalar values to take. Must be `0` or greater; values at or above the scalar length of `value` yield the whole string."
VERDICT:   incomplete
EVIDENCE:  `.ai/man-content.md` §2 requires each parameter description to say what happens at zero/empty/negative. The description says "must be 0 or greater" but not what a violation does; the reader has to find it three paragraphs into Description. Probe `p_neg/src/main.mfb` (`io::print(strings::right("hello", -1))`) built with the release `mfb` and run printed `Error: 7-705-0002 / Message: Argument value is not valid for the requested operation.` with exit 255, matching `gen_left_right.rs:lower_strings_left_right_window`'s `raise_error(..., "ErrInvalidArgument")` on `count < 0`. Zero is likewise not mentioned in the parameter row (probe printed `[]` for `right("hi", 0)`).
SUGGESTED: "The number of trailing Unicode scalar values to take. `0` returns the empty string; a count at or above the scalar length of `value` returns the whole string; a negative count raises `ErrInvalidArgument`."

Everything else on the page checked out against the binary. Probes under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-right`, each built as an `mfb init` project with the release `mfb` and run:

- Example 1 printed `llo`, `hi`, `[]` as the page implies (suffix, clamp, zero).
- Example 2 printed `😀c` (multi-byte scalar counts as one).
- Edge probe: `right("", 3)` and `right("", 0)` both printed `[]`; `right("hello", 5)` printed `hello` (inclusive bound, confirms "at or above"); `right("héllo", 4)` printed `éllo` (no scalar split); original `hello` unchanged after the call (no mutation). For `"ae\u{301}"` (4 bytes, 2 graphemes), `right(s, 1)` returned a 2-byte string, i.e. the lone combining mark, confirming "right can cut a cluster in half".
- AttributedString probe: `strings::right(astrings::fromString("hello world"), 5)` compiled, typed as `AttributedString`, and `toString` printed `world`, confirming the overload paragraph; `astrings/helper_right.rs` delegates to `strings::right` so the clamp and negative-count rules are identical.
- `strings::mid` declares only `ErrIndexOutOfRange` (`func_mid.rs` `errors: vec!["ErrIndexOutOfRange"]`), so the contrast sentence is accurate.
- Vocabulary: no banned words, no ownership/borrow model taught; "new String" and "not mutated" are permitted vocabulary. No compiler internals on the page.
- UNVERIFIED: that `ErrOutOfMemory` is actually raisable from this call. It is declared in the descriptor and the lowering materializes a fresh string, but I could not trigger an out-of-memory condition in a probe.
