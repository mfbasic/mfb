# bug-374: a user-declared `RESOURCE … CLOSE BY` native resource is never closed at scope exit

Last updated: 2026-07-21
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Fixed
Regression Test: `tests/native_resource_scope_drop.rs` (5 codegen assertions, one
per §15 exit path, all failing before the fix) and
`tests/rt-behavior/native/native-resource-scope-drop-rt` (runtime, 2.34 GB → 8.5 MB)

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

- [x] Add `tests/rt-behavior/native/native-resource-scope-drop-rt`: a loop that
      drops a native resource without closing it, asserting bounded retention.
      Confirm it fails today (RSS grows ~146 KB/iteration).
- [x] Confirm via `--ncode` that the dropping function emits no close/reclaim,
      and that the built-in contrast case does. Record both.
- [x] Audit every in-tree user-declared resource for whether it would become a
      double close once scope exit also closes; record a verdict per site.

Acceptance: met. The fixture peaks at **2.34 GB** before the fix and **8.5 MB**
after (273×). Pre-fix `--ncode` for the dropping function contains
`_mfb_linker_sql_open` and no close/reclaim; post-fix it contains
`_mfb_linker_sql_close`, `resource_cleanup_reclaim`, and `resource_reclaim_skip`,
matching the built-in `File` contrast case. Audit verdict: **every** in-tree site
becomes a double close, and every one is safe — see "Close-exactly-once" below.

### Phase 2 — the fix

- [x] Thread the user resource-closer table into the code builder.
- [x] Extend `resource_cleanup_symbol` to fall back to it.
- [x] Confirm `state_type` and `has_io_buffers` are still computed correctly for
      a user resource (`has_io_buffers` must be false — pinned by
      `only_the_builtin_file_resource_uses_io_buffers`).

Carried on `TypeModel` (`resource_closers`) rather than as a fourth `CodeBuilder`
parameter: `TypeModel` already crosses this exact boundary for the same reason
(`resource_names`, added by bug-372), and it is the only layer that sees both the
`RESOURCE` declarations and `link_functions`.

Acceptance: met. Both fixtures flat; 110 native/libsnd/resource acceptance tests
pass with no double close.

### Phase 3 — every exit path + full validation

- [x] Verify the close fires on RETURN, EXIT/CONTINUE, FAIL, PROPAGATE,
      auto-propagated failure, TRAP routing, and EXIT PROGRAM, per §15's list.
- [x] `cargo test`; `scripts/test-accept.sh target/debug/mfb <tmp> 'native*'
      'libsnd*' 'resource*'` with a hermetic `MFB_HOME`.
- [x] Re-run the 20 000-iteration reproduction and record the new peak RSS.

Acceptance: met. A program driving normal exit, RETURN, EXIT, CONTINUE, and
FAIL/TRAP together over 5 000 iterations peaks at **5.84 GB** before the fix and
**18.9 MB** after (308×). The reproduction itself: **2.92 GB → 11.4 MB**, within
noise of the explicit-close variant's 10.2 MB.

Measured retention, 20 000 iterations each, macOS aarch64 debug:

| Variant | before | after |
| --- | --- | --- |
| module-declared `RESOURCE Db CLOSE BY sql::close` | 2.92 GB | 11.4 MB |
| imported binding (`IMPORT sqlite3`, re-exported closer) | 2.92 GB | 11.0 MB |
| stateful `AS RES Db STATE DbInfo` (String-inlining payload) | 2.93 GB | 15.9 MB |
| all §15 exit paths together (5 000 iters × 5 paths) | 5.84 GB | 18.9 MB |
| return-then-use, return-then-drop (40 000 iters) | — | 22.9 MB |

The stateful row confirms the report's "independent of `STATE`" claim from the
other direction: `state_type_name` splits the `STATE` clause off the type string
and is not builtin-gated, and the cleanup lookup keys off `base_resource_name`,
so the STATE payload is reclaimed with the record.

## What the fix turned up that the design did not anticipate

**1. The imported-binding path needs a different name, and nearly shipped broken.**
The design says "thread the user resource-closer table into the code builder",
which covers a project declaring its own `RESOURCE`. It does not cover a program
that *imports* a binding: a decoded package carries no `native_resources` at all
(`ir/binary.rs` drops them by contract), so the close op has to come from the
package's `RESOURCE_TABLE` instead. Worse, the name there is package-internal.
`bindings/sqlite3` re-exports its closer (`EXPORT FUNC close AS sqliteLink::close`),
so `Db`'s serialized `close_function` is the bare alias `close`, while the
importing module routes it as `sqlite3.close`.

A first cut resolved close ops by matching the dotted `alias.func` against
`link_functions`. That fixed the module-declared case and left every *imported*
resource still leaking — `IMPORT sqlite3` + drop still peaked at 2.92 GB. The
landed fix stores the declared name and resolves it through `function_symbols`,
the same table an explicit `sql::close(db)` call goes through, with the package
branch qualifying by package name exactly as `ir/package.rs` qualifies the
routing alias.

**2. Close-exactly-once resolves the opposite way from the design's expectation,
and the "redundant" close must NOT be removed.** The same builtin-only
`resource_close_function` lookup that caused this bug appears at two more sites,
both of which were inert only because no cleanup existed to retire:

- `deactivate_moved_resource_arguments` (`builder_codegen_primitives.rs:1512`) —
  an explicit close does not retire the binding's cleanup for a user resource.
- the `RETURN` ownership transfer (`builder_codegen_primitives.rs:2364`).

Both are now live, and the tempting follow-up is to extend the lookup there too
so a native resource closes exactly once, as a built-in `File` does. **That would
introduce a leak.** A `LINK` close thunk only sets `RESOURCE_CLOSED_BIT`; unlike
the `fs.close` runtime helper it never frees the 80-byte record (verified in the
emitted thunk: no `arena_free`, no reclaim). The scope-exit cleanup's
`emit_resource_block_reclaim` is therefore the *only* thing that reclaims the
record — on the explicit-close path too. Retiring the cleanup would drop the
reclaim with it.

So the second close is load-bearing, not waste, and this is also why a native
function emits one more close site than the built-in equivalent. The second call
is harmless because plan-59-B's `closed` flag makes it a defined
`ERR_RESOURCE_CLOSED` no-op that `emit_resource_cleanup_call` already treats as
benign on the drop path. Pinned by
`explicitly_closed_native_resource_is_still_reclaimed`.

The `RETURN` site needs no change either: plan-59-D's `escaping_value_slot`
identity skip already branches past both close and reclaim when the resource
being dropped is the one escaping. Verified — a function returning a `RES Db`
that the caller then uses and closes runs clean over 40 000 iterations at 22.9 MB
flat, with no double free and no use-after-close.

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

  **Resolved:** landed after plan-59-B, as recommended. The `closed` flag was
  already in place (the thunk sets `RESOURCE_CLOSED_BIT` and carries the
  closed/moved guard), so the fix was safe by construction and the 110
  native/libsnd/resource fixtures — every one of which closes explicitly — passed
  unchanged.

## Summary

The engineering risk is not emitting the call — it is close-exactly-once. Today
every native program closes explicitly precisely *because* drop does not; adding
drop-close turns all of them into double closes on the same handle. That is why
the recommended sequencing is behind plan-59-B's flag rather than in front of it.

Left untouched: the built-in resource path, the cleanup machinery itself (which
is correct and already does the right thing for `File`), and §15 — which is not
wrong and must not be edited to match the implementation.
