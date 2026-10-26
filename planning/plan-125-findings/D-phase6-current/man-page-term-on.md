I have everything I need; writing up the findings now.

### 1. "four things" introduces a five-step list
UNIT:      man-page:term/on
CLAIM:     "The call does four things, in this order."
VERDICT:   wrong
EVIDENCE:  The rendered page lists steps 1 through 5 under that sentence (`mfb man term on`). The implementation does five things in that order: grid allocation, state reset, the alternate-screen escape write, raw mode, then the kitty probe (`screen_mode.rs:emit_on`, read in sequence: `term_grid::emit_grid_alloc`, the `writes` table, `emit_write_const(ESC_ON_SYMBOL)`, `emit_configure_stdin_terminal`, `emit_kitty_enable`).
SUGGESTED: "The call does five things, in this order."

### 2. `term::enableMouse` is not gated and the page says every call but two is
UNIT:      man-page:term/on
CLAIM:     "Every other `term::` call except `term::isOn` and `term::didResize` short-circuits while TUI mode is off — to a no-op, or to an inert default for the getters, or to `ErrUnsupported` for `term::terminalSize` — so nothing a program draws takes effect until `term::on` has returned."
VERDICT:   wrong
EVIDENCE:  `src/codegen/term/core/term/mouse.rs:emit_enable_mouse` doc comment: "Not gated on `active`: asking for mouse reporting before `term::on` is a sequencing mistake … The mode word and the ring are independent of TUI mode, so the request simply takes effect." `grep -c emit_gate_inactive mouse.rs` is 0 while draw.rs/attributes.rs/text.rs each gate. So `term::enableMouse` before `term::on` sends the mouse-tracking escapes to the terminal immediately; it is a third ungated call. (Live mouse reporting itself needs a tty, which the sandbox lacks, so the terminal-side effect is read from source, not observed.)
SUGGESTED: "Every other `term::` call except `term::isOn`, `term::didResize` and `term::enableMouse` short-circuits while TUI mode is off — … Mouse tracking is independent of TUI mode: `term::enableMouse` takes effect whenever it is called, and `term::off` withdraws it."

### 3. Ctrl+C under full key reporting only interrupts while the program is reading
UNIT:      man-page:term/on
CLAIM:     "Ctrl+C still interrupts the program either way."
VERDICT:   misleading
EVIDENCE:  The flags pushed are 31, which include "all keys as escape codes" (`term/core/term/mod.rs:ESC_KITTY_PUSH` = `ESC [ > 31 u`), so the terminal no longer sends the 0x03 byte the line discipline turns into SIGINT. The interrupt is re-created by the key reader: `src/codegen/io/input/reader.rs:emit_interrupt_key` doc comment — "a Ctrl+C press or repeat with no other modifier, while the protocol is on, raises SIGINT and is not returned", and it is "**POSIX only**". On a kitty-protocol terminal, a Ctrl+C typed while the program is busy between reads is not delivered until the next `io::` read decodes it. Without the protocol, raw mode leaves ISIG on (`screen_mode.rs` comment lists only `~ICANON`/`~ECHO`), so Ctrl+C is immediate. Not observable in the sandbox (no tty); read from source.
SUGGESTED: "Ctrl+C still stops the program either way. On a terminal using full key reporting it arrives as a key, so it takes effect the next time the program reads with an `io::` reader rather than at once."

### 4. The key-reporting handshake is also triggered by the line readers and the mouse poller
UNIT:      man-page:term/on
CLAIM:     "Only a program that reads keys with the `io::` readers asks, and the question takes a moment only on a terminal that is slow to answer it."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/io/input/mod.rs:INPUT_BUFFER_READERS` lists `io::readRaw`, `readScalar`, `readGrapheme`, `readKey`, **`readLine` and `input`**, and the doc comment on `program_uses_input_buffer` adds "or polls the mouse". `term::on` emits the probe when `input_state_offset` is `Some` (`term/core/term/mod.rs` `"term.on"` arm), which is set from that predicate (`module_layout.rs:495`). A program whose only reader is `io::input` still asks and still waits up to the probe window — `KITTY_PROBE_MS = 500` ms on a terminal that answers neither query. "Reads keys" undersells both the trigger and the worst-case wait.
SUGGESTED: "Only a program that reads from the console with an `io::` reader — a key reader, `io::input` or `io::readLine` — or polls the mouse asks. A local terminal answers at once; one that is slow or never answers delays `term::on` by up to half a second, after which the terminal counts as not supporting it."

### 5. `term::on` clears a pending resize notice and the page does not say so
UNIT:      man-page:term/on
CLAIM:     "Resets `term::` state to defaults: foreground white (255, 255, 255), background black (0, 0, 0), bold off, underline off, cursor visible, and the cursor at the home position (row 0, column 0)."
VERDICT:   incomplete
EVIDENCE:  The reset table in `screen_mode.rs:emit_on` also writes `TERM_STATE_DID_RESIZE_OFFSET` to 0. A program that calls `term::on` again (the re-entry case the page documents) discards a resize that `term::didResize` had not yet reported. Probe after the first `term::on` printed `didResize=FALSE` (probe at `/tmp/plan-125-scratch/D-phase6-current/man-page-term-on/probe/src/main.mfb`, stderr line "fg=255,255,255 bg=0,0,0 bold=FALSE ul=FALSE didResize=FALSE"); the re-entry clearing is read from the source table, since the sandbox cannot deliver a resize.
SUGGESTED: Append: "… and any resize `term::didResize` has not yet reported is forgotten, since the new surface is already sized to the terminal."

Everything else on the page checked out. The two examples compile and run under `mfb build` in the scratch directory; the first emits the alternate-screen enter, the red `hello in red`, a full 24-row repaint (24 distinct `ESC[n;1H` rows with output redirected, the documented fallback) and the restore sequence `ESC[?1049l` at exit. The second shows exit presenting the frame and leaving the alternate screen without an explicit `term::off`. The probe confirmed: getters are inert and setters are no-ops before `term::on`; `term::terminalSize` raises the ErrUnsupported message with output redirected both before and after `term::on`; a second `term::on` resets bold and colour to defaults and the earlier frame text never appears in the output; raw-mode save is kept from the first entry (`screen_mode.rs` bug-728 skip); `term::isOn` reads TRUE after on and FALSE after off; and the two `?1049h` writes versus one `?1049l` match "runs the whole sequence again".
