### 1. AttributedString attributes are silently discarded

UNIT:      man-page:io/write  
CLAIM:     “io::write writes value to standard output exactly as stored and adds nothing.”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/engine/value/builder_values.rs:3355-3368` rewrites an `AttributedString` argument to `toString(a)` before the writer runs; the probe `attributed_write.mfb` applied `astrings::bold()` to `"X"` and `./attributed_project/build/attributed_project.out | od -An -tx1` printed `58`, with no styling bytes.  
SUGGESTED: `io::write writes a String, or the visible text of an AttributedString, to standard output and adds nothing. Attributes on an AttributedString do not affect this output.`

### 2. The AttributedString parameter omits its visible-text-only behavior

UNIT:      man-page:io/write  
CLAIM:     “The attributed text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/engine/value/builder_values.rs:3355-3368` converts this overload to `toString(a)`, so only its visible text reaches the common writer. The `attributed_write.mfb` probe printed only `X` (`58`) after applying `astrings::bold()`.  
SUGGESTED: `The visible text to write. Its attributes are ignored; the visible UTF-8 text is emitted unchanged and may be empty.`

### 3. Byte-for-byte claim is false in TUI mode

UNIT:      man-page:io/write  
CLAIM:     “The text is treated as UTF-8 and emitted byte for byte, with no escaping and no newline translation.”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/io/gen_write_family.rs:292-303` routes TUI-active output to `term_grid::emit_grid_write`; `src/codegen/term/grid/term_grid.rs:535-540` states that writer interprets `\n` and `\r`, wrapping and scrolling while stamping glyphs into the grid rather than emitting bytes.  
SUGGESTED: `Outside TUI mode, a String's UTF-8 bytes are written unchanged, with no escaping or newline translation. In TUI mode, output is written into the terminal grid, where line breaks, carriage returns, wrapping, and scrolling affect its placement.`

### 4. Raisable error is absent from the Errors table

UNIT:      man-page:io/write  
CLAIM:     “A zero-byte or failing write is a failure and raises ErrWriteFailed.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/io/gen_write_family.rs:286-291` raises `ErrWriteFailed`, but `src/codegen/builtins/io/func_write.rs:91-105` declares `errors: vec![]` for both overloads, so the rendered page has no Errors table.  
SUGGESTED: `Add the derived ErrWriteFailed Errors-table row: “The output write returns zero bytes or fails.”`