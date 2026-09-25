# bug-695: a resource placed into a container without a `RES` binding is never closed

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness (resource leak: OS handles, and for a `Thread` a worker never cancelled)

Status: Fixed
Regression Test: tests/runtime/rt_scope_drop_leaks.rs
(`a_resource_temporary_in_a_container_is_closed`)

## STATUS: FIXED (see the commit that archives this document)

A new IR-lowering pass, `ir/resource_hoist.rs`, names each such temporary before
the escape analysis runs: a producer call in an element position whose type is a
resource becomes `RES $res_tmpN AS T = <call>` just before its statement, and the
element becomes `$res_tmpN`, so it floats exactly like a named binding. The pass
never reorders evaluation (it stops hoisting at the first earlier expression with
an effect it did not hoist) and leaves anything it cannot type in place.

Verified: the reproduction prints `opened 200` under `ulimit -n 64`; the record
form likewise, with the `--debug` report flat; `H[t := thread::start(work, 5)]`
ends with `thread.table_live 0`. The regression test fails without the pass
(`b695_list_append: main-arena live_bytes grew 4800 B`) and passes with it. An
IR sweep over all 958 fixtures with `.ir` goldens changed exactly two, both
this shape (`syntax/resources/ownership-collection-resource-valid`,
`rt-behavior/threads/bug692-record-thread-copy`); `artifact-gate all` → 0 diffs.
Probed for the double-close direction: a returned list literal of temporaries, a
returned record, a map value, and a non-resource call ahead of a producer (not
hoisted, order preserved) — every run with `double_free_skips 0`.

Spec §15.6 (`15_resource-management.md`): "A resource element placed into a
collection may be any pointer to the resource — a named `RES` binding or a
temporary alike. The collection holds pointers to resources owned by the
outermost scope that touches them." The verifier accepts the temporary
(`ir/verify/values.rs`, the plan-59-E comment: "a temporary is as admissible as a
binding; the resource is still closed exactly once, by that scope"). Codegen
never closes it: the escape analysis (`ir/resource_escape.rs`) floats only NAMED
`RES` bindings into a container's owned-list, so a producer call written straight
into the container has no close obligation anywhere.

It is not collection-specific: a record literal's `RES` field fed by a producer
call leaks the same way.

## Failing Reproduction

```basic
IMPORT collections
IMPORT io
IMPORT fs

FUNC main AS Integer
  MUT i AS Integer = 0
  WHILE i < 200
    MUT pool AS List OF RES fs::File = []
    pool = collections::append(pool, fs::openFile("project.json"))
    i = i + 1
  END WHILE
  io::print("opened " & toString(i))
  RETURN 0
END FUNC
```

- Observed under `ulimit -n 64`: `Error: 7-702-0002 Write or flush operation
  failed.`, exit 255 — the descriptors ran out.
- The record form, `LET h AS H = H[f := fs::openFile(p)]` with
  `TYPE H / f AS RES fs::File / END TYPE`, fails identically; a `--debug` build of
  it reports `arena.0.live_bytes 16480` for a single record.
- Control: binding first — `RES g AS fs::File = fs::openFile(p)` then
  `H[f := g]` (or `append(pool, g)`) — prints `opened 200`, exit 0, and the
  `--debug` report is flat.
- With a `Thread` (plan-156-D makes it storable): `LET h AS H =
  H[t := thread::start(work, 5)]` leaves `thread.table_live 1` and the 96-byte
  view live — the worker is never cancelled and its control block never
  released. `pool = collections::append(pool, thread::start(f, i))`, the natural
  way to fill a worker pool, has the same defect.

Measured 2026-09-25 on the plan-156 branch (worktree-P-156); the `fs::File`
programs involve no plan-156 code.

## Root Cause

`ir/resource_escape.rs:Analyzer::scan_collection_expr` collects `RES` binding
NAMES inserted into a collection or record; a call expression in that position
contributes nothing, so no `ResOwner::Float` exists for it, no owned-list push
is emitted, and no scope-drop cleanup is registered for the temporary.

## Goal

A resource temporary in an element, map-value or `RES`-field position is owned
exactly as a named `RES` binding in that position would be: closed once, by the
outermost scope that touches it. Direction: hoist such a temporary into a
synthesized `RES` binding before the escape analysis runs, so the existing float
machinery owns it.
