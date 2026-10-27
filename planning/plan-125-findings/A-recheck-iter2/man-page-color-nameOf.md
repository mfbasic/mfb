### 1. Implementation rationale about table order leaks into the developer page
UNIT:      man-page:color/nameOf
CLAIM:     "Both spellings still resolve through color::fromName; only the reverse direction has to choose, and choosing by name rather than by table order means the answer cannot change if the table is ever reordered."
VERDICT:   out-of-scope
EVIDENCE:  The second clause is the Rust doc comment on `BODY` in `src/codegen/builtins/color/func_name_of.rs` ("Taking the minimum is order-independent ... would flake this member's golden the day that order changed") restated for the developer. "Table order" and "if the table is ever reordered" describe how the walk is implemented and why the author chose it, which is a `.ai`/spec concern (§1: it only matters to someone reading the source). The developer-visible fact is just the guarantee, which the previous sentence already states. Behavior itself verified: probe `p_probe` printed `grey -> gray`, `darkgrey -> darkgray`, `darkslategrey -> darkslategray`, `dimgrey -> dimgray`, `lightgrey -> lightgray`, `lightslategrey -> lightslategray`, `slategrey -> slategray`, `cyan -> aqua`, `magenta -> fuchsia`.
SUGGESTED: "Both spellings still resolve through `color::fromName`; only the reverse direction has to choose, and the choice is fixed: the alphabetically first spelling, always."

Everything else on the page checks out against the binary. Evidence for the no-finding sentences, so the absence is auditable:

- **Both examples** compiled with `mfb build -q` in scratch projects under /tmp/plan-125-scratch/A-recheck-iter2/man-page-color-nameOf. Example 1 printed `rebeccapurple`. Example 2 printed `red`, `unnamed (TRUE)`, `unnamed (TRUE)`, matching the page's claim that an off-by-one colour and a translucent one both raise ErrNotFound.
- **Nine pairs.** An awk pass over the packed values in `helper_name_table.rs` found exactly nine duplicate values: seven gray/grey pairs plus aqua/cyan and fuchsia/magenta. The Rust comment on the body says "six", but that comment is not rendered, so it is not a page finding.
- **Alpha gate.** The table has no `transparent` entry and its smallest packed value is black at 0xFF000000, so every entry has alpha 255 as claimed. Probe `p_probe2` with `rgba(255, 0, 0, 0)` exited 255 with error 7-705-0004 "a colour with alpha 0 has no CSS name", confirming ErrNotFound rather than silent alpha dropping.
- **Exact, not nearest.** `#ff0001` raised ErrNotFound in example 2; the body has no distance computation.
- **Errors table.** Both FAIL sites in the body use 77050004; ErrOutOfMemory is the generic allocation row and the page explains it in one sentence.
- **Parameter description** states opacity and exact match; there is no numeric range to bound, so no inclusive/exclusive question arises.
- **Vocabulary.** No banned words and no borrow/ownership framing on the rendered page.
