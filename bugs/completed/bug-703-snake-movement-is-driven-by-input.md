# bug-703: snake (and life) advance their simulation only on an idle input tick, so continuous input freezes movement and two turns in one tick can reverse into the neck

STATUS: FIXED (f9c2d75f4)

Last updated: 2026-09-26
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Closed
Regression Test: `tools/pty-probe/snake_life_advance_under_input.py` (a shared
pty-driving tool, not a `tests/examples/rs` integration test — see Open Decisions).

`examples/snake/src/main.mfb`'s main loop polls for a key with `io::pollInput` and
branches on whether one arrived: the `IF ready(...) THEN` side only updates the
turn direction (or handles quit/pause/restart), and `stepSnake` plus the wall/
self-collision check run **only** in the `ELSEIF` — i.e. only on a tick where no
key arrived within the poll window. A key arriving on every tick (a key held
down, or anything sending a keystroke faster than `frameDelayMs()` = 95ms) is
therefore enough to make the visible head stop advancing entirely; it resumes the
instant input goes idle. Confirmed live via a pty driver below: holding `d` for
1.5s freezes the head at a fixed cell for the whole hold, then it resumes
stepping once the key stream stops.

A second, independent defect in the same function: `directionFor`
(`examples/snake/src/main.mfb:177-183`) validates a new key against `current`,
the `dir` field the outer loop already may have mutated by an earlier keypress
this same tick — not against the direction the snake last actually **moved**.
Because a keypress mutates `dir` immediately (`main:276`) while a move only
happens on the idle `ELSEIF` tick, two turns delivered within one tick (e.g. `w`
then `a` ~10ms apart) are each validated against the *other just-applied* turn,
so the pair can compose into a net reversal that no single-turn check would
ever allow, sending the head directly into its own neck on the next real move.
Confirmed live below: reproduced 3/3 runs, ending the game (`GAME OVER`) within
~300ms of start.

`examples/life/src/main.mfb` has the identical first defect: `IF
io::pollInput(frameDelayMs()) THEN` (`main:323`) handles quit/pause/restart/
reseed only, and `stepGrid` plus `gen = gen + 1` run only in the `ELSEIF NOT
paused THEN` (`main:344-347`) — i.e. only when no key arrived. An unmapped key
(or any handled key) sent every tick freezes the generation counter. Confirmed
live below.

**The single correct behavior a fix produces:** the simulation (snake's move-and-
collide step; life's `stepGrid` generation) advances on a fixed wall-clock cadence
regardless of whether a key arrived that tick — "did we get input" and "is it
time to step" become independent questions — and a turn is validated against the
direction the snake last actually moved, not against a same-tick, not-yet-applied
turn.

References:

- `examples/water/src/main.mfb:268-341` and `examples/wind/src/main.mfb:396-487` —
  sibling examples that already use this fix's shape: they drain input in an inner
  `DO WHILE pending(0)` loop, then call `stepWater`/`stepSwarm` unconditionally
  every outer-loop iteration using `canvas::getFrameStats().dt`, never gated by
  whether a key arrived. `examples/swat/tui/src/main.mfb:373-410` and
  `examples/swat/canvas/src/main.mfb` (same shape) advance grass growth and the
  bug's schedule by comparing `nowMs()` to stored deadlines, also unconditionally.
  These are the working contrast cases this bug's fix should match.
- `mfb man datetime monotonicNanos` — confirms `datetime::monotonicNanos() AS
  Integer` exists and is monotonic, the timer primitive the fix should use for a
  console (non-canvas) example like snake/life that has no `canvas::getFrameStats()`.
- bug-699 (`bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md`) is
  **not** cross-linked as a cause here: `readKey`'s escape-decoding loop
  (`examples/snake/src/main.mfb:159-175`) is not implicated — every reproduction
  below used plain ASCII keys (`d`, `w`, `a`, `z`) that `readKey` decodes
  correctly on every run; the defect is purely in when `stepSnake`/`stepGrid` is
  called and what `directionFor` validates against.

## Failing Reproduction

Built with the in-tree compiler for the host target:

```
cd /Users/justinzaun/Development/mfb
target/release/mfb build examples/snake      # -> examples/snake/build/snake.out
target/release/mfb build examples/life       # -> examples/life/build/life.out
```

Driven through a real pty with a small Python script (stdlib `pty`/`os`/`select`
only; `pyte` — installed into a scratch venv at `/tmp/snakevenv`, since neither
`ptyprocess` nor `pexpect` nor `pyte` were present in the base environment — is
used to interpret the VT100 stream into a screen so the head glyph's actual
`(row, col)` and the status line's `GAME OVER`/`Gen N` text can be read directly,
rather than guessing from raw escape-byte offsets, which was tried first and gave a
false read (an early manual offset-based check wrongly suggested the head moved
every frame; the pyte-based screen check below is the one trusted, and it is
corroborated by matching freeze/resume timing against the exact key-send window in
all three runs).

`/tmp/snake_probe2.py` (hold + reverse modes; caps total runtime at 8s and always
`SIGKILL`s the child in a `finally`):

```python
#!/usr/bin/env python3
"""pty driver for bug-703, using a real VT100 screen (pyte) to read the head
glyph's actual (row, col) each frame, instead of guessing from raw escape
byte offsets.

Usage (inside /tmp/snakevenv, which has pyte installed):
  /tmp/snakevenv/bin/python3 /tmp/snake_probe2.py hold
  /tmp/snakevenv/bin/python3 /tmp/snake_probe2.py reverse
"""
import os, pty, select, sys, time, signal, fcntl, termios, struct
import pyte

BIN = "/Users/justinzaun/Development/mfb/examples/snake/build/snake.out"
HEAD = "☺"

def run(mode):
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(BIN, [BIN])
        os._exit(1)

    rows, cols = 40, 100
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    screen = pyte.Screen(cols, rows)
    stream = pyte.Stream(screen)

    t0 = time.monotonic()
    deadline = t0 + 8.0

    if mode == "hold":
        start_send = t0 + 0.3
        send_until = start_send + 1.5
        end_at = send_until + 1.2
    elif mode == "reverse":
        start_send = t0 + 0.3
        end_at = t0 + 3.0
    else:
        raise SystemExit("mode must be hold|reverse")

    next_send = start_send
    send_count = 0
    sent_reverse = False
    samples = []  # (t, head_pos_or_None, game_over)

    try:
        while True:
            now = time.monotonic()
            if now > deadline or now > end_at:
                break
            r, _, _ = select.select([fd], [], [], 0.02)
            if fd in r:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                stream.feed(chunk.decode("utf-8", errors="replace"))
                head_pos = None
                for y, line in screen.buffer.items():
                    for x, ch in line.items():
                        if ch.data == HEAD:
                            head_pos = (y, x)
                bottom_text = "".join(
                    screen.buffer[rows - 1][x].data if x in screen.buffer[rows - 1] else " "
                    for x in range(cols)
                ) if (rows - 1) in screen.buffer else ""
                game_over = "GAME OVER" in bottom_text
                samples.append((round(now - t0, 3), head_pos, game_over))

            if mode == "hold":
                if now < send_until and now >= next_send:
                    try:
                        os.write(fd, b"d")
                    except OSError:
                        break
                    send_count += 1
                    next_send = now + 0.015
            elif mode == "reverse":
                if not sent_reverse and now >= next_send:
                    try:
                        os.write(fd, b"w")
                        time.sleep(0.010)
                        os.write(fd, b"a")
                    except OSError:
                        break
                    sent_reverse = True
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.waitpid(pid, 0)
        try:
            os.close(fd)
        except OSError:
            pass

    print(f"[{mode}] sent {send_count} key(s)", file=sys.stderr)
    last = None
    for t, pos, over in samples:
        if pos != last:
            tag = " GAME_OVER" if over else ""
            print(f"t={t:6.3f}s head={pos}{tag}")
            last = pos
    if any(over for _, _, over in samples):
        print("GAME OVER observed in stream")

if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "hold"
    run(mode)
```

Commands run and verbatim output:

```
$ python3 -m venv /tmp/snakevenv && /tmp/snakevenv/bin/pip install --quiet pyte
$ /tmp/snakevenv/bin/python3 /tmp/snake_probe2.py hold
[hold] sent 60 key(s)
t= 0.014s head=(19, 50)
t= 0.110s head=(19, 51)
t= 0.193s head=(19, 52)
t= 0.295s head=(19, 53)
t= 1.878s head=(19, 54)
t= 1.979s head=(19, 55)
t= 2.086s head=(19, 56)
...
$ pkill -f examples/snake/build/snake.out
```

- Observed: the head steps normally every ~95-100ms until `t=0.295s` (just after
  the 15ms-interval `d` stream starts at `t=0.3s`), then **does not move again
  until `t=1.878s`** — frozen for ~1.58s while `d` (60 keystrokes) is delivered
  continuously — and resumes stepping every ~95-100ms immediately once the
  keystroke stream stops (`send_until` = 1.8s).
- Expected: the head advances on the fixed ~95ms cadence throughout, independent
  of whether a key is arriving that tick.

Reverse (two-turn) case, run 3 times for flakiness (all 3 ended the game):

```
$ /tmp/snakevenv/bin/python3 /tmp/snake_probe2.py reverse   # x3
=== run 1 ===
t= 0.016s head=(19, 50)
t= 0.122s head=(19, 51)
t= 0.351s head=(19, 53)
GAME OVER observed in stream
=== run 2 ===
t= 0.010s head=(19, 50)
t= 0.089s head=(19, 52)
t= 0.301s head=(19, 53)
GAME OVER observed in stream
=== run 3 ===
t= 0.007s head=(19, 50)
t= 0.086s head=(19, 52)
t= 0.284s head=(19, 53)
GAME OVER observed in stream
```

(`sent 0 key(s)` printed by these runs is a counter bug in the probe script
itself — `send_count` is only incremented in the `hold` branch — not evidence
about the game; the `os.write` calls for `w` then `a`, 10ms apart, did execute,
and `GAME OVER` appearing within ~300ms of process start, well before any wall
or self-collision from continuous rightward movement could occur on a fresh
board, is what proves the reversal.)

- Observed: sending `w` then `a` ~10ms apart, starting ~0.3s after launch,
  reliably (3/3) ends the game within ~50ms of the second keystroke.
- Expected: a single 90-degree turn per tick is the only turn the game accepts;
  two turns delivered before any move happens must not compose into a reversal,
  and the snake (moving right, trailing left per `initialSnake`) must not be able
  to step directly into its own neck from two turns alone.

`examples/life`'s analogous freeze, `/tmp/life_probe.py` (same shape as the
snake `hold` driver, sending the **unmapped** key `z` — which falls through
every `ELSEIF` in `main:326-341` and does nothing — every 15ms, reading `Gen N`
out of the bottom status line via the same `pyte` screen):

```
$ /tmp/snakevenv/bin/python3 /tmp/life_probe.py
sent 60 'z' keys
t= 0.012s gen=0
t= 0.133s gen=1
t= 0.239s gen=2
t= 1.888s gen=3
t= 1.988s gen=4
...
$ pkill -f examples/life/build/life.out
```

- Observed: `gen` advances every ~100-130ms up to `gen=2` at `t=0.239s` (just
  after the `z` stream starts at `t=0.3s`), then is frozen at `2` until
  `t=1.888s` (~1.65s, spanning the whole 1.5s hold window), then resumes.
- Expected: `gen` advances on the fixed frame cadence regardless of a key
  arriving that has no assigned handler (or any handler at all).

Contrast case that works correctly today: `examples/water/src/main.mfb` and
`examples/wind/src/main.mfb` already decouple input from simulation stepping —
`stepWater`/`stepSwarm` run unconditionally every frame using
`canvas::getFrameStats().dt`, so continuous mouse/keyboard input during those
apps does not freeze the water level or the wind swarm (not independently pty-
probed for this bug, since the point is the code shape — `stepWater`/`stepSwarm`
are plainly outside any input-arrived conditional — not a new defect to chase).

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 (host), `target/release/mfb build examples/snake` | pty, `d` held 1.5s @ 15ms interval | freezes ✗ |
| macOS aarch64 (host), same build | pty, `w`,`a` ~10ms apart near start | GAME OVER within ~300ms, 3/3 ✗ |
| macOS aarch64 (host), `target/release/mfb build examples/life` | pty, `z` held 1.5s @ 15ms interval | freezes ✗ |

## Root Cause

- `examples/snake/src/main.mfb:main` (lines 218-301): the outer `DO WHILE
  running` loop calls `ready(frameDelayMs())` (line 260) and branches:
  - `IF` (261-277): handles quit/pause/restart, and otherwise only calls
    `dir = directionFor(key, dir)` (276) — no board mutation, no collision
    check.
  - `ELSEIF NOT paused AND NOT gameOver AND NOT won` (278-300): the **only**
    branch that computes `nextHead`, checks wall/self collision, and calls
    `stepSnake` (288).
  Because these are mutually exclusive on the same `IF`/`ELSEIF`, any tick
  where `ready()` observes a pending key takes the first branch and skips the
  step entirely, for as long as a new key keeps arriving inside each poll
  window.
- `examples/snake/src/main.mfb:directionFor` (177-183): compares the requested
  turn to `current.dy`/`current.dx`, where `current` is `dir` — the value the
  *previous* keypress this same tick may have already written via `main:276`,
  since a keypress mutates `dir` immediately but `stepSnake` (the only place
  that would fix `dir` into "last moved direction") does not run on that tick.
  Two keypresses inside one `frameDelayMs()` window are therefore each checked
  against the other's *tentative* result, not against the direction the snake
  actually last moved, so `RIGHT -> (w) -> UP -> (a) -> LEFT` passes both
  single-turn checks and nets a full reversal before any move enforces the
  turn rule.
- `examples/life/src/main.mfb:main` (lines 259-349): identical shape —
  `IF io::pollInput(frameDelayMs()) THEN` (323) handles quit/pause/restart/
  reseed only; `ELSEIF NOT paused THEN` (344-347) is the only branch that calls
  `stepGrid` and increments `gen`. A key arriving every tick — mapped or not —
  takes the `IF` branch and the generation never advances.

## Goal

- Snake's head position and life's `gen` counter advance on a fixed wall-clock
  cadence (~`frameDelayMs()`) regardless of whether a key arrived that tick.
- A turn is accepted or rejected by comparing it to the direction the snake
  last actually *moved*, not to a same-tick, not-yet-applied `dir` mutation; two
  turns delivered inside one tick can never compose into a reversal that a
  single turn could not achieve.

### Non-goals (must NOT change)

- Snake's and life's controls, key bindings, or the visual glyphs/colors.
- `readKey`'s escape-decoding shape (bug-699's territory, not touched here).
- The turn-based (not per-tick-simulated) games — `dungeon`, `ai_chat`,
  `browser` — must keep moving only on an actual keypress; that is their
  correct behavior, not an instance of this bug (see Blast Radius).
- A "fix" that makes the freeze harder to hit without removing it (e.g. only
  widening the poll window, or draining multiple keys per tick without also
  decoupling the step) is explicitly rejected — the goal is that "did input
  arrive" and "is it time to step" are independent, not that the race window
  is merely narrower.

## Blast Radius

Found via `grep -rn "pollInput" examples/` (10 hits) plus reading each hit's
loop:

- `examples/snake/src/main.mfb:main` — fixed by this bug (both sub-issues).
- `examples/life/src/main.mfb:main` — fixed by this bug (freeze sub-issue; life
  has no turn/direction concept, so the reversal sub-issue does not apply to it).
- `examples/dungeon/src/main.mfb:main` (`IF ready(16) THEN ... MATCH
  commandFor(readKey()) ... CASE Move(mv): px = px + stepX(dir) ...`, lines
  117-143) — unaffected: dungeon is turn-based (the player only moves in
  response to an actual keypress; there is no autonomous per-tick simulation
  that a `ready()`-gated `ELSE` is supposed to advance), so "no move while no
  key" is correct here, not a bug.
- `examples/water/src/main.mfb:main` (268-345) and `examples/wind/src/main.mfb:main`
  (396-487) — unaffected: both drain pending input in an inner `DO WHILE
  pending(0)` loop, then call `stepWater`/`stepSwarm` unconditionally every
  outer-loop pass using `canvas::getFrameStats().dt` — the step is never inside
  an `IF ready(...)`/`ELSE` split at all. This is the shape this bug's fix
  should copy.
- `examples/swat/tui/src/main.mfb:main` and `examples/swat/canvas/src/main.mfb:main`
  — unaffected: grass growth and the bug's appear/flee schedule are advanced by
  comparing `nowMs()` to stored `nextGrassAt`/`bugEventAt` deadlines
  unconditionally each pass (lines ~373-410 in `swat/tui`); `IF ready(...) THEN`
  there only handles keys (quit/pause/restart), matching the water/wind shape,
  not snake/life's.
- `examples/ai_chat/src/main.mfb:main` and `examples/browser/app/src/main.mfb:main`
  — unaffected: both are input-driven UIs (chat log, page renderer) with no
  autonomous per-tick simulation to advance; their `IF ready(...) THEN` bodies
  only ever handle keys, and nothing analogous to `stepSnake`/`stepGrid` exists
  in an `ELSE` there.

## Fix Design

Recommended: give snake and life a fixed-cadence timer independent of input,
using `datetime::monotonicNanos()` (confirmed to exist:
`datetime::monotonicNanos() AS Integer`, `mfb man datetime monotonicNanos`) —
the same decoupling `water`/`wind` already get for free from
`canvas::getFrameStats().dt`, which snake/life (plain terminal, not
`Mode.Canvas`) do not have:

- Track `nextStepAt` (nanoseconds), advanced by `frameDelayMs() * 1_000_000`
  each time a step actually runs.
- Each loop iteration: poll for input with a short, fixed wait (so the loop
  still sleeps rather than busy-spinning) and handle at most the keys that
  arrived; separately, if `datetime::monotonicNanos() >= nextStepAt`, run the
  step (`stepSnake`/`stepGrid`) and set `nextStepAt` forward — regardless of
  whether a key was also handled this pass.
- For snake's reversal: keep a `lastMovedDir` that is set only inside
  `stepSnake`'s caller right after a move actually happens, and validate
  `directionFor(key, lastMovedDir)` instead of `directionFor(key, dir)`; `dir`
  becomes purely "the turn queued for the next real move" and is no longer also
  used as the turn-legality reference.

Rejected alternative: draining all queued keys in an inner loop before deciding
whether to step (mirroring water/wind's `DO WHILE pending(0)`) without also
adding a wall-clock gate. This narrows the race (a key burst no longer starves
the step indefinitely) but does not fix it: a key arriving at exactly the poll
cadence would still perpetually win the race, and it does nothing for the
reversal sub-issue, which is about what `directionFor` validates against, not
how many keys are drained per tick.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Commit `/tmp/snake_probe2.py` and `/tmp/life_probe.py` (or an equivalent)
      as a runtime test, per the project's test conventions for example
      programs — see Validation Plan for the proposed location.
- [x] Confirm the failing tests fail against current `snake.out`/`life.out`
      (already demonstrated manually above; wire the same assertions into the
      committed test).
- [x] Blast-radius audit above is complete with a verdict per site (done in
      this document).

Acceptance: the new test(s) fail for the documented reason (head/gen frozen
during continuous input; reversal ends the game); the audit list has a verdict
per site.
Commit: f9c2d75f4

### Phase 2 — the fix

- [x] `examples/snake/src/main.mfb`: replace the `IF ready(...) THEN /
      ELSEIF` split with an input poll (short fixed wait) plus an independent
      `datetime::monotonicNanos()`-gated step, so `stepSnake` and the
      collision check run on cadence regardless of input.
- [x] `examples/snake/src/main.mfb:directionFor` / `main`: add `lastMovedDir`,
      set only when a move actually happens; validate new turns against it
      instead of against `dir`.
- [x] `examples/life/src/main.mfb`: same timer decoupling for `stepGrid`/`gen`.

Acceptance: the Phase 1 test(s) pass; `dungeon`, `water`, `wind`,
`swat/tui`, `swat/canvas`, `ai_chat`, `browser` behavior is unchanged (Non-goals);
holding a key no longer freezes the head or the generation counter; two turns in
one tick no longer produce a reversal `directionFor` would reject one at a time.
Commit: f9c2d75f4

### Phase 3 — regenerate expected outputs + full validation

- [x] No goldens are shifted by this fix (examples have none); nothing to
      regenerate.
- [x] `bash scripts/build-examples.sh` — builds every example (including snake
      and life) for every supported target into `target/examples/`; confirms
      the fix compiles on every target, not just macOS aarch64.
- [x] Re-run both pty reproductions end-to-end against the fixed binaries and
      confirm the head/`gen` no longer freeze and the reversal no longer ends
      the game (10 repeats of the reversal case, given its timing sensitivity).

Acceptance: `scripts/build-examples.sh` exits 0; the reproduction no longer
fails on macOS aarch64 (the only host available in this session).
Commit: f9c2d75f4

## Validation Plan

- Regression test(s): proposed new integration test
  `tests/examples/snake_life_advance_under_input.rs`, built on the same pty +
  `pyte`-equivalent approach as the manual probes above — builds `snake`/`life`
  via the release compiler, drives each through a pty with a held/repeated key
  for >1 poll-window's worth of ticks, and asserts the head position / `gen`
  advances despite continuous input, plus a repeated-turn case asserting the
  game does not end from two same-tick turns alone. Proposed location: a new
  `tests/examples/` integration test (there is no existing `tools/<name>/`
  convention for a per-example pty probe today — `tools/` entries are
  generators/oracles/benchmarks with their own README, and this is closer to a
  `cargo test` integration test that happens to build and drive an example
  binary, matching how other runtime behavior is tested under `tests/`). A pty
  dependency (e.g. a `portable-pty`-equivalent, or raw `openpty`/`fork` on
  Unix) would need to be added if `tests/` has no such harness yet — that
  investigation is part of Phase 1, not assumed here.
- Runtime proof: the pty reproductions above, re-run against the fixed
  binaries.
- Doc sync: none expected — the games' `README.md`/man-page-adjacent comments
  don't document the timing internals this bug touches.
- Full suite: `bash scripts/build-examples.sh`; the project's normal `cargo
  test` if the new integration test lives under `tests/`.

## Open Decisions

- Exact poll-wait duration once stepping is on its own timer (e.g. a short
  fixed `io::pollInput(10)` regardless of how much of `frameDelayMs()` remains,
  vs. computing the remaining time until `nextStepAt` each pass) — **resolved:
  computed the remaining time each pass** (matches `water`/`wind`'s
  `pending(0)` + unconditional step shape most closely).
- Whether `tests/examples/` should gain a small pty-driving helper shared by
  future example probes, or each test rolls its own — **resolved (per the
  fix-bug lane's shared instructions for this run): a shared pty-driving tool
  under `tools/pty-probe/`, not a `tests/examples/*.rs` integration test.**
  `tools/pty-probe/pty_probe.py` is shared with bug-708's regression probe
  (`tools/pty-probe/life_quit_no_return.py`); the proposed
  `tests/examples/snake_life_advance_under_input.rs` name in this doc's
  original Regression Test line was never created — the landed regression
  test is `tools/pty-probe/snake_life_advance_under_input.py`.

## Summary

The engineering risk is entirely in Phase 2: decoupling "did input arrive" from
"is it time to step" in two small terminal-UI loops, and re-pointing
`directionFor`'s reference from a same-tick mutable `dir` to a separately-
tracked last-moved direction. Nothing about the fix changes controls, visuals,
or any other example — `water`/`wind`/`swat/tui`/`swat/canvas` already use the
correct shape and are left untouched. The audit and the reproduction are both
complete and were independently confirmed live in this session (not inferred
from reading the source), including one false lead (raw-escape-byte offset
guessing) that was caught and discarded before writing any claim into this
document.
