# bug-546: `thread::accept` of a user-declared `THREAD_SENDABLE` resource fails native lowering

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: **FIXED** (2026-09-05). Two of this document's own conclusions were
wrong and are corrected below under "The root cause is one layer up".
Regression Test: `tests/rt-behavior/native/native-resource-thread-accept-rt`
(runtime), plus three unit pins in
`src/codegen/collection/layout/builder_collection_layout.rs`
(`res_field_record_layout_tests`).

`RESOURCE … THREAD_SENDABLE` is the opt-in that lets a user-declared native
resource cross a thread boundary (`mfb man thread`: "a resource a program
declares may cross when it is declared `THREAD_SENDABLE`"). Taking one off the
receiving end fails the build:

```
error: native inlined field size not available for type 'Db' while lowering bind d AS Db
```

No error code, no file, no line — the same unlocated shape as bug-479, which is
the other known caller of this message.

## Failing Reproduction

`project.json` needs a `libraries.sqlite3` entry (the `LINK` block below is only
scaffolding for a declared resource; nothing is executed).

```basic
IMPORT io
IMPORT thread

RESOURCE Db CLOSE BY sql::close THREAD_SENDABLE

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

ISOLATED FUNC worker(t AS ThreadWorker OF RES Db TO Integer, n AS Integer) AS Integer
  RES d AS Db = thread::accept(t, 1000)
  RETURN 1
END FUNC

FUNC main AS Integer
  LET a AS Thread OF RES Db TO Integer = thread::start(worker, 0)
  io::print("started")
  RETURN 0
END FUNC
```

- Observed (macOS aarch64, release, main at `4d56f1a1a`): the error above, exit 1.
- Expected: a build, and `started` on stdout.

The same shape with any BUILT-IN sendable resource (`tcp::Socket`,
`tcp::Listener`, `tls::Socket`, `tls::Listener`, `udp::Socket`, `fs::File`)
builds since bug-535 — so this is specific to a user-declared resource.

## Root Cause (CONFIRMED 2026-09-05)

Reproduced on `c61133282` with a `LINK "c"` scaffold (`fopen`/`fclose`):

    error: native inlined field size not available for type 'Db' while lowering bind d AS Db

The classification predicates are **built-in only**:

    is_resource_type(t)                 -> resource::is_builtin_resource_type(t)
    is_thread_sendable_resource_type(t) -> resource::is_builtin_sendable_resource_type(t)
    (src/codegen/builtins/mod.rs:141-147)

`copy_value_to_current_arena`'s dispatch
(`src/codegen/memory/arena/builder_arena_transfer.rs:415-434`) has three arms —
collection, *sendable* resource (deep copy), *any* resource (carry the pointer,
move-only) — and a user-declared `RESOURCE Db THREAD_SENDABLE` matches **none**
of them, because it is not a builtin. It therefore falls through to the flat /
record path, which asks for an inlined field size for a type that is really an
8-byte handle. That is the same shape the registry already documents for
`audio.AudioOutput` (`registry/mod.rs:2242`): "a bare resource spelling is
invisible to the resource classification", so a handle gets flat-copied.

The module's own resources ARE known to codegen — `TypeModel::resource_names`
(`engine/builder/mod.rs:751`), keyed by the bare declared type, which is what
bug-535's fix used. So the missing ingredient is not information, it is that the
two predicates consult only the builtin table.

## The design question a fix must answer first (do not skip to the predicate)

Widening the predicates is a two-line change and is **not obviously safe**, so it
must not be made without answering this:

- The **sendable** arm deep-copies the record via `copy_resource_to_current_arena`.
  Its sibling arm's comment warns a deep clone "would both duplicate the OS handle
  and assume the fixed `File` layout, which audio's larger `AudioHandle` does not
  share". A user-declared resource's record comes from its `LINK` block; whether
  it matches the canonical layout the copier assumes is unverified.
- The **pointer** arm is move-only and layout-agnostic, but a cross-thread accept
  that merely carries a pointer publishes a reference into the *sender's* arena —
  and arena state is per-thread. That is exactly bug-498's class (fixed there by
  copying in the sender's arena), so choosing this arm without checking would
  risk reintroducing a use-after-free rather than fixing a build error.

So the two candidate one-line fixes lead to a duplicated OS handle and a
cross-arena dangle respectively, unless the record layout question is settled
first. The bug's own fallback — reject `THREAD_SENDABLE` on a user-declared
resource with a rule code and a location — remains available and is strictly
better than today's unlocated lowering error, but it removes a documented
capability (`mfb man thread`), so it is a product decision rather than a
mechanical fix.

## The record-layout question is ANSWERED (2026-09-05) — it is the canonical record

`src/codegen/link/thunk/link_thunk.rs:1555-1585` (`if function.return_resource`):
a `LINK` function returning `AS RES T` does **not** hand back the bare native
handle. It `arena_alloc`s `RESOURCE_RECORD_SIZE` and fills the canonical plan-80
record — `{tag@0 = RESOURCE_TAG_NATIVE, FD@8 = the handle, CLOSED@16 = 0,
STATE@24, buffer words zeroed}` — and its own comment states the intent:

> the exact shape a built-in `fs.File STATE S` uses, so `.state`,
> drop-reclamation (plan-52-B), and the closed guard all work unchanged.

So a user-declared resource's record **is** the layout `copy_resource_to_current_arena`
already handles. That removes both hazards recorded above:

- **No duplicated OS handle / wrong layout.** The deep copy is over the same
  canonical record a builtin uses; `emit_copy_resource_live_slots` carries the
  declared tail generically and emits nothing when a resource declares no slots.
- **No cross-arena dangle.** The sendable arm copies *into the current arena*,
  which is exactly what makes it the safe arm and why the pointer arm was the
  risky one.

**So the correct arm is the sendable one**, and the fix is to make the dispatch
reach it for a user-declared resource rather than to widen the pointer arm.

## What still has to be plumbed (the real remaining work)

`TypeModel::resource_names` records only the type NAME
(`engine/validation/validation.rs:304-306` — `"resource" => resource_names.insert(...)`).
It does **not** record whether the declaration carried `THREAD_SENDABLE`, and
codegen has no other sendability signal for a user type: the only `sendable`
mention in the builder is a comment. Routing every user-declared resource to the
sendable arm would therefore also send resources the author did NOT mark, which
is the opposite error — the frontend forbids transferring a non-sendable
resource, and codegen must not quietly permit it.

So the change is:

1. carry `THREAD_SENDABLE` from the `RESOURCE` declaration into the NIR type
   entry and into `TypeModel` beside `resource_names` (a sendable subset);
2. consult it in `is_thread_sendable_resource_type`'s caller (or a new builder
   predicate that ORs builtin ∪ declared-sendable), leaving
   `is_builtin_resource_type` alone so no builtin behaviour moves;
3. keep the non-sendable user resource on the pointer arm, matching today's
   frontend rule.

With the layout question settled this is a scoped change rather than an open
design problem, but it is still plumbing through NIR, so the "medium (1h-2h)"
estimate remains optimistic. The runtime half of the Goal — the accepted handle
is usable and closed exactly once — must be proven with an `rt_*` test, not just
a build.

## The root cause is one layer up (MEASURED 2026-09-05, and it corrects this doc)

The failure was localized by marking the two sites that can emit the message and
rebuilding: it is `builder_collection_layout.rs`
(`emit_inlined_block_size_from_ptr_slot`), reached from `copy_flat_block`
(`:407`) — **not** the record marshaller. `copy_flat_block` is reachable from
`emit_thread_copy_real` on exactly one arm, the first one:

    other if self.type_is_memcpy_copyable(other) => self.copy_flat_block(...)

So `Db` never reached the three-arm dispatch this document analysed. It was
classified **flat** before the resource arms were consulted, and the section
above ("`copy_value_to_current_arena`'s dispatch … a user-declared `RESOURCE Db
THREAD_SENDABLE` matches none of them") named the wrong miss.

The real miss is in `flatness_walk` (`builder_collection_layout.rs`), the single
walk behind both `type_is_memcpy_copyable` and `type_is_arena_transferable`. Its
resource arm asked `crate::codegen::builtins::is_resource_type`, which answers
for the **built-in registry only**. A user-declared `Db` matched no arm and fell
to the final `else`, `!record_field_is_pointer(model, Db)` — and
`record_field_is_pointer` has no resource arm either (by design; a resource
*field* is a plain 8-byte slot). So the walk answered **`true` for both modes**,
by accident rather than by decision.

That accident is precisely the one the arm's own comment already records for
`Res(_)` before plan-114-B, and it has two consequences of very different size:

- **`type_is_memcpy_copyable(Db) == true`** routed a bare `Db` bind to
  `copy_flat_block`. That is the reported build failure, and it is the benign
  half — it fails loudly at compile time.
- **`type_is_arena_transferable(Db) == true`** is the silent half, and the worse
  one. That predicate is the gate on whether a thread transfer may relocate a
  block wholesale; a relocated resource handle points into the **sender's**
  arena, which is per-thread. It never got the chance to do damage only because
  the memcpy half failed the build first.

So the fix is at the classification, not at the dispatch: `flatness_walk`'s
resource arm now asks a model-aware predicate (`is_resource_nominal` = built-in ∪
`TypeModel::resource_names`), which answers `false` for both modes for the same
reason a built-in resource does — the resource record is separately allocated
with its own lifetime and its own close op.

### The sendability plumbing this doc scoped does not exist to be done

The section above concluded that `THREAD_SENDABLE` "does not reach codegen" and
that carrying it "through NIR" was the remaining work. That is wrong on both
paths, measured:

- **The project's own declarations.** `NirModule::native_resources` is
  `Vec<crate::ir::IrNativeResource>`, carried verbatim from the IR
  (`target/shared/nir/lower.rs`), and `IrNativeResource` has a `sendable: bool`
  field populated straight from the declaration
  (`ir/lower_link.rs`: `sendable: resource.thread_sendable`). It was already in
  the module `TypeModel::from_module` is built from; nothing needed plumbing.
- **An imported package's declarations.** `BinaryReprResourceExport` carries
  `sendable` from the `.mfp` `RESOURCE_TABLE`, and
  `TypeModel::from_module_and_packages` already reads those rows to register
  `resource_names` and `resource_closers`.

So the change is a `sendable_resource_names` set on `TypeModel`, filled from both
of those, and `is_sendable_resource_nominal` = built-in ∪ that set. A resource
declared WITHOUT `THREAD_SENDABLE` stays on the move-only pointer arm, matching
the frontend rule — verified: transferring one still fails with
`2-203-0063 TYPE_THREAD_NOT_SENDABLE` at four call sites.

### The deep-copy arm is the right one, and that is now a test result

This document argued from `link_thunk.rs` that a user resource's record is the
canonical plan-80 record, so `copy_resource_to_current_arena` is correct and the
move-only pointer arm would risk a cross-arena dangle. Both halves are now
measured rather than argued. With the sendable predicate temporarily narrowed so
a declared resource takes the **pointer** arm instead, the regression fixture
fails at runtime with `7-705-0009` (2000 iterations); with the deep-copy arm it
reports `used=2000 movedCount=2000` and exits 0. The fixture's iteration count is
what separates them — at three iterations both arms pass.

### A SECOND defect, same blind spot, found by census — bug-425 never reached user resources

Auditing every direct caller of the two builtin-only predicates turned up a
second one on this exact path. `builder_thread_cleanup.rs`'s `defer_resource_flag`
gated on `builtins::is_thread_sendable_resource_type`:

    let defer_resource_flag =
        matches!(target, "thread.transferResource" | "thread.emitResource")
            && crate::codegen::builtins::is_thread_sendable_resource_type(&arg_values[1].type_);

That flag is bug-425's whole fix: it defers the `moved|closed` store on the
SENDER's record from copy time to the enqueue-success branch, so a transfer that
fails leaves the sender's handle open. Builtin-only means it was never set for a
user-declared resource, so the store happened at copy time regardless of outcome.

Reproduced before the fix (worker blocks and never accepts, cap-1 queue, so every
`transfer(t, f, 0)` fails with `ErrTimeout`): the first use of the handle inside
the `TRAP` handler dies with

    Error: 7-703-0009
    Resource handle was moved to another thread by `thread::transfer` and is no
    longer usable by the sender.   [exit 255]

which contradicts `mfb man thread transfer` verbatim: "If the transfer fails, the
sending binding is still open, so a `TRAP` handler can close it or try again."
After the fix the same program reports `closed=50`, exit 0. Pinned by
`tests/rt-behavior/native/native-resource-transfer-fail-usable-rt`, the
user-resource twin of `thread-resource-transfer-fail-leak`.

This is why the fix widens *both* predicates through `TypeModel` rather than
special-casing the one dispatch that failed: the blind spot is the predicate, and
it had more than one consumer.

### What the regression fixture actually asserts

`tests/rt-behavior/native/native-resource-thread-accept-rt` covers the Goal's
second half ("the accepted handle is usable and closed exactly once"):

- `used=2000` is an **identity** proof, not a liveness one. Main creates the
  `seeded` table on each database before transferring it, and the worker's only
  statement is an `INSERT INTO seeded`. A handle that arrived duplicated,
  re-opened or pointing at a stale block would not have that table, so
  `sqlite3_exec` would return non-zero, `SUCCESS_ON status = 0` would raise, and
  the count would fall short.
- `movedCount=2000` is the **sender** half of closed-exactly-once: after the
  transfer the sender's binding raises `ErrResourceMoved` (7-703-0009), so the
  sender neither uses nor closes it. The receiver's scope drop then runs the
  registered close op through the path `native-resource-scope-drop-rt` already
  pins.

RSS was tried first as the "closed exactly once" instrument and rejected: with
two threads racing, the arena high-water mark depends on scheduling, and removing
work from the loop body *raised* peak RSS (153 MB vs 76 MB at 20 000
iterations). It is not a usable oracle here.

## Original hypothesis (now superseded by the section above)

`thread::accept`'s return type is the resource's declared type. The "native
inlined field size not available" message comes from the field-size lookup that
knows built-in resource record layouts; a user-declared `RESOURCE` is presumably
not registered where that lookup reads. bug-479 reaches the same message from a
different direction (`Result OF Thread OF …` in an inline `TRAP` desugar), which
suggests the lookup has a general fall-through rather than one missing entry —
worth reading both together before designing a fix.

## Goal

- The reproduction builds, and the accepted handle is usable and closed exactly
  once on the receiving thread.
- Failing that, `THREAD_SENDABLE` on a user-declared resource is rejected at the
  source level with a rule code and a location, rather than at native lowering
  with an internal message. Silently accepting the declaration and then failing
  the build is the worst of the three outcomes.

### Non-goals (must NOT change)

- `thread::accept`'s behaviour for built-in resources.
- The `RESOURCE … CLOSE BY` close path (bug-374/375 are both live there).

## Blast Radius

- Whatever owns the inlined-field-size table — shared with bug-479, so the two
  should be read together and may share a fix.
- `src/ir/verify/resources.rs` (`THREAD_SENDABLE` handling) and
  `src/codegen/runtime/thread/` (the transfer/accept copy), if the record layout
  of a user resource is genuinely not carried across.

## Validation Plan

- Regression test: the program above, built for every target (`validate` and
  lowering run per target).
- Runtime proof: only if the fix is the "make it work" branch — the accepted
  handle must reach the registered `CLOSE BY` op exactly once.

## Summary

Found while fixing bug-535, whose per-resource sweep asked the same question of
a user-declared resource that it asked of the six built-in sendable ones. The
six built-ins were the helper-accounting bug; this one is a different failure at
a different layer and needs its own answer.
