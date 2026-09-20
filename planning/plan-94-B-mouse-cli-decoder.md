# plan-94-B: Mouse events — CLI decoder, ring, and pollMouse

Last updated: 2026-09-20
Effort: large (3h–1d)
Depends on: plan-94-A (surface, record layout and storage regions frozen)

Implement mouse input end-to-end in **CLI (console) mode**: `term::enableMouse`
emits/withdraws ANSI mouse tracking; a shared stdin **decoder ("pump")** sits at
the single per-byte read choke point, recognizes SGR mouse sequences, and
enqueues decoded events into a per-thread **timestamped overwrite-on-full ring**;
`term::pollMouse` returns the oldest event ≤100 ms old. This is the core engine
and the highest design uncertainty in the whole feature — it is the cheapest
experiment that either confirms or falsifies the "one pump serves all modes"
premise before any app backend is built (C–E reuse this decoder verbatim, feeding
it injected bytes).

Behavioral outcome: on the three linux + macOS targets, a program that calls
`term::enableMouse(TRUE)` and polls in a loop receives `Down`/`Up`/`Move`/`Drag`/
`ScrollUp`/`ScrollDown` events with correct cell coords and modifiers, driven by
SGR sequences fed to a pty; events older than 100 ms are dropped; a thread must
`thread::openStdIn` to receive them.

References:

- plan-94-A §3–§4 (shared design: pump placement, ring, storage regions, the two
  read-path hazards — **read first**; not repeated here).
- plan-15 stdin broadcast log: `src/codegen/io/stdin/stdin_broadcast.rs`,
  `src/codegen/error/constants/error_constants.rs`.
- The read-path choke point: `src/codegen/builtins/io/gen_read_family.rs:43`
  (`emit_stdin_byte_read`) and its callers `func_read_char.rs`,
  `func_read_byte.rs`, `gen_read_line_family.rs`.
- `src/codegen/builtins/io/func_poll_input.rs` — the readiness call the pump must
  not be allowed to make into a liar (plan-94-A §4.6).
- `src/codegen/term/core/term.rs` `emit_on`/`emit_off` (where mouse-mode ANSI
  enable/disable belongs), and the `didResize` per-arena flag as the precedent
  for a term-state read-and-clear.
- Monotonic-clock candidates: `src/codegen/builtins/perf/perf.rs:202`
  (`emit_read_monotonic_nanos`),
  `src/codegen/builtins/datetime/func_monotonic_nanos.rs::lower_monotonic_nanos`,
  `src/codegen/builtins/net/gen_ping.rs:1220` (`emit_monotonic_nanos`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-94-A complete | `mfb build` of a `pollMouse` program runs and prints `None`; the `MOUSE_STATE_*` region and `_mfb_rt_mouse_mode` exist | NOT MET |
| A monotonic-nanos primitive is reusable outside its home package | read the three call sites above; confirm no per-package coupling | UNVERIFIED (first task of Phase 1) |

> If plan-94-A is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `term::enableMouse(TRUE)` writes `\x1b[?1000h\x1b[?1002h\x1b[?1006h` to stdout
  once and sets `_mfb_rt_mouse_mode = 1` (cells);
  `enableMouse(FALSE)` and `term::off` write the disable sequence
  `\x1b[?1000l\x1b[?1002l\x1b[?1006l` and clear the word.
- The stdin read path decodes SGR mouse reports out of the byte stream into the
  ring; non-mouse bytes still reach `io::readChar`/`readByte`/`readLine`
  unchanged.
- `term::pollMouse` returns the oldest ring event with age ≤100 ms (monotonic),
  else the `None` record; repeated calls drain the frame.
- A thread without `thread::openStdIn` gets no events (existing trap).
- `io::pollInput` does not become a liar while mouse mode is on (§3d).
- `io::input`/`io::readLine` do not echo mouse reports onto the screen (§3e).

### Non-goals (explicit constraints)

- **No app-backend changes** (C–E). B is CLI/console only. It does, however, place
  the pump *above* the app/console branch so C–E need no second decoder.
- **`io::readChar` semantics for non-mouse bytes are unchanged.** A byte that is
  not part of a recognized mouse sequence is returned to the program exactly as
  today (including a bare ESC and unrecognized `\x1b[` sequences — those pass
  through; the decoder only consumes complete, recognized SGR mouse reports).
- **No decoded-key handling** (`planning/term.md` item 7) — the pump is structured
  to add it later, but B decodes mouse only; every non-mouse escape passes
  through.
- **No `canvas::` behavior.** `canvas::pollMouse` stays the A stub until C/D/E
  feed the pump pixel coordinates.
- Mouse mode off ⇒ the read path is byte-for-byte the pre-B behavior (the pump is
  gated on `_mfb_rt_mouse_mode`).

## 2. Current State

**The read path.** Every `io::` read helper gets its bytes from one function,
`emit_stdin_byte_read` (`src/codegen/builtins/io/gen_read_family.rs:43`), which
branches on `app_mode`:

- **console** → `emit_stdin_next_byte` → `_mfb_rt_stdin_next_byte`
  (`src/codegen/io/stdin/stdin_broadcast.rs:132,255`), the cooperative per-thread
  reader: main auto-subscribes at entry, a worker calls `thread::openStdIn`, and
  the fast path reads from that thread's arena-local 4 KiB copy of the global log
  taking no lock.
- **app** → a direct `read(0,…,1)` of the window input pipe with an EINTR guard;
  there is no broadcast log in app mode
  (`gen_read_family.rs:38-41`).

That single function is the pump's home. The pre-migration draft of this plan
recommended wrapping `_mfb_rt_stdin_next_byte` instead; that would have decoded
console only and left all three app backends undecoded, so **that Open Decision is
closed in favour of `emit_stdin_byte_read`.**

**Terminal mode.** `term::on` already puts the tty into raw/cbreak
(`~ICANON`/`~ECHO`/`VMIN=1`/`VTIME=0`, bug-149), recorded in
`TERM_STATE_RAW_ACTIVE_OFFSET`
(`src/codegen/error/constants/error_constants.rs:318`). Mouse reporting needs raw
mode and it is already there — B inherits it rather than building it. The
complication is the other direction: `io::input`/`io::readLine` bracket their read
with a **cooked-mode restore** (`:322`/`:325`), which re-enables echo and line
buffering for the duration.

**`term::on`/`off`** (`src/codegen/term/core/term.rs`, the `emit_on`/`emit_off`
emitters reached from the `match call` at `:306`) already own per-arena term-state
initialization and the alt-screen ANSI writes. The mouse enable/disable ANSI
belongs in the same place.

### Verified properties

- **The fast-path reader is per-thread and lock-free**, so a decoder wrapping it
  needs no lock; the ring is worker-local (plan-94-A §4.3). Verified from the
  fast-path comment and the arena-local buffer slots in
  `src/codegen/io/stdin/stdin_broadcast.rs`.
- **Three independent monotonic-nanos emitters already exist** in three different
  packages (`perf`, `datetime`, `net` — cited above), which is strong evidence the
  primitive travels. Phase 1 picks one; it does not need to build a fourth.
- **`io::pollInput` answers without consuming** and has its own console/app split
  (`func_poll_input.rs`, `if !app_mode` at `:91`) — so it is a second consumer of
  the same readiness question and must be taught about the pump (§3d).

## 3. Design

Layers, added in order of decreasing uncertainty.

**(a) The pump.** A decode stage inside `emit_stdin_byte_read`, between the byte
source (either branch) and the byte it returns. Gated on `_mfb_rt_mouse_mode`:
zero ⇒ pass-through (identity, byte-identical to today). Non-zero ⇒ a small state
machine: on `\x1b`, buffer bytes; if they complete an SGR mouse report
`\x1b[<b;x;yM`/`m`, decode → enqueue → consume (return nothing to the caller,
loop for the next real byte); if the buffered prefix cannot be a mouse report,
flush it back to the caller unchanged, one byte at a time (so unrecognized escapes
and a bare ESC pass through). Partial-sequence state lives in the
`MOUSE_STATE_PARSE_*` slots (plan-94-A §4.4a). **This is where the design risk
is** — it changes who consumes bytes on the read path.

**(b) The ring.** Per plan-94-A §4.2: per-arena arena block pointed at by
`MOUSE_STATE_RING_PTR`, 64 slots of `(kind, button, coordA, coordB, modifier bits,
u64 stamp)`, head/tail in the mouse-state region, overwrite-on-full, a monotonic
read at enqueue and at poll, prefix-skip-older-than-100 ms on poll.

**(c) enable/disable + gating.** `enableMouse` writes the ANSI mode set/reset,
sets `_mfb_rt_mouse_mode` to 1 (cells) or 0, and allocates/frees the ring block;
`term::off` also writes the reset and clears the word (idempotent) so a program
that forgets leaves the terminal clean. `pollMouse` drains the ring.

**(d) `io::pollInput` must not start lying.** With mouse on, bytes can be ready
that will never become a character. Teach `pollInput` to run the same pump: when
the readiness check says "ready", drain through the decoder; if everything drained
was mouse, re-check rather than returning TRUE. The cost is bounded — the drain is
the work `readChar` would have done anyway — and the alternative is a readiness
predicate whose TRUE no longer implies a non-blocking read. Phase 2 owns this and
it is a test, not a footnote.

**(e) The cooked-mode window must not echo mouse reports.** `io::input` and
`io::readLine` restore cooked mode around their read. Emit the 1000/1002/1006
*disable* immediately before the restore and the *enable* immediately after the
re-apply, so no report can arrive while the tty is echoing. Mouse events are lost
for the duration of a line read, which is correct and expected: a program asking
for a typed line is not tracking the pointer.

**SGR decode.** `b` low 2 bits = button (0=Left, 1=Middle, 2=Right, 3=none/move);
bit 5 (32) = motion → `Move` (no button held) or `Drag` (button held); bits 2/3/4
= shift/alt/ctrl; button codes 64/65 = ScrollUp/ScrollDown; trailing `M` =
press/`Down`, `m` = release/`Up`. `x`/`y` are 1-based → subtract 1. In console
mode the unit is cells; the decoder stores the numbers and does not care
(plan-94-A §4.3).

### Byte-identity note

Mouse-mode-**off** codegen for the read path MUST stay byte-identical to pre-B
(the pump is behind the mode word). That is the one place a byte-identity check is
the right gate: a fixture that never enables mouse must not diff. Mouse-mode-**on**
paths are new code; their gate is the rt test. A diff in an off-path fixture = a
bug in the gating; root-cause (objdump one fixture) and fix — not a design stop.

## Phases

### Phase 1 — Ring + monotonic clock + pollMouse drain (no decoder yet)

Deliver the queue and poll semantics with a test hook that enqueues synthetic
events, so the ring/TTL is proven before touching the read path.

- [ ] Pick the monotonic-nanos emitter from the three candidates; confirm it is
      callable outside its home package and note the finding in Corrections.
      Record whether a Windows path exists (irrelevant to B, which is
      linux/macOS, but E needs the answer).
- [ ] Implement the ring over the `MOUSE_STATE_*` slots plan-94-A Phase 2
      reserved: allocate the block on `enableMouse(TRUE)`, free on
      `enableMouse(FALSE)`/`off`.
- [ ] `term::pollMouse` (replacing the A stub): prefix-skip stale + return oldest
      ≤100 ms; a small internal `enqueue` helper used by Phase 2 and by a
      test-only hook.
- [ ] Tests: an rt case proving overwrite-on-full keeps the newest; an event
      polled within 100 ms is returned; one polled after >100 ms is dropped; drain
      returns `None` at the end.

Acceptance: rt test shows FIFO drain, overwrite-keeps-newest, and the 100 ms skip,
using a monotonic clock (sleep between enqueue and poll to cross 100 ms).
Commit: —

### Phase 2 — The pump in the read path

- [ ] `src/codegen/builtins/io/gen_read_family.rs`: insert the gated decode stage
      into `emit_stdin_byte_read`, above the `app_mode` branch so both sources
      feed it. Off ⇒ identity; on ⇒ SGR state machine → enqueue;
      unrecognized/partial ⇒ flush through unchanged. Partial-sequence state in
      the `MOUSE_STATE_PARSE_*` slots.
- [ ] `src/codegen/term/core/term.rs`: `enableMouse`/`off` emit the ANSI
      set/reset and write `_mfb_rt_mouse_mode`.
- [ ] `src/codegen/builtins/io/func_poll_input.rs`: drain-and-recheck through the
      pump (§3d).
- [ ] `io::input`/`io::readLine`: withdraw and restore mouse tracking around the
      cooked-mode window (§3e).
- [ ] Decide and implement the presentation-mode gate for `term::pollMouse`
      (plan-94-A §4.5): it should trap `ErrWrongMode` outside `Console` like its
      siblings.
- [ ] Document broadcast (per-subscriber) semantics in the term-backend spec:
      each subscribed+mouse-enabled thread decodes independently.
- [ ] Tests, `tests/runtime/rt_native_term_runtime.rs`, feeding SGR sequences to a
      pty: a click at a cell returns `Down` then `Up` with correct coords; a drag
      returns `Drag`; wheel returns `ScrollUp`/`ScrollDown`; ctrl-click sets
      `.ctrl`; interleaved keyboard bytes still reach `io::readChar`; an
      unrecognized `\x1b[Z` passes through untouched; **`io::pollInput` returning
      TRUE is always followed by a non-blocking `readChar`**; **a `readLine`
      during mouse mode echoes no escape bytes**.
- [ ] Thread test: a worker without `openStdIn` polling mouse gets only `None`
      (and a raw stdin read still traps `ErrInvalidContext`).

Acceptance: the pty-driven rt test decodes all six event kinds with correct
coords/modifiers; keyboard-passthrough, unrecognized-escape-passthrough,
pollInput-honesty and no-echo all hold; the mouse-off read path is byte-identical.
Commit: —

### Phase 3 — Goldens + docs

- [ ] Regenerate `scripts/artifact-gate.sh <exe> term` ×5 and confirm fixtures
      that never enable mouse do **not** diff. Update the rendered man prose in
      `func_enable_mouse.rs`/`func_poll_mouse.rs` from stub wording to real
      behavior (verify with `mfb man term enableMouse`, `mfb man term pollMouse`);
      add the term-backend spec section for the input decoder, the 100 ms TTL,
      broadcast semantics, and the `pollInput`/`readLine` interactions.

Acceptance: `scripts/artifact-gate.sh <exe> term` passes;
`scripts/man-examples-gate.sh` passes; `scripts/man-census.sh --memory-scope`
reports 0 unclassified hits.
Commit: —

## Validation Plan

- Tests: `tests/runtime/rt_native_term_runtime.rs` (ring TTL, all six kinds,
  passthrough, `pollInput` honesty, no-echo, thread opt-in).
- Coverage check: the pty test actually enables mouse and drains events.
- Runtime proof: pty harness feeding `\x1b[<0;10;5M`/`m` → program prints
  `Down 4,9` / `Up 4,9` (0-based).
- Doc sync: term-backend spec (decoder, TTL, broadcast, the two hazards); rendered
  man pages.
- Acceptance: `cargo test --test rt_native_term_runtime`; `cargo test --bin mfb`;
  `scripts/artifact-gate.sh <exe> term`.

## Open Decisions

- **ESC flush timing** — flush an incomplete `\x1b[<…` prefix on the next
  non-continuing byte (no timer) vs. an ESCDELAY timer. Recommend no timer for
  mouse (SGR reports arrive whole in practice); revisit for item 7 keys. (§3a)
- **`io::pollInput` honesty vs. cost** — drain-and-recheck (recommended) vs.
  documenting that TRUE means "bytes ready", not "character ready". (§3d)

*(The pump-placement decision the pre-migration draft left open is closed by
§2: `emit_stdin_byte_read` is the single choke point over both the console and
app byte sources.)*

## Corrections

<Filled in during execution — esp. the monotonic-nanos choice and anything the
read path turns out to do that §2 does not describe.>

## Summary

B is the engine and the risk center: it changes stdin consumption at the one
function every read helper goes through. Placing the pump there rather than at the
console-only reader is what lets C–E inject bytes instead of building three more
event queues. Two read-path neighbours that did not exist when this plan was first
written — `io::pollInput` and the `readLine` cooked-mode restore — are now
first-class Phase 2 work rather than discoveries. It is fully runtime-testable
under a pty, so the risk is bounded by tests before any app backend depends on it.
