All probes are done. Every description sentence checked out against the binary; one parameter description over-promises. Here is the report.

### 1. `value` description promises 0 for an empty value, but an empty needle wins
UNIT:      man-page:strings/count
CLAIM:     "The string to scan. May be empty, in which case the result is `0`."
VERDICT:   misleading
EVIDENCE:  Lowering in `src/codegen/builtins/strings/func_count.rs:lower` tests `needle` length for zero and branches to `strings_count_invalid` before the value/needle length comparison that yields 0. Probe `/tmp/plan-125-scratch/D-phase4/man-page-strings-count/probe_emptyboth.bas` (`strings::count("", "")`) printed `Error: 7-705-0002 Argument value is not valid for the requested operation.` with exit 255, not `0`. The description body has the same ambiguity ("A needle longer than value yields 0, as does an empty value. An empty needle is refused…") without saying which rule wins when both apply.
SUGGESTED: Parameter: "The string to scan. May be empty; with a non-empty `needle` the result is then `0`." Description: "A `needle` longer than `value` yields `0`, as does an empty `value` — unless `needle` is also empty, in which case the empty-needle check below wins and the call raises."

Everything else on the page held. Audit trail for the sentences that are correct:

- **Examples** (`ex1.bas`, `ex2.bas` staged as `mfb init` projects and built with the P-125 release binary): printed `2`, `0` and `3`, `1`, matching the page's "abcabc"/"bc", "xyz"/"a", "aaa"/"a", "aaa"/"aa" claims.
- **`probe1.bas`** printed: `longer: 0`, `emptyval: 0`, `nfc/nfd: 0`, `nfc/nfc: 1`, `case: 1`, `multibyte: 3`, `replace aaa/aa: xa`, `replace abcabc/bc: a-a-`, `contains empty: TRUE`, `find empty: 0`, `regex count aaa/aa: 1`, `regex count aaa/a: 3`. That confirms the longer-needle rule, empty-value rule, no normalization, no case folding, scalar-boundary counting, the replace-parity claim, the contains/find-answer-for-empty claim, and the regex::count leftmost-first non-overlapping claim.
- **Empty-needle refusals**: `probe_empty.bas` (`count("abc","")`), `probe_split.bas` (`split("abc","")`), `probe_regex_empty.bas` (`regex::count("abc","")`) and `probe_replace_empty.bas` all raised `7-705-0002`, matching the page and the Errors table row (code 77050002, message text verified against `src/docs/spec/diagnostics/02_error-codes.md` line 100).
- **AttributedString overload**: `probe_astr.bas` (`count(astrings::fromString("abcabc"), "bc")`) printed `2`; `probe_astr2.bas` with an empty needle raised `7-705-0002`, confirming "same value, type, and errors".
- **"see mfb man strings for the rule"**: `mfb man strings` has a section "The empty needle" stating the count/rewrite-refuses rule verbatim.
- **Scope and vocabulary**: no registry/lowering/Rust terms; "Neither operand is modified" and "scalar" are within the permitted vocabulary; no borrow/ownership model is taught.
