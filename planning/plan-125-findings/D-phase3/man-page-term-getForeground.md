### 1. Channel access still requires `color`
UNIT:      man-page:term/getForeground
CLAIM:     "Reading the channels off the result without naming the type needs no extra import."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/D-phase3/man-page-term-getForeground/import-channel.mfb`, compiled with the specified release binary, fails: `TYPE_UNKNOWN_VALUE`: “Field `red` belongs to `color::Color`, whose fields are not visible in this file… add `IMPORT color`.”
SUGGESTED: `Reading the result's channels also requires IMPORT color.`

### 2. A raisable error is absent from the Errors table
UNIT:      man-page:term/getForeground
CLAIM:     "Takes no arguments and returns color::Color."
VERDICT:   incomplete
EVIDENCE:  The rendered page has no Errors table. `src/codegen/builtins/term/func_get_foreground.rs:register` declares `errors: vec![]`, but `src/codegen/term/core/term.rs:emit_get_color` calls `raise_error_into(symbol, "ErrOutOfMemory", ...)` when creating the returned `color::Color` fails.
SUGGESTED: `Register ErrOutOfMemory in the function's Errors table: raised when the result color cannot be created.`

### 3. Storage-format implementation detail in developer documentation
UNIT:      man-page:term/getForeground
CLAIM:     "The value is the module's current foreground attribute, unpacked from the 24-bit value that term::setForeground stored."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/term/core/term.rs:emit_set_color` packs the channels into private terminal state, and `emit_get_color` unpacks them. This storage representation does not affect what an MFBASIC developer writes; the observable behavior was verified by `probe.mfb`, which printed `inactive: 255,255,255,255`, `on: 255,255,255,255`, `set: 0,128,255,255`, and `off: 255,255,255,255`.
SUGGESTED: `It returns the current foreground colour. Immediately after term::on it is white (255, 255, 255); after term::setForeground it has that colour until the next term::setForeground or term::on.`