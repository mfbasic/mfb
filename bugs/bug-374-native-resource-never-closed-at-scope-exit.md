# bug-374: a user-declared `RESOURCE … CLOSE BY` native resource is never closed at scope exit

Last updated: 2026-07-20
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: (none yet — Phase 1 adds `tests/rt-behavior/native/native-resource-scope-drop-rt`)

A resource declared with `RESOURCE T CLOSE BY nativeOp` — the form every native
binding uses — is **never closed and never reclaimed when its binding leaves
scope**. No close call is emitted, no drop-reclamation is emitted, and no
diagnostic is issued. The resource silently leaks for the life of the thread,
including the native handle itself.

The spec is explicit that this must not happen (`./mfb spec language
resource-management` §15, opening paragraph):

> Resources are closed automatically by lexical drop (§14.7) when their owning
> binding leaves scope, on every exit path: normal scope exit, RETURN,
> EXIT/CONTINUE, FAIL, PROPAGATE, an auto-propagated failure, and TRAP routing.

and the very same spec section uses a *native* resource — `RESOURCE SfFile CLOSE
BY sfClose` — as its worked example, so native resources are plainly in scope of
that guarantee.

**What makes this dangerous is that it is silent and it looks correct.** The
program runs, exits 0, and prints the right answers. Every in-tree native fixture
passes because each one closes explicitly; nothing exercises the drop path that
the spec says users may rely on. A user following §15 and omitting the explicit
close gets a program that leaks a native handle per iteration.

**The single correct behavior a fix produces:** a `RES x AS T` binding of a
user-declared resource emits, at every scope exit, the same close-then-reclaim
sequence a built-in `File` already emits — the registered `CLOSE BY` op called
with the record, followed by `emit_resource_block_reclaim`.

Found while executing plan-59-A Phase 3, whose task 2 was to "confirm scope-drop
calls the registered close op with the record". It does not call it at all; the
task's premise is false. Recorded as plan-59-A Correction C9.

References:

- `./mfb spec language resource-management` §15 — the guarantee being violated
- `src/target/shared/code/builder_control.rs:260` — the registration site whose
  `else if let Some(symbol) = self.resource_cleanup_symbol(type_)` silently
  falls through for a user resource
- `src/target/shared/code/builder_codegen_primitives.rs:1362` —
  `resource_cleanup_symbol`
- `src/builtins/mod.rs:117` — `resource_close_function`, built-ins only
- `src/ir/verify/mod.rs:3442` — `close_op_for`, which *does* know user resources;
  the table a fix needs to reach codegen
- `planning/plan-59-A-universal-resource-record.md` Corrections C9

## Failing Reproduction

```basic
IMPORT io

RESOURCE Db CLOSE BY sql::close

LINK "sqlite3" AS sql
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

' Relies on the §15 guarantee: no explicit close.
FUNC dropIt() AS Nothing
  RES db AS Db = sql::open(":memory:")
END FUNC

FUNC main() AS Integer
  MUT i AS Integer = 0
  WHILE i < 20000
    dropIt()
    i = i + 1
  WEND
  io::print("done")
  RETURN 0
END FUNC
```

Measured with `/usr/bin/time -l`, 20 000 iterations, macOS aarch64, same binary
shape, the only difference being whether the resource is closed explicitly:

| Variant | peak RSS |
| --- | --- |
| relies on scope drop (above) | **2 920 579 072 B ≈ 2.92 GB** |
| identical loop with an explicit `sql::close(db)` | **10 452 992 B ≈ 10.4 MB** |

- **Observed:** ~146 KB retained per iteration; 279× the closed variant. Exit 0,
  no diagnostic, correct output.
- **Expected:** flat retention, matching the explicit-close variant.

**Confirmed in the emitted code, not only by RSS.** For the `dropIt` above,
`--ncode` shows the function's relocations contain `sql_open` and nothing else —
no `sql_close`, no `resource_cleanup_reclaim`, no `resource_reclaim_skip`.

**Contrast case that works correctly today** — the identical shape with a
built-in `File` *does* emit the full sequence:

```basic
FUNC openOnly() AS Nothing
  RES f AS File = fs::open("/tmp/x.txt", "write")
  fs::writeAll(f, "x")
END FUNC
```

```
{ "op": "bl", "target": "_mfb_rt_fs_fs_close" },
{ "op": "b.eq", "target": "resource_cleanup_reclaim_27" },
{ "op": "b.ne", "target": "resource_reclaim_skip_28" },
```

So the cleanup machinery is present and correct; user resources simply never
reach it.

**The bug is independent of `STATE`,** which bounds it and rules out plan-59-A as
the cause: a stateful `AS RES Db STATE Info` binding emits the same zero
close/reclaim instructions. The stateful native path has been record-wrapped
since plan-53-A, i.e. both before and after plan-59-A, so this behavior predates
plan-59 entirely.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS 24.6.0 | aarch64, debug | fails ✗ |

## Root Cause

`builder_control.rs:260` registers a scope cleanup only when
`resource_cleanup_symbol(type_)` yields a symbol:

```rust
} else if let Some(symbol) = self.resource_cleanup_symbol(type_) {
    self.active_cleanups.push(ActiveCleanup::Resource(ResourceCleanup { … }));
}
```

`resource_cleanup_symbol` (`builder_codegen_primitives.rs:1362`) resolves the
close op through `crate::builtins::resource_close_function`, which is
(`src/builtins/mod.rs:117`) a direct delegation to
`resource::builtin_resource_close_function` — a lookup in `BUILTIN_RESOURCES`
(`src/builtins/resource.rs:116`), an 8-entry map of `File`, `Socket`, `Listener`,
`UdpSocket`, `TlsSocket`, `TlsListener`, `AudioInput`, `AudioOutput`.

A user-declared `RESOURCE Db CLOSE BY sql::close` is not in that map. The lookup
returns `None`, the `else if` falls through, no `ActiveCleanup::Resource` is
pushed, and scope exit therefore has nothing to emit. There is no error path —
the absence of a cleanup is indistinguishable from a scope with no resources.

The built-in contrast case is immune because `File` *is* in the map.

The information a fix needs already exists elsewhere in the compiler: the
verifier resolves user close ops via `close_op_for` (`src/ir/verify/mod.rs:3442`).
The defect is that this table never reaches the code builder.

## Goal

- A `RES` binding of a user-declared `RESOURCE T CLOSE BY op` emits, at every
  scope exit path, the registered close op called with the record pointer,
  followed by `emit_resource_block_reclaim`.
- The reproduction's peak RSS becomes flat in the iteration count, within noise
  of the explicit-close variant.

### Non-goals (must NOT change)

- **Close-exactly-once must be preserved.** A resource that is *also* closed
  explicitly must not then be closed again by scope exit. Since plan-59-B this is
  what the `closed` flag is for; until it lands, the fix must not introduce a
  double close.
- **Do not change the built-in path.** `File`/`Socket`/… already work; the fix
  extends the lookup, it does not rewrite the cleanup machinery.
- **Do not "fix" this by requiring an explicit close.** Making the omission a
  compile error would contradict §15 and break the spec's own worked example.
- **Do not fix it by removing the §15 guarantee from the spec.** The tempting
  cheap resolution — document the leak as intended — is forbidden; the guarantee
  is load-bearing for plan-59-D and -E.

## Blast Radius

Every user-declared resource in the tree relies on this and none currently
exercises it, which is why it went unseen:

- `bindings/sqlite3` (`Db`, `Stmt`) — affected; all in-tree uses close explicitly.
- `bindings/libsnd` (`SoundFile`) — affected; closes explicitly.
- All 18 `tests/rt-behavior/native/` fixtures — affected in principle; every one
  closes explicitly, so all pass today.
- `tests/rt-behavior/native/native-stateless-record-rt` (added by plan-59-A) —
  its `openOnly()` helper deliberately drops without closing, so it is currently
  **leaking by design of this bug**; it becomes a regression guard once fixed.
- Built-in `File`/`Socket`/`Listener`/`UdpSocket`/`TlsSocket`/`TlsListener`/
  `AudioInput`/`AudioOutput` — unaffected, present in `BUILTIN_RESOURCES`.

## Fix Design

Thread the user resource-closer table (the one `close_op_for` already consults)
into the code builder, and have `resource_cleanup_symbol` fall back to it when
`builtin_resource_close_function` misses.

The correctness risk concentrates in **close-exactly-once**, not in emitting the
call. Today every in-tree native program closes explicitly; once scope exit also
closes, each of those becomes a double close. Ordering therefore matters:

- Landing this **after plan-59-B** means the `closed` flag makes the second close
  a defined no-op, and the fix is safe by construction.
- Landing it **before** plan-59-B requires the static rules to guarantee no path
  both closes explicitly and drops — which is exactly the guarantee plan-59-E
  removes.

**Recommendation: sequence this after plan-59-B.** See Open Decisions.

**Rejected alternative — register the cleanup in the IR/verifier instead.** The
verifier already knows the close op but does not emit code; duplicating the
cleanup model there splits one decision across two layers.

**Rejected alternative — have `BUILTIN_RESOURCES` absorb user declarations.** It
is a `LazyLock` static describing the language's own resources; making it
per-program would turn a constant into mutable compiler state.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt-behavior/native/native-resource-scope-drop-rt`: a loop that
      drops a native resource without closing it, asserting bounded retention.
      Confirm it fails today (RSS grows ~146 KB/iteration).
- [ ] Confirm via `--ncode` that the dropping function emits no close/reclaim,
      and that the built-in contrast case does. Record both.
- [ ] Audit every in-tree user-declared resource for whether it would become a
      double close once scope exit also closes; record a verdict per site.

Acceptance: the new fixture fails on retention for the documented reason; the
audit lists a verdict per declaring site.
Commit: —

### Phase 2 — the fix

- [ ] Thread the user resource-closer table into the code builder.
- [ ] Extend `resource_cleanup_symbol` to fall back to it.
- [ ] Confirm `state_type` and `has_io_buffers` are still computed correctly for
      a user resource (`has_io_buffers` must be false — pinned by
      `only_the_builtin_file_resource_uses_io_buffers`).

Acceptance: Phase 1's fixture shows flat retention; all 18 native fixtures still
pass with no double close.
Commit: —

### Phase 3 — every exit path + full validation

- [ ] Verify the close fires on RETURN, EXIT/CONTINUE, FAIL, PROPAGATE,
      auto-propagated failure, TRAP routing, and EXIT PROGRAM, per §15's list.
- [ ] `cargo test`; `scripts/test-accept.sh target/debug/mfb <tmp> 'native*'
      'libsnd*' 'resource*'` with a hermetic `MFB_HOME`.
- [ ] Re-run the 20 000-iteration reproduction and record the new peak RSS.

Acceptance: full suite green; retention flat on every exit path; the
reproduction's RSS matches the explicit-close variant within noise.
Commit: —

## Validation Plan

- Regression test: `native-resource-scope-drop-rt`, failing on retention before
  and flat after. Retention must be *measured*, not inferred from exit code — a
  leak is invisible to exit status, which is precisely why this survived.
- Runtime proof: the 20 000-iteration loop, 2.92 GB → expected ~10 MB.
- Doc sync: none expected — the spec already states the correct behavior; this
  makes the implementation match it.
- Full suite: `cargo test`; `scripts/test-accept.sh` for `native*` `libsnd*`
  `resource*`.

## Open Decisions

- **Sequence before or after plan-59-B?** Recommend **after**. Every in-tree
  native program closes explicitly, so making scope exit also close creates a
  double close on each; plan-59-B's `closed` flag makes the second one a defined
  no-op. Landing this first would require auditing every native program for
  close-then-drop by hand, and would leave the guarantee resting on the static
  rules that plan-59-E deletes.

## Summary

The engineering risk is not emitting the call — it is close-exactly-once. Today
every native program closes explicitly precisely *because* drop does not; adding
drop-close turns all of them into double closes on the same handle. That is why
the recommended sequencing is behind plan-59-B's flag rather than in front of it.

Left untouched: the built-in resource path, the cleanup machinery itself (which
is correct and already does the right thing for `File`), and §15 — which is not
wrong and must not be edited to match the implementation.
