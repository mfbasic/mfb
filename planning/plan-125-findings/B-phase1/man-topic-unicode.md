### 1. Terminal display-width behavior is absent
UNIT:      man-topic:unicode  
PAGE:      package-wide  
CATEGORY:  coverage  
CLAIM:     The topic explains byte, scalar, and grapheme measures, but never mentions terminal-column width—the fourth relevant Unicode measure.  
VERDICT:   missing  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man unicode --all` renders only `byteLen`, scalar indexes/`len`, and `graphemes`. `src/codegen/builtins/strings/func_display_width.rs:DESC` defines `strings::displayWidth` as terminal columns and distinguishes it from all three; the probe built and run with the release binary printed `3` for `len("A😀é")`, `7` for its byte length, and `6` for `strings::displayWidth("日本語")`.  
SUGGESTED: Add: “For terminal alignment, use `strings::displayWidth`: terminal columns are distinct from bytes, scalars, and grapheme clusters. Use `strings::padLeftToWidth` or `strings::padRightToWidth` when columns must line up.”

### 2. The overview does not lead readers to the Unicode syntax page
UNIT:      man-topic:unicode  
PAGE:      package-wide  
CATEGORY:  discoverability  
CLAIM:     The See also list has no route to the `String`/`Scalar` literal and `\u{HEX}` escape documentation.  
VERDICT:   missing  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man unicode --all` lists only `strings`, `general len`, `general find`, and `general mid`. `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man types string` renders the missing developer-facing literal syntax, including `\u{HEX}` and backtick-delimited `Scalar` literals.  
SUGGESTED: Add `mfb man types string` to See also, and add direct links for the functions named in the overview (`strings byteLen`, `strings graphemes`, `strings normalizeNfc`, and `strings caseFold`).