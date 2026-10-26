All claims are now checked. Findings follow.

### 1. `TermSize` is described as the surface size, but the function page says it is the terminal's live size
UNIT:      man-page:term/types
CLAIM:     "The size of the drawing surface in whole character cells, returned by term::terminalSize."
VERDICT:   misleading
EVIDENCE:  `mfb man term terminalSize` (func_terminal_size.rs:DESC) says the opposite for the window between a resize and the next `term::sync`: "the size is read live from the terminal … immediately after a resize and before the next `term::sync`, this call can report the new size while the surface is still the old one." So the record holds the terminal's size at the moment of the call, and the drawing surface only takes that size at the next `term::sync`. The types page and the function page disagree about what the two numbers measure during exactly the situation the second sentence ("after a resize") tells the reader to care about.
SUGGESTED: "The size of the terminal in whole character cells at the moment `term::terminalSize` was called. The drawing surface takes that size at the next `term::sync`, so right after a resize the two can briefly differ. A program cannot build one or `WITH`-update it; ask `term::terminalSize` again after a resize rather than caching the first answer."

### 2. `MouseEvent` does not say it is a value a program can build and `WITH`-update
UNIT:      man-page:term/types
CLAIM:     "One thing the mouse did, as reported by term::pollMouse."
VERDICT:   incomplete
EVIDENCE:  Probe `build_mouseevent.mfb` under /tmp/plan-125-scratch/D-phase6-current/man-page-term-types: `LET e AS term::MouseEvent = term::MouseEvent[kind := term::MouseKind.Drag, button := term::MouseButton.Left, row := 2, column := 3, shift := FALSE, ctrl := TRUE, alt := FALSE]` then `LET f = WITH e { row := 9 }` compiles and prints `9 TRUE`. The sibling `TermSize` entry on the same page states explicitly that it cannot be built or `WITH`-updated; `MouseEvent` is silent, so a reader has no way to know the two records differ. `.ai/man-content.md` §2 requires mutation-vs-new-value to be stated. (Source confirms the intent: term/mod.rs `MOUSE_EVENT_TYPE` doc says it is "an ordinary value record", unlike `TERM_SIZE_TYPE`.)
SUGGESTED: Append: "It is an ordinary record: a program may build one with `term::MouseEvent[…]` and produce a changed copy with `WITH`, for example to synthesise or replay an event."

### 3. `MouseKind.None` omits the most common reason a program sees it: mouse reporting was never enabled
UNIT:      man-page:term/types
CLAIM:     "None — Nothing is pending. What an idle term::pollMouse reports, and what a terminal that does not report mouse activity always reports."
VERDICT:   incomplete
EVIDENCE:  `mfb man term pollMouse` (func_poll_mouse.rs:DESC): "Events only arrive after `term::enableMouse(TRUE)`. Before that — and on a terminal that does not report mouse activity at all — every call reports `MouseKind.None`." Probe `enums.mfb`: calling `term::pollMouse()` with no `term::on()` and no `term::enableMouse` prints `TRUE TRUE` (kind = None, button = None), no error. The types page names the terminal case but not the `enableMouse` case, which is the one a new user actually hits, and nothing else on the page mentions `term::enableMouse` at all.
SUGGESTED: "None — Nothing is pending. What `term::pollMouse` reports when idle, before `term::enableMouse(TRUE)` has been called, and on a terminal that does not report mouse activity at all."

### 4. `alt` cannot be verified without a real terminal and keyboard
UNIT:      man-page:term/types
CLAIM:     "TRUE when Alt (Option) was held as the event happened."
VERDICT:   UNVERIFIED
EVIDENCE:  src/codegen/io/mouse/decode.rs maps SGR bit 8 (`SGR_ALT_BIT`) to `alt`, which is the xterm "meta" modifier bit. Whether a macOS terminal sets that bit when Option is held depends on the terminal's own Option-as-Meta setting, which the sandbox cannot exercise (no interactive tty; the `script` pty has no keyboard). `shift` (bit 4) and `ctrl` (bit 16) follow the same path. NEEDS-NETWORK-PROBE is not the right tag; this needs a human at a terminal, or an injected SGR sequence `ESC [ < 8 ; 1 ; 1 M` fed to the program's stdin, which I did not attempt because `term::on` requires a tty.
SUGGESTED: If a live check shows Terminal.app or iTerm2 does not set the bit by default, soften to "TRUE when the terminal reports Alt (Option) held as the event happened; some terminals only report it when Option is configured to act as Meta."

Everything else on the page checked true against the binary:

- All seven `LineStyle` glyph pairs and all six `FillStyle` glyphs: drawing probe `p_draw/src/main.mfb` run under `script` printed rows `─━┄┅┈┉═`, `│┃┆┇┊┋║` and `█░▒▓▚▞`, matching `TERM_HLINE_CODEPOINTS`, `TERM_VLINE_CODEPOINTS` and `TERM_FILL_CODEPOINTS` in error_constants.rs and the page text exactly.
- `MouseKind` and `MouseButton` rules (wheel events carry no button, `Move` means no button held, `Drag` means a button held, `Down`/`Up` name the button): decode.rs tests the wheel codes before the button bits, then bit 5 separates `Move` from `Drag` by whether the low bits named a button, and the terminator decides `Down` vs `Up`. Row and column are zero-based: decode.rs subtracts 1 from both wire coordinates.
- `TermSize` cannot be built or `WITH`-updated: probes `build_termsize.mfb` and `with_termsize.mfb` both fail to compile. Aside for the main thread, not a page defect: both fail with the generic `TYPE_UNKNOWN_VALUE` ("Initializer for binding `s` does not have a known type") rather than a message saying the record is read-only, even though term/mod.rs says "the read-only check repoints here". That looks like a diagnostic gap worth a bug, since the user gets no hint why.
- No banned memory vocabulary and no compiler-internal sentences on the rendered page; "cache" is the only memory-adjacent word and it is used in its everyday sense.
