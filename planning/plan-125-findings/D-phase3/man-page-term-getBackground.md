### 1. Channel access still requires `color`
UNIT:      man-page:term/getBackground
CLAIM:     "Reading the channels off the result without naming the type needs no extra import."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/D-phase3/man-page-term-getBackground/inferred-channel/src/main.mfb` imports only `term` and `io`, assigns `LET current = term::getBackground()`, then reads `current.alpha`. Building with the specified release binary fails with `TYPE_UNKNOWN_VALUE`: “Field `alpha` belongs to `color::Color`, whose fields are not visible in this file… add `IMPORT color`.”
SUGGESTED: `Reading any channel from the result also requires IMPORT color, because the color::Color fields are defined by that package.`

### 2. It does not return exactly an alpha-bearing input colour
UNIT:      man-page:term/getBackground
CLAIM:     "after a `term::setBackground` call it is exactly the colour that was set"
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/D-phase3/man-page-term-getBackground/probe/src/main.mfb` sets `color::rgba(1, 2, 3, 4)` and prints the returned channels after `term::off`; it prints `set:1,2,3,255`. `src/codegen/term/core/term.rs:emit_get_color` explicitly stores alpha `255`; `func_set_background.rs` states alpha is ignored.
SUGGESTED: `After term::setBackground, the returned red, green, and blue channels match the colour that was set; alpha is always 255.`

### 3. The page omits a raisable error
UNIT:      man-page:term/getBackground
CLAIM:     "`term::getBackground` returns the colour that subsequently drawn text will be written in, as a `color::Color`. It takes no arguments."
VERDICT:   incomplete
EVIDENCE:  No Errors section is rendered because `src/codegen/builtins/term/func_get_background.rs:register` declares `errors: vec![]`. But `src/codegen/term/core/term.rs:emit_get_color` allocates the returned `color::Color`; its `alloc_error` path calls `raise_error_into(..., "ErrOutOfMemory", ...)`.
SUGGESTED: `Add ErrOutOfMemory to the function’s declared errors, with: “The returned color cannot be created.”`

### 4. Packed-state implementation detail is out of scope
UNIT:      man-page:term/getBackground
CLAIM:     "The value is the module's current background attribute, unpacked from the 24-bit value that `term::setBackground` stored."
VERDICT:   out-of-scope
EVIDENCE:  The 24-bit packing is an implementation detail of `src/codegen/term/core/term.rs:emit_get_color`; a developer only needs the observable RGB/alpha contract. `.ai/man-content.md` §3 excludes lowering and implementation mechanics from man pages.
SUGGESTED: `The result is the current background colour. term::on resets it to black; term::setBackground changes its red, green, and blue channels, while alpha remains 255.`