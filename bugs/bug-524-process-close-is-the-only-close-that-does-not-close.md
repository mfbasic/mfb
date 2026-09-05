# bug-524: `process::close` is the only `close` in the language that does not close its argument

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `spikes/api-review/bug-524-process-close/` promoted to a `tests/` fixture

Six built-in packages export a `close`. Five of them close the resource and
release the OS handle:

| member | intro |
| --- | --- |
| `fs::close` | "Close an open `File` resource and release its operating-system handle" |
| `tcp::close` | "Close a TCP socket or listener and release its OS handle." |
| `tls::close` | "Close a TLS socket or listener and release its OS handle." |
| `udp::close` | "Close a UDP socket and release its OS handle." |
| `audio::close` | "Close an audio stream and release its operating-system resources; the handle cannot be used again." |
| **`process::close`** | **"Close a child's standard input, signalling end-of-input; the child keeps running."** |

`process::close(p)` closes the *child's stdin pipe*. The `Process` handle stays
open, `process::receive` still works, `process::isRunning` still returns TRUE,
and the child keeps executing.

The documentation cost of the mismatch is visible on the page itself: it needs
a paragraph headed "**`process::close` does not close the handle**", plus a
"Despite the name" sentence, plus a third qualification about which conditions
*do* make it raise `ErrResourceClosed`. And the `p` parameter row still reads:

> The child process handle whose standard input to close. **The handle stays
> open — you still close it.**

which is wrong in a second way: a `Process` has **no public close at all**. Its
registry `close_function` is the internal `__drop` op, and the comment beside it
says so — *"a `Process` is released automatically by lexical scope, not a public
`close`"*. There is nothing for "you" to close.

In a language where the scope-exit drop is the normal way a resource closes,
and where `<pkg>::close` is otherwise a reliable idiom, this is the first
support question.

The single correct behavior a fix produces: the operation that ends a child's
input is named for what it does, and `process::close` either means what every
other `close` means or does not exist.

References:

- `src/codegen/builtins/process/func_close.rs:23` — the intro
- `src/codegen/builtins/process/mod.rs:215-235` — `close_function: DROP`, and
  the comment stating there is no public close
- `src/codegen/builtins/{fs,tcp,tls,udp,audio}/func_close.rs` — the five that agree
- Spike: `spikes/api-review/bug-524-process-close/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-524-process-close
./spikes/api-review/bug-524-process-close/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
before close: isRunning = TRUE
after  close: isRunning = TRUE
after  close: receive   = apple
after  close: pid       = 48809
```

  Every call after `process::close(sorter)` succeeds. Compare `fs::close(f)`
  followed by any `fs::` call on `f`, which raises.

- Expected: either `process::close` closes the handle like its five siblings, or
  the operation is named `process::closeInput` / `process::endInput` and
  `process::close` does not exist.

Contrast cases that bound the bug:

- `tcp::close`'s own page says "`tcp::close` is the only `tcp` call that
  **closes** its argument. Every other function leaves the handle open." That
  is the invariant readers carry into `process`.
- `process::close` *is* correctly idempotent on the input pipe and does raise
  `ErrResourceClosed` on a dropped or detached handle, so the underlying
  behavior is sound. Only the name is wrong.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | the pipe-close is per-platform; the naming defect is not. Confirm the spike's output in Phase 3 |

## Root Cause

Not a code defect — a naming choice whose cost is paid in every reader's head.
`process::close` was registered as `close` (`process/func_close.rs:81`) at a
point when it was the package's only closing-ish operation, and the semantic it
actually implements — half-close the write end of the child's stdin pipe — has
no counterpart in the other five packages, all of which own a whole OS handle.

The `p` parameter description is a separate, smaller defect: it is the stock
"handle stays open — you still close it" blurb used by non-consuming resource
parameters across the tree, applied to the one resource type that has no public
close. `grep -rn "you still close it" src/codegen/builtins/` will show how many
other members share the sentence; on those it is correct.

## Goal

- The operation that signals end-of-input to a child is named for that
  (`process::closeInput` or `process::endInput`).
- `process::close` either becomes an alias that is documented as deprecated, or
  is removed — see Open Decisions.
- The `p` parameter description stops telling the caller to close a handle that
  has no public close.
- The "does not close the handle" / "Despite the name" paragraphs become
  unnecessary and are deleted.

### Non-goals (must NOT change)

- The behavior. Closing the child's stdin, its idempotence on the pipe, the
  `ErrResourceClosed` on a dropped/detached handle, and the fact that the child
  keeps running are all correct and must be preserved exactly.
- The five sibling `close` members.
- `Process`'s `close_function: DROP` and its scope-exit release.
- `process::detach`, `process::signal`, `process::waitFor`.
- **Tempting wrong fix, forbidden:** making `process::close` also close the
  handle, "for consistency". That silently breaks the documented and useful
  pattern — send input, close stdin, *then read the output* — which is the
  entire example on the page. Consistency here means renaming, not merging.

## Blast Radius

`grep -rn "process::close\|process\.close" src/ tests/ examples/ benchmark/`
in Phase 1. Known sites:

- `src/codegen/builtins/process/func_close.rs` — the member, renamed by this bug.
- `src/codegen/builtins/process/mod.rs` — the registration and the `Process`
  resource row's parameter blurb.
- `src/codegen/builtins/process/func_send.rs`, `func_send_bytes.rs` — both
  document "after `close`, further `send` raises `ErrResourceClosed`"; their
  prose follows the rename.
- Every acceptance fixture and example calling `process::close` — each is a
  source change if the old name is removed, and none if it is kept as an alias.
  This is what decides the Open Decision.
- `src/docs/spec/**` — gated by nothing; grep for `process::close`.
- The five sibling `close` members — unaffected, and the reason the rename is
  worth doing.
- Other members sharing the "you still close it" blurb — **unaffected**, the
  sentence is correct for a non-consuming parameter on a resource that *does*
  have a public close. Only `process`'s use of it is wrong.

## Fix Design

Rename to `process::closeInput`. It reads as an action on the input stream
rather than on the handle, it sorts next to `process::send`/`sendBytes` in the
function list where the reader is already looking, and it needs no explanatory
paragraph.

`endInput` is the alternative; it is marginally clearer about the *signal*
(EOF) rather than the *mechanism* (closing a pipe). `closeInput` is preferred
because the pipe really is closed and a reader debugging with `lsof` will see
that.

The compatibility question is whether `process::close` survives. The package
already supports aliases (`Parameter.aliases`, and `func_close.rs` registers
`process` as an alternate spelling for `p`), so keeping `close` as a member
alias is mechanically cheap. But an alias named `close` that does not close is
exactly the footgun this bug is about, preserved indefinitely.

Rejected: keeping the name and improving the docs. Three qualifying paragraphs
is already the improved version, and it has not made the name mean what it
says.

Rejected: adding a *second* member `process::closeInput` alongside `close`, both
live. Two names for one operation, one of which is misleading, is worse than
either alternative.

## Phases

### Phase 1 — audit (no behavior change)

- [ ] Land `spikes/api-review/bug-524-process-close/` (done).
- [ ] `grep -rn "process::close" src/ tests/ examples/ benchmark/ src/docs/` —
      list every call site and doc mention. The count decides the Open Decision.
- [ ] `grep -rn "you still close it" src/codegen/builtins/` — confirm the blurb
      is correct everywhere except `process`.
- [ ] Add a fixture asserting the current behavior (`isRunning` TRUE and
      `receive` working after the call), so the rename cannot quietly change it.

Acceptance: the call-site list is complete; the behavior fixture passes.
Commit: —

### Phase 2 — the rename

- [ ] Rename the member to `closeInput`; rewrite its intro and description
      without the "despite the name" apologetics.
- [ ] Fix the `p` parameter description: the handle is not closed by this call
      and has no public close; it is released when its binding ends.
- [ ] Update `func_send.rs`/`func_send_bytes.rs` prose.
- [ ] Update every call site from Phase 1.
- [ ] Apply the Open Decision on whether `close` survives as an alias.

Acceptance: the Phase 1 behavior fixture still passes under the new name; no
`process::close` remains except a deliberately-kept alias.
Commit: —

### Phase 3 — regenerate + validation

- [ ] Regenerate `.ncodesum` goldens the rename shifts (run the regen scripts
      under **bash**).
- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh process --run`.
- [ ] Re-run the spike on Linux and Windows.

Acceptance: full suite green; golden deltas are only the symbol rename.
Commit: —

## Validation Plan

- Regression test: the Phase 1 fixture — after the call, `isRunning` is TRUE,
  `receive` returns the child's output, and `send` raises `ErrResourceClosed`.
- Runtime proof: `spikes/api-review/bug-524-process-close/` under the new name.
- Doc sync: `func_close.rs`, `func_send.rs`, `func_send_bytes.rs`,
  `process/mod.rs`, `src/docs/spec/**`.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

**Decided (2026-09-04): rename only. `process::close` is NOT changed to close
the handle, and no `close` that closes the handle is added.**

The question asked was whether the fix renames the member, changes `close` to
actually close, or both. It renames, for two reasons:

- Making `close` close the handle would break the pattern the page's own
  example is built on — send the input, close stdin, *then* read the child's
  output. A `close` that ends the handle makes that sequence impossible, and it
  is the single most common reason to call the member.
- There is nothing for it to become. A `Process` has no public close by
  design: its `close_function` is the internal `__drop` op and the resource is
  released by lexical scope (`process/mod.rs:215-218`). Adding a public
  handle-closing `close` would be new surface duplicating the scope drop, not a
  correction.

Note there is **no `closeOutput` to pair with `closeInput`.** The child's
stdout is drained by `process::receive` and no member closes it; the naming
should not imply a symmetry the package does not have.

Still open:

- Does `process::close` survive as a deprecated alias? **Decide from the
  Phase 1 call-site count.** If it is small and all in-tree, remove the name
  outright — the whole point is that `close` should mean close. If there is
  external exposure, keep it as an alias with a deprecation note and a removal
  target, and accept that the footgun persists until then.

## Summary

The behavior is correct and stays untouched; the risk is entirely in the rename
sweep and the golden regeneration it shifts. The one judgement call — whether
`close` survives as an alias — should be made from the Phase 1 count rather
than in advance, because keeping it preserves exactly the confusion this bug
exists to remove.
