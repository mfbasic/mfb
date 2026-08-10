# plan-94-B: Mouse events — CLI decoder, ring, and pollMouse

Last updated: 2026-08-09
Effort: large (3h–1d)
Depends on: plan-94-A (surface + record layout frozen)

Implement mouse input end-to-end in **CLI (console) mode**: `term::enableMouse`
emits/withdraws ANSI mouse tracking; a shared stdin **decoder ("pump")** sits in
the io read path, recognizes SGR mouse sequences, and enqueues decoded events into
a per-thread **timestamped overwrite-on-full ring**; `term::pollMouse` returns the
oldest event ≤100 ms old. This is the core engine and the highest design
uncertainty in the whole feature — it is the cheapest experiment that either
confirms or falsifies the "one pump serves all modes" premise before any app
backend is built (C–E reuse this decoder verbatim, feeding it injected bytes).

Behavioral outcome: on the three linux + macOS targets, a program that calls
`term::enableMouse(TRUE)` and polls in a loop receives `Down`/`Up`/`Move`/`Drag`/
`ScrollUp`/`ScrollDown` events with correct cell coords and modifiers, driven by
SGR sequences fed to a pty; events older than 100 ms are dropped; a thread must
`thread::openStdIn` to receive them.

References:

- plan-94-A §3–§4 (shared design: pump, ring, per-arena/thread placement — read
  first; not repeated here).
- plan-15 stdin broadcast log: `src/target/shared/code/stdin_broadcast.rs`,
  `src/target/shared/code/io_stdin.rs`, `error_constants.rs:600–713`.
- `src/target/shared/code/perf.rs:197` `emit_read_monotonic_nanos` (CLOCK_MONOTONIC).
- `src/target/shared/code/term.rs` `emit_on`/`emit_off` (where mouse-mode ANSI
  enable/disable belongs), and the `didResize` per-arena flag slot as the
  precedent for the mouse-mode flag.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-94-A complete | `git log --oneline \| grep plan-94-A` shows the landed phase; `mfb build` of a `pollMouse` program returns `None` | NOT MET |
| `emit_read_monotonic_nanos` usable outside perf | read `src/target/shared/code/perf.rs:197` + call site — confirm no perf-table coupling | UNVERIFIED (first task of Phase 1) |

> If plan-94-A is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `term::enableMouse(TRUE)` writes `\x1b[?1000h\x1b[?1002h\x1b[?1006h` to stdout
  once (and records mouse-mode active in a per-arena term-state slot);
  `enableMouse(FALSE)` and `term::off` write the disable sequence
  `\x1b[?1000l\x1b[?1002l\x1b[?1006l`.
- The stdin read path decodes SGR mouse reports out of the byte stream into the
  ring; non-mouse bytes still reach `io::readChar`/`readByte`/`readLine` unchanged.
- `term::pollMouse` returns the oldest ring event with age ≤100 ms (monotonic),
  else the `None` record; repeated calls drain the frame.
- A thread without `thread::openStdIn` gets no events (existing trap).

### Non-goals (explicit constraints)

- **No app-backend changes** (C–E). B is CLI/console only.
- **`io::readChar` semantics for non-mouse bytes are unchanged.** A byte that is
  not part of a recognized mouse sequence is returned to the program exactly as
  today (including a bare ESC and unrecognized `\x1b[` sequences — those pass
  through; the decoder only consumes complete, recognized SGR mouse reports).
- **No decoded-key handling** (item 7) — the pump is structured to add it later,
  but B decodes mouse only; every non-mouse escape passes through.
- Mouse mode off ⇒ the read path is byte-for-byte the pre-B behavior (the pump is
  gated on the per-arena mouse flag).

## 2. Current State

The plan-15 read path (per plan-94-A §4.4): main auto-subscribes at entry; a
worker calls `thread::openStdIn`; `_mfb_rt_stdin_next_byte`
(`error_constants.rs:701`) returns the next byte for the calling thread from its
arena-local 4 KiB copy of the global log, taking no lock on the fast path.
`io::readChar`/`readByte`/`readLine` (`src/target/shared/code/io_stdin.rs`) sit on
top of `_mfb_rt_stdin_next_byte` (broadcast log) in console mode.

`term::on`/`off` (`src/target/shared/code/term.rs:emit_on`/`emit_off`) already own
per-arena term-state initialization and ANSI writes (they emit the alt-screen
enter/leave and, since the didResize work, initialize a per-arena flag slot). The
mouse-mode flag and the enable/disable ANSI belong in the same place.

### Verified properties

- **A monotonic-nanos codegen primitive exists** — `emit_read_monotonic_nanos`
  (`perf.rs:197`) emits `clock_gettime(CLOCK_MONOTONIC)` inline (Darwin=6/Linux=1).
  VERIFY (Phase 1 first task): it is callable outside the perf builder and that a
  Windows path exists or is added (perf may special-case Windows; irrelevant to B
  since B is linux/macOS, but note it for D/E — Windows app is C-E scope).
- **The fast-path reader is per-thread and lock-free** — so a decoder wrapping it
  needs no lock; the ring is worker-local (plan-94-A §4.3). Verified from
  `error_constants.rs:701` ("Fast path … takes no lock") and the arena-local buffer
  slots (`:614–623`).

## 3. Design

Three layers, added in order of decreasing uncertainty:

**(a) The pump.** A decode stage between the raw per-thread byte source
(`_mfb_rt_stdin_next_byte`) and the char stream the read helpers return. Gated on
the per-arena mouse-mode flag: off ⇒ pass-through (identity, byte-identical to
today). On ⇒ a small state machine: on `\x1b`, buffer bytes; if they complete an
SGR mouse report `\x1b[<b;x;yM`/`m`, decode → enqueue → consume (return nothing to
the char stream, advance to the next real char); if the buffered prefix cannot be
a mouse report, flush it back to the char stream unchanged (so unrecognized escapes
and bare ESC pass through). Partial-sequence state (a sequence split across
`_mfb_rt_stdin_next_byte` calls) lives in per-arena slots. **This is where the
design risk is** — it changes who consumes bytes on the read path.

**(b) The ring.** Per plan-94-A §4.2: per-arena arena block, 64 slots of
`(kind,button,row,col,shift,ctrl,alt, u64 stamp)`, head/tail in per-arena slots,
overwrite-on-full, `emit_read_monotonic_nanos` at enqueue and poll,
prefix-skip-older-than-100 ms on poll.

**(c) enable/disable + gating.** `enableMouse` writes the ANSI mode set/reset and
flips the per-arena mouse flag; `term::off` also writes the reset (idempotent) so a
program that forgets leaves the terminal clean. `pollMouse` drains the ring.

SGR decode: `b` low 2 bits = button (0=Left,1=Middle,2=Right,3=none/move); bit 5
(32) = motion → `Move` (no button) or `Drag` (button held); bits 2/3/4 = shift/alt/
ctrl; button codes 64/65 = ScrollUp/ScrollDown; trailing `M` = press/`Down`, `m` =
release/`Up`. `x`/`y` are 1-based → subtract 1 for 0-based cell coords.

### Byte-identity note

Mouse-mode-**off** codegen for the read path MUST stay byte-identical to pre-B
(the pump is behind the flag). That is the one place a byte-identity check is the
right gate: `byte-identity/io` and any stdin fixture must not diff when mouse is
never enabled. Mouse-mode-**on** paths are new code; their gate is the rt test.
A diff in an off-path fixture = a bug in the gating; root-cause (objdump one
fixture) and fix — not a design stop.

## Phases

### Phase 1 — Ring + monotonic clock + pollMouse drain (no decoder yet)

Deliver the queue and poll semantics with a test hook that enqueues synthetic
events, so the ring/TTL is proven before touching the read path.

- [ ] Verify `emit_read_monotonic_nanos` is reusable; note findings in Corrections.
- [ ] Add per-arena slots: mouse-mode flag, ring block pointer, head, tail, parse
      state (`error_constants.rs` TERM_STATE_* family, per plan-94-A §4.2/§4.4).
- [ ] `src/target/shared/code/term.rs`: allocate the ring block on `enableMouse(TRUE)`
      (free on `enableMouse(FALSE)`/`off`); implement `pollMouse` = prefix-skip
      stale + return oldest ≤100 ms (replaces the A stub); a small internal
      `enqueue` helper (used by Phase 2 and a test-only hook).
- [ ] Tests: an rt case using a temporary test-only enqueue path (or a decoder fed
      synthetic bytes once Phase 2 lands) proving: overwrite-on-full keeps newest;
      an event polled within 100 ms is returned; one polled after >100 ms is
      dropped; drain returns `None` at the end.

Acceptance: rt test shows FIFO drain, overwrite-keeps-newest, and the 100 ms
skip, using a monotonic clock (sleep between enqueue and poll to cross 100 ms).
Commit: —

### Phase 2 — The pump in the read path (decode SGR mouse; pass-through else)

- [ ] `src/target/shared/code/io_stdin.rs` (+ `stdin_broadcast.rs` if the decode
      must wrap `_mfb_rt_stdin_next_byte`): insert the gated decode stage; off ⇒
      identity; on ⇒ SGR state machine → enqueue; unrecognized/partial ⇒ flush
      through unchanged. Partial-sequence state in the per-arena slots.
- [ ] `enableMouse`/`off`: emit the ANSI set/reset sequences; flip the flag.
- [ ] Document broadcast (per-subscriber) semantics in the term-backend spec
      (plan-94-A §4.4): each subscribed+mouse-enabled thread decodes independently.
- [ ] Tests: `tests/rt_native_term_runtime.rs` — feed SGR sequences to a pty:
      a click at a cell returns `Down` then `Up` with correct coords; a drag
      returns `Drag`; wheel returns `ScrollUp`/`Down`; a modifier (ctrl-click) sets
      `.ctrl`; interleaved keyboard bytes still reach `io::readChar`; an
      unrecognized `\x1b[Z` passes through untouched.
- [ ] Thread test: a worker without `openStdIn` polling mouse gets only `None`
      (and a raw stdin read still traps `ErrInvalidContext`).

Acceptance: the pty-driven rt test decodes all six event kinds with correct
coords/modifiers; keyboard-passthrough and unrecognized-escape-passthrough hold;
mouse-off read path is byte-identical (`byte-identity/io` unchanged).
Commit: —

### Phase 3 — Goldens + docs

- [ ] Regenerate `byte-identity/term` ×5 (mouse-on codegen now present in the
      cover fixture if it exercises enable/poll) and confirm `byte-identity/io`
      unchanged. Update man `enableMouse.md`/`pollMouse.md` from stubs to real
      behavior; term-backend spec section for the input decoder + 100 ms TTL +
      broadcast semantics.

Acceptance: `scripts/artifact-gate.sh <exe> term` and `io` pass; man examples
compile (`scripts/check-man-examples.py`).
Commit: —

## Validation Plan

- Tests: `tests/rt_native_term_runtime.rs` (ring TTL, all six kinds, passthrough,
  thread opt-in); descriptor tests unchanged from A.
- Coverage check: the pty test actually enables mouse and drains events.
- Runtime proof: pty harness feeding `\x1b[<0;10;5M`/`m` → program prints
  `Down 4,9` / `Up 4,9` (0-based).
- Doc sync: term-backend spec (decoder, TTL, broadcast), man pages.
- Acceptance: `cargo test --test rt_native_term_runtime`; `cargo test --bin mfb`;
  `scripts/artifact-gate.sh <exe> term io`.

## Open Decisions

- **Pump placement** — wrap `_mfb_rt_stdin_next_byte` (one choke point, covers all
  read helpers) vs. decode inside each of `readChar`/`readByte`/`readLine`.
  Recommend wrapping the single reader. (§3a)
- **ESC flush timing** — flush an incomplete `\x1b[<…` prefix on the next
  non-continuing byte (no timer) vs. an ESCDELAY timer. Recommend no timer for
  mouse (SGR reports arrive whole in practice); revisit for item 7 keys. (§3a)

## Corrections

<Filled in during execution — esp. the `emit_read_monotonic_nanos` reuse finding
and the pump-placement decision once the read path is read in full.>

## Summary

B is the engine and the risk center: it changes stdin consumption on the per-thread
read path. It is fully runtime-testable under a pty, so the risk is bounded by
tests before any app backend depends on it.
