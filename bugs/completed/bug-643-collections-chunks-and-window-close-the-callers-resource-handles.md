# bug-643: `collections::chunks` and `collections::window` close the caller's resource handles

Last updated: 2026-09-27
Effort: medium
Severity: HIGH
Class: Correctness (resource closed without the program asking; double close at scope exit)

Status: Fixed
Regression Test: `tests/rt-behavior/resources/bug643-collection-helpers-keep-caller-handles-rt`;
unit tests `builder_owned_cleanup::argument_resource_tests`

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

Only handles copied into an emitted block are closed. With three file handles and
`collections::window(handles, 1, 2)` (`/tmp/p125-ex/colwinpart`), the two handles
placed into windows are closed (`77030004`), while the handle the stride skips still
reads normally:

```
windows = 2
h0 (in window 0): <raised 77030004>
h1 (skipped by stride): one
h2 (in window 1): <raised 77030004>
```

This fits the root-cause hypothesis below: the close runs per element copied into a
block. `chunks` places every element into some block, so it closes them all.

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

## Root cause (confirmed 2026-09-27 — the hypothesis above was wrong)

Reproduced on macos-aarch64 at `a0082e4ae` exactly as described. The mechanism is
not a temporary's drop and not specific to nested results. It is the **bind-from-call
adoption** in the `NirOp::Bind` lowering (`src/codegen/engine/control/builder_control.rs`,
the `is_res_marked_resource_collection` branch that calls `setup_owned_list` +
`emit_owned_list_seed_from_collection`): any `List OF RES X` bound from a call
pushes every handle in the result onto the binding scope's owned-list, and that
scope closes them when it exits. §15.6 intends this for handles a callee *produced*
and transfers on `RETURN`; a `collections` helper produces none, it only re-packages
the caller's own pointers, which already have an owner enclosing the binding.

`chunks` and `window` hit it through their own source bodies: each binds
`LET piece AS List OF T = __collections_slice(value, …)` inside the `WHILE` body
(`--nir`: `bind piece ... call #collections_slice$RES$fs$File`), so every handle
copied into a block was adopted by the loop body and closed at the end of that
iteration. `take`/`mid` looked safe only because the doc's control bound them at the
handle's own scope, where the second close is the closed flag's no-op.

The same adoption fires for the caller's own binding in any inner scope. Measured at
base, every one of these closed the caller's handle (`CLOSED`): `take` and `filter` in
an `IF`, `drop`/`mid` in a `WHILE` body, `flatten`/`get` of a `List OF List OF RES`,
`append`/`prepend`, `set`, `insert`/`removeAt`. `values` of a `Map OF K TO RES` was
already fine. The closed-pulled crash (`fs::close` "already closed", then
`Cleanup failure: 7-703-0004`) is the drain re-closing a handle the program closed.

A second seam of the same defect (Phase 3): a BORROWED element treated as an owner.
The bug-695 hoist named every resource-typed call in an element position
`RES $res_tmpN = <call>` — including `collections::get` of a `RES` element — and
the escape analysis floated that name (and a source `RES one = collections::get(…)`)
into the enclosing collection, whose owned-list drain then closed the handle the
caller owns. In `zip`'s body the float target was a `List OF Pair OF RES …`, for
which no drop exists, so every `zip` over a `List OF RES` failed to build.

Out of this doc's reach and filed separately: a USER function that hands back a
handle (or list of handles) it was passed is adopted the same way — `RES y = ident(a)`
or `LET p = same(xs)` in an inner scope closes the caller's handle (bug-755).

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

Audit results (2026-09-27, base `a0082e4ae` vs the fix):

| Helper over a `List OF RES` | base | fixed |
| --- | --- | --- |
| `chunks`, `window` (incl. stride-skipped handle) | CLOSED | open |
| `take`, `filter` in an `IF` | CLOSED | open |
| `drop`, `mid` in a `WHILE` body | CLOSED | open |
| `flatten`, `get` of a `List OF List OF RES` in an `IF` | CLOSED | open |
| `append`, `prepend`, `set`, `insert`+`removeAt` in an `IF` | CLOSED | open |
| `values` of a `Map OF K TO RES` in an `IF` | open | open |
| `take(fresh(), 1)` of a user function's fresh handles | open | open (still adopted) |
| `groupBy`/`partition`/`sortBy` with a callback over the `RES` element | does not type-check — bug-752 | same |
| `zip` of a `List OF RES` | internal build error (`owned-list element type 'Pair$RES$fs$File$Integer' has no registered close op`) | builds, handle open (Phase 3) |
| `transform` | produces its result from the callback, so it keeps adopting | — |
| a function copying `collections::get(files, i)` into a local list (element written inline, or bound `RES one = …` first) | CLOSED at the function's exit | open (Phase 3) |

## Fix

- [x] Phase 1 — RED test from the reproduction above (the handle must still be open
after `chunks` and after `window`), plus the audit probes. Commit: see Phase 2 (one
commit; the fixture is RED at `a0082e4ae`: `chunks: CLOSED`, exit 255 at the
closed-pulled probe).

- [x] Phase 2 — a binding of a `collections` re-packaging call whose arguments hold
no other call adopts nothing (`call_result_holds_only_argument_resources`,
`src/codegen/cleanup/owned/builder_owned_cleanup.rs`); a member counts as producing
only when a callback's return type reaches its return type (`transform`, `mapValues`,
`reduce`, `reduceRight`), derived from the registry signature. `chunks`/`window` man
pages and spec §15.6 updated. Commit: 0224828cd

- [x] Phase 3 — a borrowed element is never an owner: `resource_hoist` leaves a
borrowed call (`target_returns_borrowed_resource`) in place, and the escape
analysis resolves a `RES` binding initialized from one (or from a record field) to
`Local` (`initializer_is_borrowed`). Unit test
`resource_escape::tests::a_borrowed_element_never_floats`; fixture extended with
`copyEach`/`copyNamed`/`zip`. Spec architecture escape-analysis updated. Commit:
e84ab358b

STATUS: FIXED (0224828cd, e84ab358b). Deviation: the mechanism was bind-from-call adoption, not a temporary's drop, and reached every re-packaging helper in an inner scope; a second seam (a borrowed element hoisted/floated as an owner, which also broke `zip`) was fixed as Phase 3. The user-function form is bug-755 and the call-bound block leak is bug-753, both filed. Gates on the merged tip (82926917c + e703c61e3): `cargo test --release --no-fail-fast` 251 binaries, 6311 passed, 0 failed; `scripts/artifact-gate.sh target/release/mfb all` 0 diffs over 2176 goldens.
