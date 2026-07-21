# bug-371: an inline `TRAP` on a `LINK` call returning `RES ... STATE ...` bypasses `ERROR_ON`, then reads an uninitialized `STATE` record

Last updated: 2026-07-20
Effort: medium (2h–4h) — the error gate and the `STATE` marshaling disagree about ordering on the inline-TRAP path
Severity: **HIGH** (silent gate bypass leading to a garbage-driven allocation, and to SIGSEGV under a small variation)
Class: Correctness (native binding error path — memory safety)

Status: **FIXED** 2026-07-20.
Regression Test: `tests/rt-behavior/native/native-link-inline-trap-rt` (new) —
a failing `SUCCESS_ON` gate reached through an inline `TRAP` directly, through
one user frame, and through a function-level `TRAP`, each asserting the trapped
code is `77030008`; plus the annotated `RES … STATE …` form from bug-372.
`tests/syntax/native/native-res-state-inline-trap-valid` (new) covers the
compile-time half.

## What it actually was

The report's title and analysis are **wrong about the trigger**, though its
symptoms are exact. Reproduced against `sqlite3` with no `RES`, no `STATE` and a
`Nothing` return — `sql::config(9999) TRAP(e) … END TRAP` on a plain `LINK`
`FUNC` — is enough. `ERROR_ON` was never bypassed and the gate always fired:
the thunk correctly returned `x0 = ERR`, `x1 = 77030008`, `x2 = <static
message>`. The `STATE` record was never read; there was no `STATE`.

Two independent defects, in the *consumption* of that error:

**1. The thunk never set `x3` (`RESULT_ERROR_SOURCE_REGISTER`).** `x3` is also
argument register 3, so on every failure it still held whatever the marshaling
had staged. The fallible-call ABI requires `x3 == 0` for an origin-less error
(`mfb spec memory fallible-call-abi`), and a caller that *consumes* a loose
error — which is exactly what an inline `TRAP` does, and what the ordinary
propagate path does not — builds an `ErrorLoc` from `x3`. Sizing a record from a
garbage pointer produced either a nonsense allocation size (`7-701-0001
Allocation failed`) or a walk into unmapped memory (`EXC_BAD_ACCESS` in a
word-store fill loop — the report's stack). The `RES`/`STATE` variant reached the
second only because it happened to put a different value in `x3`.
Fixed in `lower_link_thunk` / `lower_link_initializer`
(`src/target/shared/code/link_thunk.rs`): `x3 = 0` at the shared `done` label, so
no future epilogue can forget it.

**2. The `Error` record size walk ignored the absent-source sentinel.** With (1)
fixed the error was caught correctly and then crashed on the way *out* of the
handler. `Error.source` absent is an offset-0 sentinel, and an origin-less
`Error` is `{code, message}` with nothing written past the message — but
`emit_record_block_size_to_slot` added an inlined `ErrorLoc` at the running
offset unconditionally, sizing a phantom record out of whatever followed the
block. Freeing the trap-local error then handed `arena_free` a garbage size and
corrupted the free list. Latent until now because no other producer emitted a
null-source `Error` that anything freed.
Fixed in `src/target/shared/code/builder_collection_layout.rs`: the walk reads
each inlined field's own offset word and skips the field when it is 0.

Also fixed in passing: `mfb build --nir/--nplan/--nobj/--ncode` never assembled
the `LINK` locator table, so dumping intermediate output for any project with its
own `LINK` block failed with `NATIVE_LIBRARY_NO_MATCH` — i.e. it was unusable in
precisely the case it was needed to diagnose this (`src/cli/build.rs`).

Attaching a postfix inline `TRAP` to a call whose target is a `LINK` `FUNC`
returning `RES <Resource> STATE <Record>` **loses the `ERROR_ON` gate**. The
failing native call is treated as having succeeded, the `STATE` record is read
from a buffer the native call never wrote, and the resulting garbage field drives
a downstream allocation.

The same call, same input, same binary — the only difference is the inline `TRAP`:

| call site | result |
|---|---|
| `LET s = libsnd::loadSound(missing)` | `7-703-0008` — `ERROR_ON` fires, correct |
| `LET s = libsnd::loadSound(missing) TRAP(e) … END TRAP` | `7-701-0001 Allocation failed` — gate bypassed, **and the `TRAP` does not catch it** |

Note the second row fails twice over: the gate does not fire, *and* the error
that does surface is not delivered to the handler that was written to catch it.
The program dies at exit 255 with a top-level error despite having a `TRAP`.

## Ground truth from libsndfile

The native call genuinely fails and genuinely returns NULL, so `ERROR_ON file =
NOTHING` has everything it needs. Confirmed against libsndfile directly (C, same
library, same input):

```
handle=0x0 err=1 msg=Format not recognised.
frames=0 ch=0 rate=0
```

`sf_open` returns `NULL`. Critically, **libsndfile does not write the `SF_INFO`
out-parameter when the open fails** — so whatever the thunk's `SfFileInfo` buffer
held before the call is what `BIND STATE file = info` marshals into `FileInfo`.
Zeroed in the C probe above only because the probe `memset`s it first.

## Failing Reproduction

Against `bindings/libsnd` at `f2f583807` with no modifications:

```basic
IMPORT io
IMPORT libsnd

FUNC main() AS Integer
  io::print("main entered")
  LET s = libsnd::loadSound("target/nope.wav") TRAP(e)
    io::print("trapped code=" & toString(e.code) & " msg=" & e.message)
    RETURN 0
  END TRAP
  io::print("loaded len=" & toString(len(s.pcm)))
  RETURN 0
END FUNC
```

```
main entered
Error: 7-701-0001
Allocation failed.
[exit 255]
```

Expected: `trapped code=77030008 …`. The handler never runs.

Delete the ` TRAP(e) … END TRAP` and the same program correctly reports
`7-703-0008`.

## Why `ErrOutOfMemory` is the symptom

`loadSound` computes `items = info.frames * info.channels` and
`bytes = items * 2`, guarded by `IF bytes > MAX_LOAD_BYTES`. With `info` read from
an unwritten buffer, `frames` is arbitrary: the product can be large enough to
exhaust the allocator, or can overflow `Integer` to a negative value that sails
under the cap check and is then used as an allocation size.

## It degrades to SIGSEGV

The `ErrOutOfMemory` is luck, not a floor. Interposing one ordinary wrapper
function between `loadSound` and the `LINK` call —

```basic
FUNC openSound(path AS String) AS RES SoundFile STATE FileInfo
  RETURN sndLink::openFile(path)
  TRAP(e)
    FAIL sndError(sndLink::lastErrNum())
  END TRAP
END FUNC
```

— turns the same input into `EXC_BAD_ACCESS`, faulting in a word-store fill loop
walking off the end of the buffer:

```
stop reason = EXC_BAD_ACCESS (code=1, address=0x10007c000)
->  0x10001005c: str    x14, [x8]
    0x100010060: add    x8, x8, #0x8
    0x100010064: sub    x9, x9, #0x1
    0x100010068: cmp    x9, #0x0
```

A garbage frame count reaching an allocation size is a memory-safety bug whose
observable form depends on the surrounding code, which is why this is filed HIGH
rather than as a wrong-error-code defect.

## Suggested Fix

Order the two operations on the inline-TRAP path the way the direct path already
orders them: **evaluate `ERROR_ON` / `SUCCESS_ON` before any `BIND STATE`
marshaling, and route a failed gate into the trap handler** rather than falling
through into the success path.

Worth checking whether the gate is being emitted at all on this path or is being
emitted after the state marshal — the fact that the handler never runs suggests
the failure is not reaching the trap's error branch at all, i.e. the call is
lowered as infallible and the `Result` the handler switches on is always `Ok`.

See also **bug-372**, a compile-time defect in the same `RES` + `STATE` + inline
`TRAP` combination; they are likely the same underspecified lowering path.

## Impact

Every native binding that returns a stateful resource — the `plan-53`/`plan-54`
`LINK RES … STATE …` shape — is affected the moment a caller writes the ordinary
defensive thing and wraps the call in a `TRAP`. `libsnd::loadSound` is the
shipped instance: a caller who traps a bad path gets an uncatchable
`ErrOutOfMemory` instead of their handler, and a file the library rejects is
processed with an uninitialized geometry record.
