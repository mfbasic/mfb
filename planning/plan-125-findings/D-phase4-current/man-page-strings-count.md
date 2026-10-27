Every probe agrees with the page. One wording issue survives.

### 1. "needle is a literal" reads as "must be a string literal"
UNIT:      man-page:strings/count
CLAIM:     "needle is a literal. To count matches of a pattern instead, use regex::count, ..."
VERDICT:   misleading
EVIDENCE:  In MFBASIC prose "literal" normally means a source-text constant (`"abc"`), so a reader can take this as "needle must be a string literal". The binary accepts any run-time String: probe `p_edges/src/main.mfb` did `LET n AS String = "aa"` then `strings::count(v, n)` and printed `unchanged:aaa/aa/1`. The intended sense is "matched literally, not as a pattern", which the lowering confirms: `func_count.rs:lower` compares bytes with `emit_string_byte_range_equal_branch`, no pattern engine.
SUGGESTED: "needle is matched literally, character for character, not as a pattern. To count matches of a pattern instead, use regex::count, ..."

Everything else on the page is true of the binary at HEAD. Audit trail of what was run under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-count` with the P-125 release `mfb`:

- **Both examples** compiled and printed exactly what the page implies: `2`, `0` and `3`, `1`.
- **Edge probe** (`p_edges`): needle longer than value gives `0`; empty value gives `0`; NFC "é" versus NFD "e"+U+0301 gives `0`; `"ABC"`/`"abc"` gives `0`; `"ééé"`/`"é"` gives `3`; needle equal to value gives `1`; `strings::replace("aaa","aa","X")` gives `Xa`, matching the single occurrence count reports; `strings::contains("abc","")` is `TRUE`; `strings::find("abc","")` is `0`; `regex::count("aaa","aa")` is `1`; both operands unchanged after the call.
- **Empty needle** (`p_emptyneedle`) and **both empty** (`p_emptyboth`) raise `7-705-0002` "Argument value is not valid for the requested operation", confirming the empty-needle rule wins over the empty-value rule, as the page says. `regex::count("abc","")` and `strings::split("abc","")` raise the same error, confirming the sibling claims.
- **AttributedString overload** (`p_attr`): `strings::count(astrings::fromString("aaa"), "aa")` prints `1`, and an empty needle raises the same ErrInvalidArgument, matching "same value, type, and errors".
- **Lowering** `src/codegen/builtins/strings/func_count.rs:lower` checks needle length zero first, then needle longer than value, then scans leftmost with a byte-exact compare and advances past the whole needle on a match, which is the non-overlapping, leftmost-first, no-normalization behaviour the page describes. The `mfb man strings` overview's "The empty needle" section states the rule the page cites.
- **Scope and vocabulary**: no banned words, no internals, no sentence teaching a borrow or ownership model. "Neither operand is modified" and "scalar" are the permitted framing.
