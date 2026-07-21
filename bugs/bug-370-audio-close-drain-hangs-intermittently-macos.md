# bug-370: `audio::close` on macOS intermittently never returns, hanging the program after the audio has finished playing

Last updated: 2026-07-20
Effort: medium (1h–2h) — a lost-wakeup race in the drain, not a logic error
Severity: **HIGH** (a hang, in the ordinary close path, on ~40% of runs)
Class: Correctness (concurrency — lost wakeup / missed condition re-check)

Status: Fixed (2026-07-21)
Regression Test: none added — see "Why no automated test" below.

Closing a macOS `AudioOutput` waits on a condition variable until the callback
thread has handed back all four of the stream's AudioQueue buffers. That wait
**intermittently never completes**: the audio plays to the end, and then the
program hangs in `audio::close` forever.

Measured on this host, same probe, 15-second timeout per run:

| build | result |
|---|---|
| pre-session (`0677ce819^`) | OK, HANG, OK, HANG, OK — **2/5 hang** |
| current `HEAD` | HANG, HANG, OK, OK, OK, OK — **2/6 hang** |

**Pre-existing, and not caused by plan-58.** The two builds hang at
indistinguishable rates. An earlier single-sample comparison suggested plan-58
had caused it — that was wrong, and it is why the table above has five and six
samples rather than one each.

The single correct behavior a fix produces: `audio::close` returns once the
queued audio has finished sounding, on every run.

References:

- `mfb man audio close` — "close holds the stream mutex and waits on the stream
  condvar until the free-buffer stack holds all four of the stream's AudioQueue
  buffers, which happens only once the callback thread has handed back every
  buffer it was playing."
- `src/target/shared/code/audio/macos.rs` — the drain wait loop and the
  AudioQueue callback that pushes buffers back onto the free stack.
- `.ai/compiler.md` — this compiler emits no atomics, so all cross-thread sync is
  pthread mutex/cond (plan-33-A §6). That is the mechanism under suspicion.

## Failing Reproduction

No native binding, no libsnd, no file I/O — a synthesized silent buffer:

```basic
IMPORT io
IMPORT audio
IMPORT collections

FUNC main AS Integer
  MUT pcm AS List OF Byte
  MUT i AS Integer = 0
  WHILE i < 44100
    pcm = collections::append(pcm, toByte(0))
    i = i + 1
  WEND
  io::print("bytes=" & toString(len(pcm)))
  RES out AS AudioOutput = audio::openOutput(44100, 2, 512)
  io::print("opened")
  audio::write(out, pcm)
  io::print("wrote")
  audio::close(out)
  io::print("closed")
  RETURN 0
END FUNC
```

A hanging run prints `bytes=44100`, `opened`, `wrote` — and never `closed`.

## The size threshold is the tell (the reasoning here is wrong — see Root Cause)

| PCM written | frames | result |
|---|---|---|
| 4096 bytes | 1024 | **always completes** (5/5) |
| 44100 bytes | 11025 | hangs ~40% |

The stream is opened with `bufferFrames = 512` and holds **four** buffers, so it
can hold 2048 frames in flight. 1024 frames fit entirely — the callback never has
to recycle a buffer, so the free stack is full the moment `close` looks at it and
the wait is satisfied immediately, with no window to lose a wakeup in.

11025 frames force repeated recycling: enqueue, play, callback returns the buffer,
enqueue again. That is where the race lives.

## Suggested Fix (as first written — WRONG, kept as a record)

Treat it as a lost wakeup. The two shapes to check, in order:

1. **`close` evaluates the predicate before waiting, under the mutex.** If it
   waits unconditionally and the callback returned the last buffer between
   `write` finishing and `close` acquiring the mutex, the signal is already gone
   and nothing will ever signal again — the queue is idle. This matches the
   observed profile exactly: it needs the callback to be *nearly* done, which is
   why it is intermittent and why the never-recycled case never trips.
2. **The callback signals while holding the same mutex**, and `close` re-checks
   the predicate in a `while` loop rather than an `if` after each wake.

A timeout on the wait would mask this rather than fix it, and would trade a hang
for truncated audio; the predicate/locking discipline is the actual fix.

## Root Cause (measured)

**It is not a lost wakeup, and the condvar code was never at fault.** Both shapes
above were already correct in the tree: `close` checks `free_top >= NUM_BUFFERS`
under the mutex before waiting and re-checks in a `while` loop, and the callback
pushes and signals while holding that same mutex. Reading the emitted aarch64
back confirmed the register allocation and the cond/mutex argument addresses too.

The drain never completes because **a buffer never comes back**. At a hang, the
state page reads `free_top = 3`, `closed = 0`, `started = 1`, with only three
distinct pointers on the free stack — the queue is holding a fourth buffer it
will never finish. The AudioQueue IO thread is alive and rendering the whole
time, so nothing is deadlocked or dead.

`close` is not involved at all. A probe that writes the PCM and then merely polls
`audio::available` for 2.5 s, never calling `close`, plateaus at 1536 frames
(three buffers) on exactly the runs that would have hung, and 2048 (four) on the
runs that would not.

What strands the buffer is **enqueuing a buffer that holds less than a full
`bufferFrames`**. Sizes sweep cleanly, `bufferFrames = 512`, `bytesPerFrame = 4`,
so one buffer is 2048 bytes:

| PCM written | shape | stranded |
|---|---|---|
| 8192 / 10240 / 43008 bytes | exact multiples of 2048 | 0 / 6 runs |
| 44100 bytes | 21 full + one 1092-byte tail | 3 / 6 runs |

Starvation is *not* the trigger: writing twelve full 2048-byte buffers with a
deliberate stall between each — guaranteeing the queue runs dry repeatedly —
stranded nothing in 6 runs. Only the short buffer does it.

The mechanism, confirmed directly: attaching to a hung process and calling
`AudioQueueEnqueueBuffer` with one more *full* buffer released the stranded
buffer and the program ran to completion, printing `closed`. The queue holds a
partly-filled buffer waiting for enough data to complete a device period; at end
of stream that data never arrives, so the buffer is never finished, its callback
never runs, and the drain waits forever. Whether the tail happens to land on a
period boundary is what made it look intermittent.

Two fixes were tried and **measured to fail**, rather than reasoned about:
re-issuing `AudioQueueStart` after every enqueue (14 hangs / 25) and calling
`AudioQueueFlush` before the drain (14 / 25). Neither makes the queue finish a
short buffer.

## The Fix

Never hand the queue a partial buffer.

- `write` fills whole buffers only. A tail too short to fill one is carried in
  the stream state (`S_PENDING_BUF` / `S_PENDING_FILL`) instead of being
  enqueued, and the next `write` resumes filling that same buffer — so
  successive writes still join without a gap.
- `close` pads whatever is left over with silence up to a whole buffer, enqueues
  it, and only then drains. At most one buffer of trailing silence (<12 ms at
  44.1 kHz) is added, and only when the stream did not end on a boundary.
- If the device rejects that padded buffer, `close` returns it to the free stack
  under the mutex, so the error path cannot reintroduce the hang.

Result on the failing reproduction: **0 hangs / 30 runs**, from 7 / 10 before.
Sizes 1024, 4096, 8192, 44100 and 45056 bytes are all clean at 8 runs each.
441 writes of 1000 bytes each (a worst case for the carry logic: every write ends
mid-buffer) plays for ~2.65 s against 2.50 s of audio, so no PCM is dropped or
duplicated. Full acceptance: 1069 tests pass.

## Why no automated test

A test that proves this needs real playback: it has to write PCM ending
mid-buffer, close, and observe that close returns. The acceptance harness has no
device-skip or platform gate, and the existing audio codegen-cover test is
compile-only precisely because "these open devices that do not exist in a test
run". An execution test would therefore hang for its timeout on every machine
without an audio output device, including the Linux boxes. Reproduce by hand with
the program above.

**The coverage that *should* have caught this is broken.**
`tests/rt-behavior/codegen-cover/cover-audio/golden/*.ncodesum` exists for all
three targets, and that test does exercise `openOutput`/`write`/`close` — but
nothing in the repo ever reads a `.ncodesum` file. `test-accept.sh` only passes
`-ncode` when a `.ncode` golden is present, so these three goldens are inert and
this change altered audio codegen without moving them. Worth its own bug.

## Impact

Every macOS program that plays more audio than its buffers hold and then closes
the stream — which is every realistic playback program, including the
`plan-58-D` deliverable `libsnd::loadSound` + `audio::write` path. The audio is
*correct and audible*; the program simply never exits. Lexical drop of an
`AudioOutput` calls the same close, so a program that never names `audio::close`
is equally affected.
