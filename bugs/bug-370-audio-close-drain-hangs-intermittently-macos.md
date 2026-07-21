# bug-370: `audio::close` on macOS intermittently never returns, hanging the program after the audio has finished playing

Last updated: 2026-07-20
Effort: medium (1h–2h) — a lost-wakeup race in the drain, not a logic error
Severity: **HIGH** (a hang, in the ordinary close path, on ~40% of runs)
Class: Correctness (concurrency — lost wakeup / missed condition re-check)

Status: Open
Regression Test: `tests/rt-behavior/audio` (new) — write more PCM than the
stream's four buffers hold, then `audio::close`, repeated enough times that a
40%-per-run hang cannot pass by luck.

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

## The size threshold is the tell

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

## Suggested Fix

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

## Impact

Every macOS program that plays more audio than its buffers hold and then closes
the stream — which is every realistic playback program, including the
`plan-58-D` deliverable `libsnd::loadSound` + `audio::write` path. The audio is
*correct and audible*; the program simply never exits. Lexical drop of an
`AudioOutput` calls the same close, so a program that never names `audio::close`
is equally affected.
