# bug-810: `audio::render` has no bound on `noteFrames`; an extreme value runs until out of memory instead of raising a documented error

Last updated: 2026-10-04

Effort: medium (1h–3h)

Severity: MEDIUM

Class: Correctness

Status: Open
Regression Test: proposed `tests/runtime/rt_audio_render_huge_note_frames.rs`

`audio::render`'s man page documents `ErrOverflow`, `ErrFloatNaN`,
`ErrInvalidFormat` and `ErrOutOfMemory` as its only raisable errors, and states
"the returned list is `note.noteFrames * 2` bytes long". For an
`AudioNote.noteFrames` near `Integer.MAX` (`9223372036854775807`), the
implementation does not raise `ErrOverflow` (or any other documented error) up
front; it enters a `FOR i = 0 TO n - 1` loop and keeps appending two bytes per
iteration, growing memory without bound, for as long as the process is left
running — reaching 2+ GB RSS and still climbing. `ErrOverflow` is reachable in
principle, but not on this path for this input: the only check in the body is
`__audio_clampS16` on a per-sample computed value, not on `noteFrames` itself.

References:

- `src/codegen/builtins/audio/func_render.rs:49-60` (`DESC`) — the documented
  Errors contract (`ErrInvalidFormat`, `ErrOverflow`, `ErrFloatNaN`,
  `ErrOutOfMemory`) and the `noteFrames * 2` byte-length promise.
- `src/codegen/builtins/audio/func_render.rs:13-46` (`BODY`,
  `__audio_render`) — the unbounded `FOR i = 0 TO n - 1` loop.
- Found during bug-807 follow-up work (bug-staging-7) reproducing findings
  reported against `http`/`canvas`/`audio`/`csv`/`mfb audit`.

## Failing Reproduction

```sh
mkdir -p /tmp/mfb-f3/src
cat > /tmp/mfb-f3/project.json <<'JSON'
{"name": "f3", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-f3/src/main.mfb <<'BASIC'
IMPORT io
IMPORT audio

FUNC main() AS Nothing
  LET env = audio::AudioEnvelope[0, 0, 0, 0, 32767]
  LET note = audio::AudioNote[440.0, 9223372036854775807, env, 0.8]
  LET tone = audio::render(note) TRAP(e)
    io::print("raised " & toString(e.code) & " msg=" & e.message)
    RETURN
  END TRAP
  io::print("len=" & toString(len(tone)))
END FUNC
BASIC
target/release/mfb build /tmp/mfb-f3
/tmp/mfb-f3/build/f3.out &
PID=$!
sleep 15
ps -o pid,rss,vsz -p "$PID"   # still running, RSS climbing
kill -9 "$PID"
```

- Observed (macos-aarch64, `bug-staging-7` release build at
  `/tmp/bs7-target/release/mfb`): the process is still running 15 seconds in,
  RSS at 2,233,104 KB (≈2.1 GB) and climbing; nothing is printed to stdout —
  neither the `TRAP` handler nor a normal return fires before the process was
  killed. Left running longer, it would continue consuming memory without any
  check ever stopping it.
- Expected: `audio::render` raises a documented error (`ErrOverflow` is the
  natural fit — the resulting `List OF Byte` length `noteFrames * 2` itself
  overflows `Integer` for this input) before entering the per-frame loop,
  rather than running unboundedly.

## Root Cause

`__audio_render`'s body (`src/codegen/builtins/audio/func_render.rs:13-46`)
takes `n AS Integer = note.noteFrames` directly from the caller-supplied
record with no check against any bound, and loops `FOR i = 0 TO n - 1`
unconditionally. The documented `ErrOverflow` is reachable only indirectly,
through `toInt(wave * env * note.gainOverall)` inside the loop
(`toInt` on a `Float` raises `ErrOverflow`/`ErrFloatNaN` per bug-807's fix to
that conversion) — a per-sample check that has nothing to do with the
magnitude of `n` itself, and the per-sample value here (`wave` bounded to
±1, `env` bounded by `sustain`/`peak` ≈ 32767, `gainOverall` 0.8) never
reaches an overflowing range. There is no check anywhere in the body of the
form "does `noteFrames * 2` fit the destination" before the loop starts.

## Goal

`audio::render` raises `ErrOverflow` immediately (before any allocation or
loop iteration) for an `AudioNote.noteFrames` whose resulting byte length
(`noteFrames * 2`) does not fit the list the implementation can produce, or
for which the loop bound itself would not complete in bounded time/memory. A
non-pathological `noteFrames` (the documented common case — seconds to
minutes of audio at 48 kHz) is unaffected.

### Non-goals (must NOT change)

- No change to `render`'s output for any `noteFrames` the current tests and
  examples already exercise.
- No change to the per-sample `ErrFloatNaN`/`ErrOverflow` checks already
  reachable through `toInt`.

## Blast Radius

- `audio::render` (`src/codegen/builtins/audio/func_render.rs`) — the fix
  site.
- `audio::openOutput`/`audio::write` callers that pass a `render`ed tone are
  unaffected (bounded `noteFrames` in every example and test:
  `grep -rn "AudioNote\[" tests/ examples/`).

## Phases

### Phase 1 — failing test

Add `rt_audio_render_huge_note_frames.rs`: call `render` with `noteFrames`
near `Integer.MAX`, assert it raises `ErrOverflow` within a short, bounded
wall-clock budget (e.g. 2 seconds) rather than running unboundedly. Confirm
red (currently times out / keeps growing memory).

### Phase 2 — the fix

Add the upfront bound check (`noteFrames * 2` overflow, or an explicit sane
ceiling) and raise `ErrOverflow` before the loop.

### Phase 3 — validation

The new test green and fast; re-run the reproduction above and confirm a
prompt `ErrOverflow` instead of unbounded growth;
`scripts/man-run-examples.sh audio --run`.
