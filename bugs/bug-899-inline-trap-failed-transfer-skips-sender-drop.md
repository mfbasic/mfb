# bug-899: a `thread::transfer` that fails under an inline `TRAP` retires the sender's scope drop — the resource is never closed (fd leak, buffered writes lost)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (resource leak)

Status: Open
Regression Test: tests/rt-behavior/threads/thread-transfer-inline-trap-fail-drop-rt (proposed)

When `thread::transfer(t, f)` fails (worker completed → `ErrInterrupted`, queue full
→ `ErrTimeout`, …) and the call carries an **inline** `TRAP`, the sender keeps the
open handle (the handler can still use it — writing to it works), but the binding's
scope-exit close is gone: the file descriptor is never closed and a buffered
`fs::File`'s pending bytes are never flushed. Each such failure leaks one fd until
`open` fails with EMFILE. The same failure under a **function-level** `TRAP`, or no
transfer at all, closes the file normally.

**Correct behavior:** on a failed transfer the resource "remains the sender's"
including its cleanup: the sender's lexical scope drop closes it exactly once,
whether the failure is trapped inline (handler `RECOVER`s or `RETURN`s) or at
function level.

References:

- `src/docs/spec/threading/08_queue-semantics.md` § Copy and move semantics:
  "`thread::transfer` instead moves a resource: on success the source record is
  marked moved/closed; on failure it remains the sender's. An inline `TRAP` can
  therefore use the resource on the failure path."
- `src/codegen/resource/cleanup/builder_resource_cleanup.rs:deactivate_moved_resource_arguments`
  own comment: "Deactivation runs only on the success path (after the result-tag
  branch), so the sender keeps ownership and cleanup when the transfer fails".
- bugs/completed/bug-425 (flag set before enqueue outcome; fixed). Its regression
  `tests/rt-behavior/threads/thread-resource-transfer-fail-leak` closes the file
  explicitly with `fs::close(f)` inside the handler, so it never exercises the
  scope-drop path this bug breaks.
- Found during spec hardening batch B09 (threading queue-semantics).

## Failing Reproduction

```
mfb init t11
# t11/src/main.mfb
IMPORT io
IMPORT fs
IMPORT thread

ISOLATED FUNC ignorer(w AS ThreadWorker OF Nothing RES fs::File TO Integer, n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC once(p AS String) AS Integer
  RES f AS fs::File = fs::open(p, "r")
  RES t AS Thread OF Nothing RES fs::File TO Integer = thread::start(ignorer, 1)
  LET r AS Integer = thread::waitFor(t)
  thread::transfer(t, f) TRAP(e)        ' fails: worker already completed (77050009)
    RECOVER
  END TRAP
  RETURN 0
END FUNC

FUNC main AS Integer
  MUT i AS Integer = 0
  WHILE i < 600
    LET x AS Integer = once("/tmp/some-existing-file.txt")
    i = i + 1
  END WHILE
  io::print("ok 600")
  RETURN 0
  TRAP(e)
    io::print("failed at " & toString(i) & " " & toString(e.code) & " " & e.message)
    RETURN 1
  END TRAP
END FUNC

mfb build t11 -q && (ulimit -n 256; ./t11/build/t11.out)
```

- Observed: `failed at 253 77020001 Read operation failed.` (EMFILE), exit 1. Same
  with the handler doing `RETURN 1` instead of `RECOVER`.
- Expected: `ok 600`, exit 0.

Data-loss variant: open `"w"`, `fs::setBuffered(f, TRUE)`, `fs::writeAll(f, "P")`,
failed inline-trapped transfer, handler `RETURN`s → the file is empty afterwards
(`c=[]`). With `fs::flush(f)` in the handler it contains `R`; without the transfer
it contains `Q` — i.e. the close at scope exit never ran.

Contrast (all `ok 600` under `ulimit -n 256`, same loop):
- no transfer (thread started and waited, file just goes out of scope);
- the same failing `thread::transfer(t, f)` with a **function-level** `TRAP`;
- a user `FUNC boom(f AS fs::File)` that `FAIL`s, called as `boom(f) TRAP(e) RETURN 1`.

## Root Cause

`src/codegen/cleanup/thread/builder_thread_cleanup.rs:emit_thread_send_runtime_helper_call`:
in the `raw` (inline-`TRAP`) branch it calls
`self.deactivate_moved_resource_arguments(target, args)` **unconditionally**, before
`materialize_current_result`, with no branch on the helper's tag. For
`thread.transferResource`/`thread.emitResource` that retires the owner's cleanup
(`retire_moved_resource_cleanup`), so no later exit of the scope closes `f` — on the
failure path too. The non-raw branch calls it after `branch_eq(ok_label)`, which is
why function-level `TRAP` is correct. (The bug-425 fix made the runtime *flag* tag-
conditional for the raw path, but not this compile-time cleanup retirement.)

## Goal

- Under an inline `TRAP`, a failed transfer leaves `f` with its scope-drop cleanup;
  a successful one retires it (no double close, no `Cleanup failure`).
- Repro prints `ok 600`; buffered variant file contains `P`.

### Non-goals (must NOT change)

- Successful transfer semantics (source flagged `moved|closed`, receiver owns it).
- Do not paper over it by closing in the handler; the handler may legitimately
  keep using `f` and must not be forced to close it.

## Blast Radius

- `emit_thread_send_runtime_helper_call` raw branch — fixed by this bug.
- `src/codegen/engine/builder/builder_emit_helpers.rs` has other raw-path
  `deactivate_moved_resource_arguments` calls (grep lines ~554/679/777); each must be
  checked for the same unconditional retirement on a fallible move.
