### 1. `TermSize` cannot be constructed or updated
UNIT:      man-page:term/types
CLAIM:     "The size of the drawing surface in whole character cells, as returned by term::terminalSize. Ask again after a resize rather than caching the first answer."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/term/mod.rs:is_read_only_record` designates `term::TermSize` read-only. Scratch probes compiled with the specified release binary: `term::TermSize[columns := 80, rows := 24]` raises `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`; `WITH size { columns := 80 }` raises `TYPE_READ_ONLY_RECORD_UPDATE`.
SUGGESTED: The size of the drawing surface in whole character cells, returned by `term::terminalSize`. You cannot construct or update a `TermSize`; ask `term::terminalSize` again after a resize.

### 2. `columns` describes positions ambiguously and omits inclusivity
UNIT:      man-page:term/types
CLAIM:     "The width in character cells, never pixels. Valid columns are 0 through columns-1."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/term/func_terminal_size.rs:DESC` identifies `columns` as the width, while `src/codegen/term/core/term.rs:emit_terminal_size` stores the terminal’s reported column count. Thus a width can be 80; `0` through `columns - 1` describes valid column positions, not valid values for this field. The rendered wording also does not explicitly state that the upper bound is inclusive.
SUGGESTED: The surface width in character cells, never pixels. Valid column positions are 0 through `columns - 1`, inclusive.

### 3. `rows` describes positions ambiguously and omits inclusivity
UNIT:      man-page:term/types
CLAIM:     "The height in character cells, never pixels. Valid rows are 0 through rows-1."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/term/func_terminal_size.rs:DESC` identifies `rows` as the height, and `src/codegen/term/core/term.rs:emit_terminal_size` stores the terminal’s reported row count. The stated range is for row positions, not for the `rows` field’s value, and its inclusive upper bound is unstated.
SUGGESTED: The surface height in character cells, never pixels. Valid row positions are 0 through `rows - 1`, inclusive.