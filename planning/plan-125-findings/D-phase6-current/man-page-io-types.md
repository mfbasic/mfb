### 1. `grapheme` is not what `io::readGrapheme` returns for a Ctrl chord or a release
UNIT:      man-page:io/types
CLAIM:     "What the keystroke typed — what io::readGrapheme would return for it — or "" for a key with no Unicode value. For a Ctrl chord it is the key's own character ("a" for Ctrl+A)."
VERDICT:   misleading
EVIDENCE:  Two probes under /tmp/plan-125-scratch/D-phase6-current/man-page-io-types (`probe/`, `probe2/`), fed the same bytes `\x01 \x1b[97;5u \x1b[97;1:3u`. `io::readKey` reports `Unicode ... g=[a] 97 ctrl=TRUE` for both Ctrl+A forms and `Unicode ev=Release g=[a]` for the release. `io::readGrapheme` on the same bytes prints `grapheme bytes: 1`, `grapheme bytes: 1`, then skips the release entirely. Model: `src/codegen/io/input/model.rs:unicode` sets `Text::Scalar(ctrl_code)` for Ctrl and `Text::None` for Release while `grapheme` keeps the character. The first clause is contradicted by the row's own last sentence and by the `Release` row of `KeyEvent`.
SUGGESTED: "The text the key types: "a", "A" for Shift+A, "\n" for Enter, U+007F for Backspace, U+001B for Escape; "" for a key with no Unicode value. It is the key's own character even when io::readGrapheme would see something else: "a" for Ctrl+A (io::readGrapheme returns U+0001), and still the character on a Release (io::readGrapheme skips a release)."

### 2. A pipe can carry Press, Repeat and Release
UNIT:      man-page:io/types
CLAIM:     "The input cannot tell a press from a repeat or a release — a terminal outside the kitty keyboard protocol, a pipe, a file. Every key is reported once, as it is typed, and no release arrives."
VERDICT:   wrong
EVIDENCE:  `printf '\x1b[97;1:1u\x1b[97;1:2u\x1b[97;1:3u\x1b[13;1:3u' | ./build/probe.out` from a pipe prints `Unicode ev=Press`, `Unicode ev=Repeat`, `Unicode ev=Release`, `Enter ev=Release`. The decoder honours a kitty `:event` subfield regardless of source (`src/codegen/io/input/decode/csi.rs:emit_event`); only the *default* for a sequence without one is `NotSupported` (`model.rs:event_default`). So from a pipe or file a release can arrive, and the page's "no release arrives" is false for data that spells one.
SUGGESTED: "The input did not say which it was: a plain keystroke on a terminal outside the kitty keyboard protocol, or bytes from a pipe or a file that do not spell an event. Such a key is reported once, as it is typed. (Bytes that spell a kitty press, repeat or release are reported as that event, whatever the source.)"

### 3. The kitty protocol only reports events while `term::on` is active
UNIT:      man-page:io/types
CLAIM:     "a terminal outside the kitty keyboard protocol, a pipe, a file."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/io/gen_read_key_family.rs:207` sets `events_symbol` only for console builds, and the reader ORs `FLAG_EVENTS` in only when the word at `INPUT_EVENTS_SYMBOL` is non-zero (`src/codegen/io/input/reader.rs:302-333`). That word is stored by `src/codegen/term/core/term/kitty.rs:emit_store_input_events`, which runs from `term::on` (guarded on `TERM_STATE_RAW_ACTIVE_OFFSET`) and is cleared by `emit_kitty_disable` on `term::off`. A kitty terminal with no `term::on` therefore reports `NotSupported` for every key. The sibling `io::readKey` page says "while term::on is active"; this page omits the condition and so names a case that does not exist as written.
SUGGESTED: "...a terminal that is not speaking the kitty keyboard protocol (a terminal that can only speaks it while term::on is active), a pipe, a file."

### 4. `shift` and `meta` are not reported by a plain terminal
UNIT:      man-page:io/types
CLAIM:     "shift — TRUE when Shift was held." and "meta — TRUE when Meta was held — Command on macOS, the Windows or Super key elsewhere."
VERDICT:   incomplete
EVIDENCE:  Probe: `printf 'A\x1b[Z\x1b[1;2A' | ./build/probe.out` prints `Unicode ... g=[A] shift=FALSE`, `Tab ... shift=TRUE`, `Up ... shift=TRUE`. A plain terminal sends a capital letter with no modifier byte (`model.rs:text` builds the token with `mods = 0`), so Shift+A is `"A"` with `shift` FALSE, while Shift with a named key does set it. `meta` is set only from a kitty/app modifier field (`keys.rs:modifier_bits`, bits 8|32); a plain terminal never sends one. The `io::readKey` page carries a table saying exactly this; the types page states both fields unconditionally, so a reader of this page alone would test `k.shift` for Shift+A and never see TRUE.
SUGGESTED: shift: "TRUE when Shift was held and the input reports it. A plain terminal reports Shift only with a named key (Shift+Tab, Shift+Up); Shift+A arrives as "A" with shift FALSE." meta: "TRUE when Meta was held — Command on macOS, the Windows or Super key elsewhere. Reported only by an --app window or a kitty-protocol terminal; a plain terminal never sets it."

### 5. `alt` as "Option on macOS" is unverified for a console
UNIT:      man-page:io/types
CLAIM:     "alt — TRUE when Alt was held — Option on macOS."
VERDICT:   UNVERIFIED
EVIDENCE:  In an `--app` window the mapping is real: `src/codegen/io/input/encode.rs:modifier_rows` maps NSEventModifierFlagOption (`1 << 19`) to the kitty alt bit, and the pipe probe confirms the Esc-prefix form (`\x1ba` → `Unicode g=[a] alt=TRUE`). Whether Option reaches a console program at all depends on the terminal emulator's "Option as Meta/Esc+" setting, which the sandbox cannot exercise (no interactive terminal). Not a defect in the binary; flagged only so the main thread can decide whether the row should say the Esc-prefix dependency.
SUGGESTED: Leave as is, or: "TRUE when Alt was held — Option on macOS. On a terminal, Alt arrives only when the terminal sends it as an Escape prefix."

Verified true and not reported: Backspace grapheme U+007F (0x08 and 0x7F both), Enter "\n" for CR, LF, CRLF and keypad `ESC O M`, Tab "\t", Shift+Tab as Tab with shift set, Escape U+001B, arrows/Home/End/F-keys/Insert/Delete with empty grapheme, Ctrl+Up and Shift+Up modifier bits, `EndKey` spelling, keypad `1` as Unicode "1", kitty Super and Meta both landing on `meta`, F13 silently skipped. No banned memory vocabulary or compiler-internal sentence on the page. The page has no example of its own; the probes above compiled and ran with the release binary.
