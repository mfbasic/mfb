### 1. `off` contradicts the package’s inactive-mode rule
UNIT:      man-pkg:term
PAGE:      term::off
CATEGORY:  consistency
CLAIM:     “After `term::off` returns, `term::isOn` reports `FALSE` and every `term::` call except `term::on` and `term::isOn` is a no-op again.”
VERDICT:   inconsistent
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man term off` prints that sentence, while `mfb man term isOn` says `term::didResize` is the third ungated call. Implementation confirms it: `src/codegen/term/core/term.rs:emit_did_resize` reads and clears the resize flag without checking active state.
SUGGESTED:  “After `term::off` returns, `term::isOn` reports `FALSE`. `term::didResize` remains available to read and clear any pending resize flag; the other calls are inert, return their documented inactive defaults, or—in the case of `term::terminalSize`—raise `ErrUnsupported`.”