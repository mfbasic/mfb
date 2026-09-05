# bug-545: a `RES` alias rebind of a socket fails the build with a missing error-string data object

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: none yet — `tests/cli_thread_accept_res_bind.rs` carries the
`fs::File` shape that already passes; the `tcp`/`udp` shapes belong beside it.

Rebinding a live `tcp::Socket` or `udp::Socket` to a second `RES` name — the
aliasing shape §15.6 documents and `tests/rt_res_rebind_alias.rs` pins — fails
the build with a message about a compiler-internal symbol:

```
error: native code data relocation target '_mfb_str_error_resource_closed' is not a data object or defined symbol
```

No error code. No source location. The same program with `fs::File` builds.

## Failing Reproduction

```basic
IMPORT io
IMPORT tcp

FUNC take(RES s AS tcp::Socket) AS Integer
  RES b AS tcp::Socket = s
  RETURN 1
END FUNC

FUNC main AS Integer
  io::print("started")
  RETURN 0
END FUNC
```

- Observed (macOS aarch64, release, main at `4d56f1a1a`): the error above, exit 1.
- Expected: `Wrote executable to …/build/mfb_project.out`.

The narrowing, all measured:

| program | result |
| --- | --- |
| `RES b AS tcp::Socket = s`, no other `tcp::` call | **fails** ✗ |
| same with `udp::Socket` | **fails** ✗ |
| same with `fs::File` | builds ✓ |
| same, plus `tcp::listen("127.0.0.1", 0)` anywhere | builds ✓ |
| the `RES s AS tcp::Socket` param with no rebind | builds ✓ |

So the trigger is an **aliasing `RES` rebind of a built-in resource whose
package contributes no `_mfb_rt_fs_*` / `_mfb_rt_thread_*` runtime symbol**.

## Root Cause

The alias rebind emits the resource closed/moved guard, which references
`_mfb_str_error_resource_closed`. Two places register that string, and both are
keyed on something other than "the module emits the guard":

- `src/codegen/engine/builder/mod.rs` (~`:1067`) emits the whole standard
  error-message set when any planned runtime symbol starts with `_mfb_rt_fs_`
  or `_mfb_rt_thread_`. That is why `fs::File` works and `tcp`/`udp` do not,
  and why adding any `tcp::` call rescues it.
- `src/codegen/memory/data/data_objects.rs` (~`:280`) registers
  `ErrResourceClosed`/`ErrResourceMoved` from a **list of call names**
  (`thread.cancel`, `thread.send`, …). An alias rebind calls none of them.

This is the bug-256 class the surrounding comments already name twice ("`net::`
programs link no `_mfb_rt_fs_*`/`_mfb_rt_thread_*` symbol, so they do not get
the whole standard set for free"), and each time it was patched by adding one
more name to the list. It is a name-keyed verdict standing in for an emission
path: the guard's emitter and the string's registrar are two lists that have to
be kept in step by hand.

Adjacent, and found the same way: bug-535 was the identical shape one layer up
(a used-helper set that a codegen-emitted close did not reach), and it too was
hidden by "any other call into the package".

## Goal

- The reproduction builds and runs.
- `ErrResourceClosed`/`ErrResourceMoved` are registered from the fact that the
  module emits the closed guard, not from a list of call names that happen to
  emit it.
- No new data object appears in a program that did not already reference the
  string — the golden delta must be confined to programs that fail today.

### Non-goals (must NOT change)

- The guard itself, or the alias rebind's no-cleanup semantics (bug-375).
- **Tempting wrong fix, forbidden:** adding `tcp`/`udp` to the symbol-prefix
  test or another name to the call list. That is the patch that has already
  been applied three times; the next resource to reach the guard by a new route
  breaks again.

## Blast Radius

- `src/codegen/engine/builder/mod.rs` — the `_mfb_rt_fs_`/`_mfb_rt_thread_`
  prefix gate.
- `src/codegen/memory/data/data_objects.rs` — the call-name list.
- `src/codegen/link/thunk/link_thunk.rs` (~`:323`) — a THIRD copy of the same
  registration, added by plan-59-B for pure-`LINK` programs. A structural fix
  should be able to subsume it; check before deleting.
- Every `.ncode`/`.ncodesum` golden whose program's data-object set changes.
  Widening the gate must be shown to add nothing to a program that builds today.

## Validation Plan

- Regression test: the `tcp` and `udp` rebind programs beside the `fs::File`
  one already in `tests/cli_thread_accept_res_bind.rs`.
- Negative proof: a program with no resource at all gains no error strings.
- Full suite + artifact gate, with the golden delta shown to be confined.

## Summary

Found while fixing bug-535, which is the same defect one layer up: a
codegen-emitted resource close that the bookkeeping around it does not know
about, hidden in both cases by the fact that almost every real program makes one
more call into the package.
