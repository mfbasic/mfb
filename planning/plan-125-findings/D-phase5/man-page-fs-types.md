### 1. "A second name … is an alias" omits that `RES g = f` retires the first name
UNIT:      man-page:fs/types
CLAIM:     "A second name for one is an alias, not a copy, and it closes itself when its binding's scope ends."
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase5/man-page-fs-types/p2_alias.bas` (built via `mfb init` + `mfb build`):
```
RES f AS fs::File = fs::open("...p2.txt", "write")
RES g AS fs::File = f
fs::writeAll(g, "via g")
fs::writeAll(f, "via f")
```
rejected at compile time: `error[2-203-0055 TYPE_USE_AFTER_MOVE]: binding is used after move — Binding \`f\` was moved and cannot be used again.` The alias sense the sentence describes does hold for a parameter and a record copy (probe `p3_record.bas`: `LET h2 AS Holder = h` then `fs::close(h.handle)` makes `fs::writeAll(f, ...)` raise `Resource handle is already closed.`; file contents `record`). But the most literal way to make "a second name" is a `RES` rebinding, and that is the one case where the first name stops working. The linked `mfb man variable` page also never shows `RES b = a` (`mfb man variable | grep -n "RES b"` finds only the two-file list example). This sentence is renderer text (`src/cli/man.rs`, the Resources preamble), so it appears on every `types` page; fix it once there.
SUGGESTED: "Each of these is a RES handle. Passing one to a call, or copying a record that holds one, gives a second name for the same open handle — an alias, not a copy — and closing through either name closes it. Binding it to a new RES name (RES g = f) instead hands the handle to g; f cannot be used again. A handle closes itself when the binding that holds it leaves scope. …"

### 2. Automatic close cannot report a failure — a write-mode File should say so
UNIT:      man-page:fs/types
CLAIM:     "An opaque handle to an open file, closed automatically when its binding goes out of scope."
VERDICT:   incomplete
EVIDENCE:  The automatic close is real: probe `p1_scope.bas` opens with `"write"`, turns on `fs::setBuffered(f, TRUE)`, writes without closing, and `fs::readText` after the scope prints `hello from scope`. But `fs::File` is registered with `close_may_fail: true` (`src/codegen/builtins/fs/mod.rs:RegistryResource` for `FILE_TYPE`), and `mfb man fs close` states that a scope-end close drains the buffer and that "A failed host close raises ErrCloseFailed, and a failed drain of the buffer raises ErrWriteFailed" — only from the explicit call. The spec (`src/docs/spec/language/15_resource-management.md`, "A close that runs as part of an implicit lexical drop cannot inject an error into program flow") confirms a scope-end close failure is never raised. So for a file opened for writing, relying on the automatic close means a failed flush is silent. I could not provoke a failing host close in the sandbox (UNVERIFIED that the data loss is silently observable end to end); the claim rests on the `fs close` page and the spec. The `fs::File` description is the one place a developer learns what the handle is, and it currently reads as if the automatic close is always equivalent to `fs::close`.
SUGGESTED: "An opaque handle to an open file. It is closed automatically when its binding goes out of scope; call fs::close yourself when you need to see a close or flush failure, because a close that happens at scope end cannot raise one."

The other sentences on the page held. "A handle may be a field of a record and an element of a collection — written List OF RES <Type>, never a bare List OF <Type>": probe `p3_record.bas` wrote through a `RES fs::File` record field and held a `List OF RES fs::File` (`holding 1`, files contained `record` and `list`); probe `p4_bare.bas` with `List OF fs::File` was rejected with `TYPE_RESOURCE_REQUIRES_RES`. "May be handed to another thread with thread::transfer": probe `p5_transfer.bas` with a companion `workers` package transferred a write-mode File to a worker, which wrote to it; the parent printed `worker returned 7` and then `from the worker` from `fs::readText`. `sendable: true` in the descriptor matches. The page has no example section and no parameters or errors table, so nothing else was checkable; no banned-vocabulary words appear on the rendered page.
