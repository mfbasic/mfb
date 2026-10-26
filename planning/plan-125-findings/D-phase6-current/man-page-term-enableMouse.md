### 1. "motion" is promised, but a terminal only reports motion while a button is held
UNIT:      man-page:term/enableMouse
CLAIM:     "`term::enableMouse(TRUE)` asks the terminal to report mouse activity — button presses and releases, motion, drags and wheel scrolling"
VERDICT:   misleading
EVIDENCE:  The enable path writes exactly `ESC[?1000h ESC[?1002h ESC[?1006h` (`src/codegen/term/core/term/mod.rs:ESC_MOUSE_ON`, and the comment above it: "1002 also report motion WHILE A BUTTON IS HELD"). Mode 1003 (any-motion tracking) is never requested. Probe `/tmp/plan-125-scratch/D-phase6-current/man-page-term-enableMouse/p_ex3` piped through `od -c` shows the bytes `033 [ ? 1 0 0 0 h 033 [ ? 1 0 0 2 h 033 [ ? 1 0 0 6 h` and nothing else. So a terminal never sends the no-button motion report, and `term::MouseKind.Move` ("The mouse moved with no button held", `mfb man term types`) cannot arrive from a terminal; only `Drag` does. A reader who writes a hover-highlight loop against this sentence will see nothing until they press a button.
SUGGESTED: "`term::enableMouse(TRUE)` asks the terminal to report mouse activity — button presses and releases, drags (motion while a button is held), and wheel scrolling — so that `term::pollMouse` can hand those events back to the program. Motion with no button held is not requested from a terminal, so `term::MouseKind.Move` never arrives there; a program that wants to track the pointer must wait for a press or a drag."

### 2. The reset-at-exit story is narrower than the binary
UNIT:      man-page:term/enableMouse
CLAIM:     "Turn reporting off before the program finishes so the mouse goes back to the user. `term::off` does that for you when it leaves TUI mode, so a program that pairs `term::on` with `term::off` is already tidy."
VERDICT:   misleading
EVIDENCE:  Two things the sentence gets wrong by omission. (a) `term::off` withdraws mouse tracking whether or not TUI mode was on: `src/codegen/term/core/term/screen_mode.rs` emits `emit_mouse_withdraw` after the `inactive` label with the comment "so it runs whether or not TUI mode was on — mouse mode is independent of `term::on`". (b) The runtime calls `term::off` at normal exit, so a program that never calls `enableMouse(FALSE)` or `term::off` still leaves the terminal clean. Probe `p_ex4` (enableMouse(TRUE), print, return from main, no off) ends its output with `033 [ ? 1 0 0 6 l 033 [ ? 1 0 0 2 l 033 [ ? 1 0 0 0 l`. `mfb man term off` already says the runtime calls it on return from main, an unhandled error, Ctrl-C or SIGTERM. The sentence as written tells the reader they must do something the runtime already does, and ties the guarantee to "leaving TUI mode" when it does not depend on it.
SUGGESTED: "Reporting is withdrawn for you when the program ends normally — on return from `main`, an unhandled error, Ctrl-C or SIGTERM — because the runtime calls `term::off` then, and `term::off` turns mouse reporting off whether or not TUI mode was on. Call `term::enableMouse(FALSE)` yourself when you want the mouse back while the program is still running, for example before printing ordinary output. A `kill -9` leaves the terminal reporting; `reset` at the shell recovers it."

### 3. The page never says the call works without `term::on`
UNIT:      man-page:term/enableMouse
CLAIM:     (omission) Both examples wrap the call in `term::on` / `term::off`; no sentence says whether `term::on` is required.
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term/mouse.rs:emit_enable_mouse` doc comment: "Not gated on `active` ... The mode word and the ring are independent of TUI mode, so the request simply takes effect." Probe `p_ex3` (no `term::on`) writes the three mode-set bytes, `term::pollMouse` returns `MouseKind.None`, and the program prints `kind=None` then `done`. Every other setter on this package no-ops while TUI mode is off (`mfb man term isOn` lists them), so a reader has every reason to assume this one does too, and it is the exception.
SUGGESTED: Add after the first paragraph: "Unlike the drawing calls, this one does not need `term::on`: a line-mode program can turn reporting on, poll with `term::pollMouse`, and turn it off again without ever entering TUI mode."

### 4. `enableMouse(FALSE)` discards events not yet polled
UNIT:      man-page:term/enableMouse
CLAIM:     "`term::enableMouse(FALSE)` stops the reporting again."
VERDICT:   incomplete
EVIDENCE:  The disable path in `src/codegen/term/core/term/mouse.rs:emit_enable_mouse` writes the reset sequence, clears the mode word, then calls `mouse_ring::emit_ring_free`, which releases the pending-event queue. Anything reported but not yet taken by `term::pollMouse` is gone; the next `pollMouse` after re-enabling starts from an empty queue (`emit_ring_alloc` in `src/codegen/io/mouse/ring.rs` allocates fresh only when the slot is zero). The page does not say so, and a program that disables, then drains "the last few events", gets `None`.
SUGGESTED: "`term::enableMouse(FALSE)` stops the reporting again and discards any events that were reported but not yet taken by `term::pollMouse`; drain first if you want them."

### 5. "reports no error" sits two paragraphs above "raises ErrWrongMode"
UNIT:      man-page:term/enableMouse
CLAIM:     "The call is **best effort** and reports no error."
VERDICT:   misleading
EVIDENCE:  The Errors table lists `ErrWrongMode`, the last paragraph says the call raises it in an `--app` build outside `app::Mode.Console`, and `src/codegen/builtins/term/gen_shared.rs:158` gates `term.enableMouse` on the presentation mode in an app build. "Reports no error" is true only of terminal support. Read on its own it contradicts the table.
SUGGESTED: "The call is **best effort** about the terminal: it reports no error when the terminal does not support mouse reporting. Not every terminal does, and in that case ..."

Verified and not findings, for the audit trail: the 25 ms figure is `INPUT_DELAY_MS = 25` in `src/codegen/io/input/mod.rs`; the suspend around `io::readLine`/`io::input` is `emit_mouse_tracking_window` called from `src/codegen/builtins/io/gen_read_line_family.rs:263,885`; the `io::readKey(0)` claim is implemented by the pump in `src/codegen/builtins/io/gen_read_family.rs:emit_pump_escape_wait`; both page examples compiled with `mfb build -q` and ran to exit 0, example 2 printing `the mouse is the terminal's again` after the reset bytes; every See-also target renders; no banned memory vocabulary and no compiler-internal sentence on the page.
