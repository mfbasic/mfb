# bug-643: `collections::chunks` and `collections::window` close the caller's resource handles

Last updated: 2026-09-16
Effort: medium
Severity: HIGH
Class: Correctness (resource closed without the program asking; double close at scope exit)

Status: Open
Regression Test: none yet — see Phase 1

Calling `collections::chunks` or `collections::window` on a `List OF RES fs::File`
**closes the resources in that list**. The caller's own `RES` binding, still in
scope, then reports the handle closed on its next use, and the scope's normal
cleanup fails trying to close it a second time.

`collections::take` and `collections::mid` on the same kind of list do **not** do
this: the caller's handle stays open.

**The single correct behavior a fix produces:** passing a list of resource handles
to `chunks` or `window` never closes those resources. After the call, the caller's
handle is exactly as open as before, and the program's own cleanup closes it once.

Found by plan-125-C Phase 4 while verifying the `chunks` and `window` man pages'
"nothing you do with a block affects `value`" claims. **Filed, not fixed**, by user
instruction during a documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```basic
IMPORT io
IMPORT collections
IMPORT fs

FUNC readState(label AS String, RES h AS fs::File) AS String
  LET line = fs::readLine(h) TRAP(e)
    RECOVER "<raised " & toString(e.code) & ">"
  END TRAP
  RETURN label & ": " & line
END FUNC

SUB main()
  fs::writeText("/tmp/chunks-a.txt", "hello")
  fs::writeText("/tmp/chunks-b.txt", "hello")
  RES a AS fs::File = fs::open("/tmp/chunks-a.txt", "read")
  RES b AS fs::File = fs::open("/tmp/chunks-b.txt", "read")
  io::print(readState("a first read", a))
  io::print(readState("b first read", b))
  LET listA AS List OF RES fs::File = [a]
  LET blocks = collections::chunks(listA, 1)
  io::print(readState("a after chunks", a))
  LET listB AS List OF RES fs::File = [b]
  LET front = collections::take(listB, 1)
  io::print(readState("b after take (control)", b))
END SUB
```

Observed (macos-aarch64, `worktree-P-125`, `/tmp/p125-ex/colchunkclose`):

```
a first read: hello
b first read: hello
a after chunks: <raised 77030004>          <- ErrResourceClosed: chunks closed it
b after take (control): <raised 77020003>  <- ErrEndOfFile: still open, as expected
```

Expected: `a after chunks` raises `77020003` (end of file), like the `take` control.

`window` behaves the same (`/tmp/p125-ex/colwin3`):

```
before any collection: hello
after building [f]: <raised 77020003>   <- still open
after window: <raised 77030004>         <- closed by the window call
after get(wins, 0): <raised 77030004>
```

A program that closes the handle it pulled out of a window crashes instead
(`/tmp/p125-ex/colwin2`): `fs::close` raises "Resource handle is already closed",
uncaught, followed by `Cleanup failure: 7-703-0004` at scope exit.

## Root cause

Hypothesis, to confirm in Phase 1. `chunks` and `window` are the two helpers here
that build a **nested** result (`List OF List OF T`). Each block is built as an
inner list and then appended into the outer result. If building that inner list
takes a copy of each element and then drops a temporary, the temporary's drop runs
the element's close path. A `RES` element is closed rather than released, because
the temporary was treated as an owner of the handle. `take` and `mid` return a
single-level slice and have no such temporary, which fits the control result.

Confirm by tracing the element drops in the `chunks`/`window` bodies
(`src/codegen/builtins/collections/func_chunks.rs`, `func_window.rs`) for a
resource element type, and comparing with `take`/`mid`.

## Non-goals

- Changing what a block holds for ordinary value types (copies).
- Deciding whether a block should hold an alias of the handle or reject
  `List OF RES` at compile time. That design question is separate. The defect
  is that the *caller's* resource is closed by a call that only reads it.
- Weakening the "already closed" error or the cleanup check, which are correctly
  reporting the damage.

## Blast-radius audit

- `chunks` and `window`: confirmed.
- `take`, `mid`: confirmed not affected (control above; `/tmp/p125-ex/coltakeres`,
  `/tmp/p125-ex/colmerge`).
- Every other helper whose result nests or re-packages elements: `groupBy`
  (buckets), `partition` (two lists), `zip` (pairs), `flatten`, `transform`,
  `filter`. Phase 1 runs the same open-handle probe on each with a `RES` element
  type, where the signature allows it.

## Fix

Phase 1 — RED test from the reproduction above (the handle must still be open
after `chunks` and after `window`), plus the audit probes. Commit:

Phase 2 — stop the temporary from closing resource elements in the nested-build
path (GREEN); full suite, resource-hygiene and leak goldens. Update the `chunks`
and `window` man pages, which describe the current behavior. Commit:
