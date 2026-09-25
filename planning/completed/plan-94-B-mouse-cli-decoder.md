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
| plan-94-A complete | `mfb build` of a `pollMouse` program runs and prints `None`; the `MOUSE_STATE_*` region and `_mfb_rt_mouse_mode` exist | MET (measured 2026-09-20: every A box ticked, phases at `912013f22`/`0072f0723`; the fixture prints `kind=None …`; `MOUSE_STATE_SLOTS`/`MOUSE_MODE_SYMBOL` exist and `_mfb_rt_mouse_mode` appears in a mouse program's `dataObjects`) |
| A monotonic-nanos primitive is reusable outside its home package | read the three call sites above; confirm no per-package coupling | MET, **but none of the three is reusable as-is** — see Corrections B1. `datetime`'s is the only correct-on-every-target one and it is a whole member body, so Phase 1 factors a shared emitter out of it rather than calling one of the three. |

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

- [x] Pick the monotonic-nanos emitter from the three candidates; confirm it is
      callable outside its home package and note the finding in Corrections.
      Record whether a Windows path exists (irrelevant to B, which is
      linux/macOS, but E needs the answer).
      — **None of the three is usable as-is**; see Corrections B1 for the
      evidence on each. Factored a correct-on-every-family emitter into
      `src/codegen/io/mouse/clock.rs` instead. **Windows answer for plan-94-E:
      yes** — `QueryPerformanceCounter`/`Frequency`, with the quotient/remainder
      split that keeps the nanos inside `u64`.
- [x] Implement the ring over the `MOUSE_STATE_*` slots plan-94-A Phase 2
      reserved: allocate the block on `enableMouse(TRUE)`, free on
      `enableMouse(FALSE)`/`off`. — `src/codegen/io/mouse/ring.rs`. 64 slots ×
      48 bytes, head/tail as absolute counts (so "full" is a subtraction with no
      ambiguous empty/full state), overwrite-on-full, monotonic stamp per slot.
      Both alloc and free are idempotent — `enableMouse(TRUE)` twice must not leak
      a block and `term::off` after `enableMouse(FALSE)` must not double-free.
- [x] `term::pollMouse` (replacing the A stub): prefix-skip stale + return oldest
      ≤100 ms; a small internal `enqueue` helper used by Phase 2 and by a
      test-only hook. — done; the stale prefix is *skipped past* (tail advances)
      rather than re-walked, so a program that stopped polling does not pay for
      the same expired events on every later poll. The "test-only hook" became
      `MFB_MOUSE_INJECT`, for the reason in Corrections B2.
- [x] Tests: an rt case proving overwrite-on-full keeps the newest; an event
      polled within 100 ms is returned; one polled after >100 ms is dropped; drain
      returns `None` at the end.
      — `native_term_mouse_ring_overwrites_oldest_and_expires_stale`.

Acceptance: rt test shows FIFO drain, overwrite-keeps-newest, and the 100 ms skip,
using a monotonic clock (sleep between enqueue and poll to cross 100 ms).
**Met**, measured: a 10-report burst drains as `COUNT:10 FIRST:0 LAST:9` (FIFO,
whole); 64 (exactly capacity) as `COUNT:64 FIRST:0 LAST:63`; 72 as `COUNT:64
FIRST:8 LAST:71` — the **newest** 64, which is what distinguishes overwrite from
"refuse the overflow" (that would have given `FIRST:0 LAST:63` and looked just as
plausible). TTL across `os::sleep`: 0 ms → 2 events, 20 ms → 2, 150 ms → 0,
400 ms → 0.
Commit: 7eed59fb2

### Phase 2 — The pump in the read path

- [x] `src/codegen/builtins/io/gen_read_family.rs`: insert the gated decode stage
      into `emit_stdin_byte_read`, above the `app_mode` branch so both sources
      feed it. Off ⇒ identity; on ⇒ SGR state machine → enqueue;
      unrecognized/partial ⇒ flush through unchanged. Partial-sequence state in
      the `MOUSE_STATE_PARSE_*` slots.
      — done, as a prologue/epilogue pair wrapping the read in a loop (a swallowed
      byte reads another, so the caller still gets exactly one byte or EOF). The
      gate is **compile-time**, not the runtime mode test the phase text implies —
      see Corrections B4, which is what makes the byte-identity claim exact.
      The decoder lives in `src/codegen/io/mouse/decode.rs`; the flush needed a
      drain cursor the plan did not anticipate (Corrections B5).
- [x] `src/codegen/term/core/term.rs`: `enableMouse`/`off` emit the ANSI
      set/reset and write `_mfb_rt_mouse_mode`. — done. `term::off` withdraws
      tracking **after** its `inactive` label, so it runs whether or not TUI mode
      was ever entered: mouse mode is independent of `term::on`, and a program
      that enabled the mouse without `term::on` must still leave the terminal
      clean. The escapes are suppressed in `--app` builds, where stdout is the
      transcript and they would be *displayed* (Corrections B6).
- [x] `src/codegen/builtins/io/func_poll_input.rs`: drain-and-recheck through the
      pump (§3d). — done, closing that Open Decision in favour of honesty. The
      re-check is forced non-blocking, which the plan does not say but has to be
      true: looping with the caller's timeout would let `pollInput(100)` wait
      100 ms *per report* while the user drags.
- [x] `io::input`/`io::readLine`: withdraw and restore mouse tracking around the
      cooked-mode window (§3e). — done; the resume side has to park the `Result`
      bank, which it did not at first and which segfaulted every line read
      (Corrections B3).
- [x] Decide and implement the presentation-mode gate for `term::pollMouse`
      (plan-94-A §4.5): it should trap `ErrWrongMode` outside `Console` like its
      siblings. — **Decided: yes.** Applied in `gen_shared::lower_term_helper` on
      the console fall-through, since no backend's app dispatch claims these two
      members yet. An ungated `term::pollMouse` in `Mode.Canvas` would silently
      answer "where is the mouse, in cells" about a surface with no cells, which
      is worse than the trap its siblings raise. (`canvas::`'s were already
      `Canvas`-gated in A — plan-94-A Corrections C4.)
- [x] Document broadcast (per-subscriber) semantics in the term-backend spec:
      each subscribed+mouse-enabled thread decodes independently.
      — `src/docs/spec/app/04_term-backend.md`, "Broadcast (per-subscriber)
      semantics", including the corollary that a thread which never called
      `thread::openStdIn` decodes nothing.
- [x] Tests, `tests/runtime/rt_native_term_runtime.rs`, feeding SGR sequences to a
      pty: a click at a cell returns `Down` then `Up` with correct coords; a drag
      returns `Drag`; wheel returns `ScrollUp`/`ScrollDown`; ctrl-click sets
      `.ctrl`; interleaved keyboard bytes still reach `io::readChar`; an
      unrecognized `\x1b[Z` passes through untouched; **`io::pollInput` returning
      TRUE is always followed by a non-blocking `readChar`**; **a `readLine`
      during mouse mode echoes no escape bytes**.
      — six cases, all passing. Fed through **stdin and the injection variable**
      rather than a pty: the properties under test are about the byte stream, and
      a pipe drives them deterministically where a pty adds timing. The pty is
      still used where it is the only thing that can prove the claim — the
      opt-in case checks silence on a real tty, because tracking escapes would
      only ever be written to one.
- [x] Thread test: a worker without `openStdIn` polling mouse gets only `None`
      (and a raw stdin read still traps `ErrInvalidContext`).
      — `native_term_mouse_is_per_thread_and_needs_stdin`. It also asserts the
      main thread *does* receive the events, or the worker's silence would prove
      nothing.

Acceptance: the pty-driven rt test decodes all six event kinds with correct
coords/modifiers; keyboard-passthrough, unrecognized-escape-passthrough,
pollInput-honesty and no-echo all hold; the mouse-off read path is byte-identical.
**Met.** `cargo test --test rt_native_term_runtime` → `13 passed; 0 failed`
before the thread case, `15` after. All six kinds decode with the exact
row/column each report encodes (the wire's `x` is the COLUMN, so the pair
transposes — every case pins both numbers, because that transposition is the
likeliest decoder bug and the hardest to spot). `CHARS:abcdef` survives
interleaved reports; `ESC [ Z` and a bare `ESC` pass through byte for byte;
`pollInput`+`readChar` retrieves every character with no block; a `readLine`
brackets its cooked window with the reset/set pair.
Commit: 7eed59fb2

### Phase 3 — Goldens + docs

- [x] Regenerate `scripts/artifact-gate.sh <exe> term` ×5 and confirm fixtures
      that never enable mouse do **not** diff. Update the rendered man prose in
      `func_enable_mouse.rs`/`func_poll_mouse.rs` from stub wording to real
      behavior (verify with `mfb man term enableMouse`, `mfb man term pollMouse`);
      add the term-backend spec section for the input decoder, the 100 ms TTL,
      broadcast semantics, and the `pollInput`/`readLine` interactions.
      — **Nothing needed regenerating.** The sweep reports `0 diff(s)` across all
      2058 goldens, which is the stronger result the phase was checking for: not
      "the term fixtures were re-blessed" but "no fixture moved at all". Getting
      there took one real fix (Corrections B7). Man prose updated on both members
      — `enableMouse` now documents the `readLine` suspension and `pollInput`'s
      continued honesty; `pollMouse` documents overwrite-on-full alongside the
      TTL, and that events are per-thread. Spec section added.

Acceptance: `scripts/artifact-gate.sh <exe> term` passes;
`scripts/man-examples-gate.sh` passes; `scripts/man-census.sh --memory-scope`
reports 0 unclassified hits.
**Met.** `./scripts/artifact-gate.sh ./target/release/mfb all` → `1466 tests,
1637 build(s), 2058 golden(s) checked, 0 diff(s)`. `man-run-examples.sh term
--run` → `47 built, 37 ran, 0 failed` apart from the 10 pre-existing
no-controlling-terminal entries already in `man-examples-not-run.txt`; both mouse
members' examples run. `man-census.sh --memory-scope term` → **0** unclassified
(one reword needed: "consumed" is on the banned list too).
Commit: 7eed59fb2

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

**B1 — none of the three monotonic-nanos emitters is reusable as-is, and the
reason matters.** §2's "Verified properties" reads the existence of three
emitters as "strong evidence the primitive travels". Reading them, it is closer
to evidence that it has been re-implemented three times because none of them is
general:

| Candidate | Reusable? | Why not |
|---|---|---|
| `perf::emit_read_monotonic_nanos` (`perf.rs:448`) | **No** | Hard-codes `CLOCK_MONOTONIC_DARWIN = "6"` (`perf.rs:50`) with no platform branch. Linux's `CLOCK_MONOTONIC` is `1`, so calling it from Linux codegen would read the wrong clock. Private to `perf`. |
| `net::emit_monotonic_nanos` (`gen_ping.rs:1220`) | **Partly** | Correctly uses `platform.clock_monotonic()`, but that accessor is `unreachable!("Windows ICMP reports its own RoundTripTime; no clock_gettime")` on Win64 (`win_x86_64/code.rs:3246`) — so a Windows build would panic the **compiler**. Private to `gen_ping.rs`. |
| `datetime::lower_monotonic_nanos` | **Correct, wrong shape** | The only one right on all three families: `QueryPerformanceCounter`/`Frequency` on Windows, `clock_gettime` elsewhere. But it is a whole `abi_function` member body — it writes `RESULT_VALUE_REGISTER`, branches to a caller-supplied `ErrOverflow` label, and addresses datetime-local frame constants (`TIMESPEC_OFFSET`, `WIN_QPC_FREQ_OFFSET`, `LOCALS_SIZE = 88`). |

**Windows answer, for plan-94-E:** yes, there is one — `QueryPerformanceCounter`
+ `QueryPerformanceFrequency`, with the nanos computed as
`(counter/freq)*1e9 + ((counter%freq)*1e9)/freq` so the fraction stays inside
`u64` (`counter*1e9` alone overflows in ~21 s at 10 MHz). E does not need to
invent one.

**What B does instead:** factors a `dst`-and-scratch-parameterised emitter out of
the datetime shape into the mouse module, so one implementation serves both the
ring stamp and every backend. It deliberately **drops the `ErrOverflow` trap**:
the stamp is only ever consumed as `now − stamp ≤ 100 ms`, and unsigned wrapping
subtraction is *correct* for any interval under 584 years, so trapping would add
a failure mode to the stdin read path in exchange for nothing. That is a
narrower contract than `datetime::monotonicNanos`, not a weaker one — it is why
the emitter is separate rather than a call into datetime's member.

**B2 — the planned "test-only hook" cannot be a registry member.** Phase 1 called
for "a small internal `enqueue` helper used by Phase 2 and by a test-only hook",
with the hook enqueuing synthetic events so the ring is proven before the read
path is touched. Measured: a `RegistryFunction` with `internal_only: true`
resolves **only** from toolchain-provided source — `builtins::is_internal_only_call`
gates it in `resolver::resolution` to non-`internal` files
(`src/codegen/registry/mod.rs:505-512`). An rt test's MFB source is user source,
so it could not call such a hook at all. Making it non-internal would put a
synthetic-event injector in the public `term::` surface permanently, which is
worse than the problem.

Replaced with the affordance the tree already uses for exactly this: an
**environment variable**, mirroring `MFB_WINAPP_INPUT`
(`src/target/win_x86_64/app/mod.rs:654`, "a test affordance … so the subclass →
pipe → readLine round-trip is box-provable over ssh without a keyboard").
`MFB_MOUSE_INJECT` carries **raw SGR bytes** — the same bytes a terminal would
send — which `enableMouse(TRUE)` feeds through the decoder.

This is strictly better than the planned hook, for a reason worth stating: the
env var feeds the *real* SGR parser rather than bypassing it, so Phase 1 proves
the decode logic **and** the ring before either goes near the read path, which is
more than the synthetic hook would have proven. It also gives C, D and E a way to
exercise mouse on a box with no mouse.

**B3 — the §3e cooked-mode bracket must preserve the `Result` registers, and the
first version did not.** Wiring the mouse suspend/resume around `io::readLine`'s
cooked-mode window segfaulted every program that read a line with mouse enabled
(`EXIT=139`, measured on `printf 'hello\n' | fixture`).

Root cause, found by reading the neighbouring call rather than guessing: the
**resume** side runs *after* the read's result is already staged in the result
bank — which is exactly why `emit_console_raw_line_mode` carries a
`preserve_result` flag at that same position. The escape write clobbers the bank,
so `io::readLine` returned a wild pointer and the program faulted on first use of
the string. The suspend side has no such problem: nothing is staged before the
read.

Fixed by giving `emit_mouse_tracking_window` the same `preserve_result` flag and
setting it on the resume call only. Verified: `LINE:hello`, exit 0, with the
suspend/resume pair visible in the byte stream either side of the read.

Worth recording because the failure mode is quiet — the escapes are emitted, the
brackets look right in the output, and the corruption only shows when the
returned value is touched. **C/D/E: any emitter inserted after a staged result
must park the bank.**

**B4 — the pump's gate is compile-time, which is what makes the byte-identity
claim exact.** The plan says the pump is "gated on `_mfb_rt_mouse_mode`: zero ⇒
pass-through (identity, byte-identical to today)" (§3a), and separately that
"mouse-mode-**off** codegen for the read path MUST stay byte-identical to pre-B"
(§Byte-identity note). Those two cannot both be literally true: a runtime gate is
a load, a compare and a branch that were not there before, on every
`io::readChar` in every program.

Resolved by gating at **compile time** as well. The pump is emitted only when
`mouse_state_offset` is `Some` — i.e. only for a program that uses
`enableMouse`/`pollMouse` — so a program that never mentions the mouse gets
*exactly* its pre-B instruction stream, not that stream plus a mode test. The
runtime check remains inside a mouse program, for the window before
`enableMouse(TRUE)`.

This is the same key that reserves the arena region and emits the mouse data
objects, so all three appear and disappear together. It also means the read
helpers' frames grow by the decoder's 16-byte clock scratch **only** in a mouse
program: the scratch is appended past the base frame rather than carved out of
it, so no existing slot offset moves either.

**B5 — the flush needs a drain cursor; the first design lost bytes.** §3a says an
un-completable prefix is "flush[ed] back to the caller unchanged, one byte at a
time". Written literally — deliver `buf[0]`, shift the rest down — that is wrong
in a way the plan does not hint at: the leftover bytes are still in the parse
buffer, so the *next* call sees a non-empty buffer and treats the replayed bytes
as a live prefix being continued. An `ESC [ Z` is then re-parsed as the start of
a new sequence instead of delivered, and its bytes are eaten.

Fixed with an explicit one-based drain cursor
(`MOUSE_STATE_DRAIN_POS_OFFSET`) and a two-entry-point contract: the reader
drains owed bytes *before* it reads anything new, and only calls the decoder when
nothing is owed. No shifting, and the replay is in order by construction.

One-based rather than a plain index because `io::pollInput` needs to express
"owe the program `buf[0]`" — having read a byte to classify it, it must push that
byte back undelivered. A zero-based cursor spells that `0`, which is also how it
spells "nothing owed".

**B6 — `enableMouse` writes no terminal escapes in an `--app` build.** Not
something the plan raises, and it is a visible defect rather than a nicety: in
app mode stdout is the window's transcript, so `\x1b[?1000h` would be *displayed*
— the user would see `[?1000h` printed into their app — and no terminal is
listening to turn reporting on anyway. The mode word is still written in both
builds, because that is what plan-94-C/D/E's UI-thread handlers actually read.

Found by reasoning about where `term::enableMouse` lands in app mode: it is not
in any backend's `emit_app_term_helper`, so unlike `term::on` it falls through to
the console body and would have written to the transcript.

**B7 — a label is an instruction, and two of them broke byte-identity.** The
byte-identity gate caught this, which is what it is for: after the `pollInput`
work, `artifact-gate.sh … all` reported **7 diffs** — all five `io`
`.ncode` targets plus two app-mode `io` fixtures. None of them uses the mouse.

Root-caused by dumping one fixture rather than theorising
(`tests/byte-identity/io`, `_mfb_rt_io_io_pollInput`): its label list had gained
`ready_recheck` and `report_ready`. Both were pushed unconditionally while the
*code* between them was correctly gated on `mouse_state_offset` — so a non-mouse
program emitted no mouse logic but did emit two extra label instructions, which
the `.ncode` stream records.

Fixed by gating the two labels on the same condition as the code they serve;
re-measured `0 diff(s)` across all 2058 goldens. The lesson generalises past this
sub-plan: **in this emitter vocabulary a label is an instruction like any other**,
so "gate the logic" is not the same as "gate the emission", and `plan-94-C/D/E`
will each be adding handlers into existing procs where the same mistake is
available.

## Summary

B is the engine and the risk center: it changes stdin consumption at the one
function every read helper goes through. Placing the pump there rather than at the
console-only reader is what lets C–E inject bytes instead of building three more
event queues. Two read-path neighbours that did not exist when this plan was first
written — `io::pollInput` and the `readLine` cooked-mode restore — are now
first-class Phase 2 work rather than discoveries. It is fully runtime-testable
under a pty, so the risk is bounded by tests before any app backend depends on it.
