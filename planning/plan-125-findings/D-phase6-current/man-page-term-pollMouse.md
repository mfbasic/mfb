### 1. "freshest-first-wins" reads as newest-delivered-first, but delivery is oldest-first
UNIT:      man-page:term/pollMouse
CLAIM:     "Events are delivered **freshest-first-wins**: an event the program does not collect within a tenth of a second is dropped rather than queued up, and a burst longer than the queue keeps its newest events rather than its oldest."
VERDICT:   misleading
EVIDENCE:  `src/codegen/io/mouse/ring.rs:emit_dequeue` reads the slot at `tail` and advances it, i.e. oldest surviving event first. Probe `drain` with `MFB_MOUSE_INJECT=$'\e[<0;5;3M\e[<0;5;3m…'` printed `Down btn=Left r=2 c=4` then `Up btn=Left r=2 c=4` then `Down btn=Right …` — wire order, not newest first. The word "delivered" attaches the rule to the order of return; the implementation's rule is about *retention*, and the order of return is strictly oldest-first.
SUGGESTED: "Events come back in the order they happened, oldest first, but the queue keeps only what is recent: an event the program has not collected within a tenth of a second is dropped rather than held, and a burst longer than the queue keeps its newest events rather than its oldest."

### 2. The queue's size is never stated
UNIT:      man-page:term/pollMouse
CLAIM:     "a burst longer than the queue keeps its newest events rather than its oldest"
VERDICT:   incomplete
EVIDENCE:  `src/codegen/error/constants/error_constants.rs:MOUSE_RING_CAPACITY = 64`. Probe `drain` with 70 injected presses at columns 1..70 printed first event `Down btn=Left r=0 c=6` (the 7th) and `count=64`. A developer sizing a drain loop or reasoning about a stall has no way to know "longer than the queue" means more than 64 from this page.
SUGGESTED: "a burst of more than 64 events keeps the newest 64 rather than the oldest"

### 3. `button` is NOT `None` for drag motion
UNIT:      man-page:term/pollMouse
CLAIM:     "`button` says which button the event is about, and is `term::MouseButton.None` for motion and wheel events, which belong to no button."
VERDICT:   misleading
EVIDENCE:  `src/codegen/io/mouse/decode.rs` (the `motion` label): motion with a button held sets `MOUSE_KIND_DRAG` and leaves `button` as the held button. Probe `drain` injecting `\e[<32;7;9M` printed `Drag btn=Left r=8 c=6`. `MouseKind.Drag` is motion, so "motion" here is wrong for half the motion kinds; the sibling `mfb man term types` says "Drag — … button says which one is down", so the two pages disagree.
SUGGESTED: "`button` says which button the event is about. It is `term::MouseButton.None` for `Move` and the two wheel kinds, which belong to no button; a `Drag` reports the button being held."

### 4. A subscribed worker thread still sees nothing unless it has called `term::enableMouse(TRUE)` itself
UNIT:      man-page:term/pollMouse
CLAIM:     "Each thread that reads standard input decodes what it reads, so a thread that has not called `thread::openStdIn` receives none; and two threads that have both subscribed each get their own copy of the same events rather than taking them from one another."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/io/mouse/ring.rs` header: the ring is allocated per-thread arena "on `enableMouse(TRUE)`", and `emit_enqueue` is a no-op when that thread's ring pointer is 0. Probe `thrA` (main calls `enableMouse(TRUE)`, starts a worker, `thread::openStdIn(t)`, worker only polls) with `printf '\e[<0;5;3M' |` printed `main: Down r=2 c=4` / `worker: None r=0 c=0`. Probe `thrB`, identical except the worker also calls `term::enableMouse(TRUE)`, printed `main: Down r=2 c=4` / `worker: Down r=2 c=4`. The sentence says subscribing is the condition; it is necessary but not sufficient.
SUGGESTED: "Each thread that reads standard input decodes what it reads, so a thread that has not called `thread::openStdIn` receives none. The event queue is also per-thread and is created by `term::enableMouse(TRUE)`, so a worker that polls must call `term::enableMouse(TRUE)` itself; the parent's call does not give it one. Two threads that have both done so each get their own copy of the same events rather than taking them from one another."

### 5. ErrOutOfMemory has no stated condition
UNIT:      man-page:term/pollMouse
CLAIM:     Errors table row `77010001 ErrOutOfMemory Allocation failed.` with no description text naming when it is raised.
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term/mouse.rs:emit_poll_mouse` (labels `_alloc_ok`/`_alloc_error`) builds the returned `MouseEvent` record and branches to the error on failure; the description never mentions this error. `.ai/man-content.md` §2 requires every raisable error with its condition.
SUGGESTED: Add to the description: "The only error it can raise on its own is `ErrOutOfMemory`, when there is no memory left to build the returned record."

### 6. App-window claims could not be executed here
UNIT:      man-page:term/pollMouse
CLAIM:     "In a `mfb build --app` window the same events come from the window rather than from a terminal, and report the same cell coordinates."
VERDICT:   UNVERIFIED
EVIDENCE:  No window can be opened in this sandbox. Read only: `src/codegen/builtins/term/gen_shared.rs` prepends `prepend_wrong_mode_gate(…, ModeRequirement::Console)` for `term.pollMouse` in app builds (supports the ErrWrongMode sentence), and `src/target/tests/app_mouse.rs` asserts `_mfb_rt_term_term_pollMouse` is linked into an app build. The "same cell coordinates" claim depends on a backend's window mouse handler, which I did not run.
SUGGESTED: No wording change proposed; the main thread should run the app-mode sibling check if it has a display.

Verified and found correct (for the audit trail): never waits and returns `None` when idle (`noenable` probe with a piped report and no `enableMouse`: `kind=None`; `ex2` on `/dev/null`: `nothing pending`); expiry after a tenth of a second (`ttl` probe, `os::sleep(200)` after injection: `kind=None`; `MOUSE_EVENT_TTL_NANOS = 100_000_000`); one event per call (drain counts matched injected counts up to 64); zero-based row/column with the wire's 1-based `x;y` swapped to row/column (`\e[<0;5;3M` → `r=2 c=4`); shift/ctrl/alt bits (`\e[<6;…` → `s=TRUE`, `\e[<25;…` → `ct=TRUE a=TRUE`); keystrokes read past a report are kept in order for the next read (`keys` probe: `printf 'ab\e[<0;5;3Mcd\n'` → `poll1 kind=Down`, `line=[abcd]`); the 4096-byte buffer stalls decoding until a read drains it (`keys2` probe with 5000 `a` bytes then a report: `poll1 kind=None`, `line length=5000`, `poll2 kind=Down r=2 c=4`; `INPUT_BUFFER_CAPACITY = 4096` and the `branch_ge(&done)` in `emit_decode_pending_input`). Both examples compile with the release `mfb` and run; `ex2` with an injected click printed `clicked at row 2 column 4`. No banned word from `scripts/man-census.sh --banned-list` appears on the rendered page, and `copy` is used only in its permitted sense.
