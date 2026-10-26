Everything on the page checked out against the binary. One finding.

### 1. Counterpart `strings::right` is never named
UNIT:      man-page:strings/left
CLAIM:     "This clamping is the difference from `strings::mid`, which raises `ErrIndexOutOfRange` when the requested window runs past the end."
VERDICT:   incomplete
EVIDENCE:  Rendered page (`mfb man strings left`): the description and the derived See-also list `strings::mid`, `strings::graphemes` and `astrings::AttributedString`, but never `strings::right`, the mirror function sharing this exact lowering (`src/codegen/builtins/strings/gen_left_right.rs:lower_strings_left_right`, `right: bool`) and the same clamp/`ErrInvalidArgument` contract. `.ai/man-content.md` §2 requires "cross-references to the sibling a developer would otherwise reach for", and its own worked example is the `left`/`right`/`mid` trio. Because See-also is derived from the prose, the omission also drops `right` from the See-also table.
SUGGESTED: "This clamping is shared with `strings::right`, which takes the trailing scalars the same way, and is the difference from `strings::mid`, which raises `ErrIndexOutOfRange` when the requested window runs past the end."

Audit trail for every other sentence, all run from /tmp/plan-125-scratch/D-phase4/man-page-strings-left with the P-125 release `mfb` (`mfb init p_X; mfb build p_X; ./p_X/build/p_X.out`):

- Example 1 printed `hel`, `hi`, `[]`; example 2 printed `a😀`. Both match the page.
- Negative count: `strings::left("abc", -1)` under inline TRAP printed `caught 77050002 Argument value is not valid for the requested operation.` Matches the Errors table and the lowering's `branch_lt(invalid)` → `raise_error("strings.left","ErrInvalidArgument")`.
- Empty value: `left("", 3)` and `left("", 0)` both printed `[]`, no error.
- Clamp: `left("héllo", 5)` and `left("héllo", 6)` both printed `héllo`; `left("héllo", 2)` printed `hé` (scalar, not byte).
- Cluster cut: `left("e" + U+0301 + "x", 1)` printed `e|1|1` (len 1, byteLen 1), confirming a combining mark is a separate scalar and the cluster is split as the page says.
- No mutation: `s` printed `abc` afterwards.
- `mid("hello", 3, 10)` printed `mid caught 77050001 ...` confirming the `ErrIndexOutOfRange` contrast.
- AttributedString overload: `left(bold("hello")+" world", 3)` → `**hel**`; count 50 → `**hello** world`; count 0 → empty; count -2 → `77050002`; original unchanged afterwards.
- Vocabulary: only "new", "mutated", "copy"-free wording; no banned memory terms and no compiler vocabulary on the rendered page.
