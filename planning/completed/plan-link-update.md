# MFBASIC Native LINK Resource Plan

Last updated: 2026-06-20

This document plans how binding packages declare new resource types through the
native `LINK` package-scope `RESOURCE … CLOSE BY …` declaration.

It **assumes `specifications/plan-resource-overhaul.md` is implemented first.**
The overhaul establishes the resource model this plan builds on — the `RES`
binding kind, the fixed four-event invalidation model, borrow-by-default calls,
data-only `STATE`, the rule that `Type`s cannot contain resources, the runtime
resource LUT, and the data-driven resource registry. This plan does **not** restate
that model; it adds native `LINK` resources as one *kind* of resource that slots
into it.

It also supersedes the earlier *source-defined* ("representation-backed") resource
design — dropped (§3). The only user-facing way to introduce a new resource type
is the native `LINK` declaration.

> **Division of labor.**
> - `plan-resource-overhaul.md` owns resource *ownership*: `RES`, the LUT,
>   four-event invalidation, `STATE`, borrow-by-default, drop/close, the
>   cleanup-failure ledger, ownership verification, **and the resource registry**.
> - **This document** owns what is specific to native `LINK` resources: the
>   `RESOURCE … CLOSE BY …` declaration and its `CPtr` representation (§5), the
>   package-scope naming and close-op re-export (§5a), the native ABI surface (§5b), `RESOURCE_TABLE`
>   serialization and the sendable/native flags (§10), and **registering** native
>   resources into the overhaul's registry (§9).
>
> Where the overhaul references "`plan-link-update.md` §9 (registry)" or "§6a
> (ledger)", treat those as the overhaul's own foundation that lands with it; this
> plan now consumes them rather than building them.

It complements:

- `specifications/mfbasic.md`
- `specifications/standard_package.md`
- `specifications/package_format.md`
- `specifications/architecture.md`
- `specifications/memory_layouts.md`
- `specifications/threading.md`
- `specifications/plan-linker.md` (native library loading — see §14)

## 1. Goal

Let a binding package declare a new **native resource** through `LINK`, and have
it behave as an ordinary resource under the overhaul's model (RES-bound,
borrow-by-default, LUT-tracked, closed via its registered close op):

```basic
' Declare the native resource at PACKAGE scope. `CLOSE BY` names its registered
' close op — a native LINK function. `EXPORT` makes the type nameable by
' importers as `sqlite::Db`, like any exported type.
EXPORT RESOURCE Db CLOSE BY sqliteLink::close

LINK "sqlite3" AS sqliteLink
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

' Re-export the registered close op under the package name so importers can close
' explicitly through `sqlite::close`. A consuming wrapper is impossible — §6.
EXPORT FUNC close AS sqliteLink::close
```

Resource positions use the overhaul's explicit `RES` / `AS RES` spelling
(`RES db AS Db`, `AS RES Db`) — a resource is `RES` in every binding, parameter,
and return position, native `LINK` functions included.

The `ABI (...)` / `SUCCESS_ON` shape above is the **proposed named-slot surface**
(§5b), not the current spec's positional form. The resource declaration form
(`RESOURCE … CLOSE BY …`) is specified in §5.

Importers bind `Db` with `RES` and use it under the overhaul's model:

- bound with `RES`, uniquely owned, never copied;
- borrowed at ordinary calls; invalidated only by the four events (registered
  close + aliases, `thread::send`, `RETURN`, scope-drop);
- auto-closed on scope exit via the LUT;
- not inspectable, forgeable, or constructible outside the binding boundary;
- not thread-sendable unless the resource opts in (§8).

A worked example lives in the `bindings/sqlite/` package (`src/lib.mfb`). It
declares two native resources (`Db` and a private `Stmt` prepared statement),
re-exports `Db`, and builds `createTable`/`dropTable`/`listTables` on top. Under
the overhaul those wrappers simply **borrow** `db` (calls borrow by default), so
the borrow/consume problem this plan previously wrestled with (old §7a) is
resolved upstream; what the example still surfaces is the ABI-vocabulary set in
§5b.

## 2. Current State (compiler reality)

Assume the overhaul has landed first, so the resource model and the registry are
already in place (registry-driven recognition replaces the old hardcoded
`builtins::is_resource_type` matching; `RESOURCE_TABLE` is consumed on import; the
sendable bit round-trips; `RES`/LUT/four-event ownership is enforced). What
remains specific to native `LINK` resources:

- Native `LINK` blocks are **non-functional**. They parse, but a `LINK` block and
  its `RESOURCE … CLOSE BY …` declaration do not reach a usable AST or any later
  stage. This plan makes them real.
- Only `File`, `Socket`, and `Listener` are actually wired up as built-in
  resources (`src/builtins/fs.rs`, `src/builtins/net.rs`). `UdpSocket` and
  `TlsSocket` are documented but **not implemented** — orthogonal to this plan,
  noted so they are not assumed to exist.

(For the historical pre-overhaul state — hardcoded recognition across
`typecheck.rs`/`binary_repr.rs`/`target/shared/*`, the unused `RESOURCE_TABLE`,
the never-written sendable bit — see `plan-resource-overhaul.md` §2, which is what
fixes them.)

## 3. Non-Goals

This plan does not add:

- **source-defined / representation-backed resources.** The dropped design let a
  package declare a resource whose hidden representation was an MFBASIC record
  with a user-written close function. It is cut because the only state that
  genuinely needs deterministic close is *side-effecting* cleanup (flush,
  rollback, delete, unlock); plain owned data is already freed by lexical drop.
  User-defined RAII guards may be revisited later as their own plan, but they are
  not part of the resource model now.
- copyable resources, non-movable resources, a `MOVABLE` option
- generic `RESOURCE` parameters or structural matching between resource types
- public field access on resources, or any user-visible representation
- storing resources in ordinary `List`/`Map` (and resource-typed record/union
  fields stored in collections). This stays a non-goal and is **explicitly
  deferred**: the cleanup model is per-binding lexical drop, not per-element, and
  collections have no per-element drop machinery. Supporting it would need
  element-wise close at every drop/overwrite/remove site, per-slot ownership
  tracking, and a borrow-from-container rule (resources are not copyable, so
  `list[i]` cannot return by copy). That is a separate subsystem for a future
  plan.
- resource capture by ordinary closures
- user-authored `BORROW`, `MOVE`, or lifetime annotations

Resources remain nominal, concrete, opaque handle types.

## 4. Resource Kinds

All resources are **opaque handles** with no user-accessible backing store. Two
kinds exist:

| kind | representation | close | lowering |
|------|----------------|-------|----------|
| built-in / standard | native handle (fd / host pointer) | compiler-provided | opaque |
| native (`LINK`) | native pointer (`CPtr`) | `LINK` wrapper function | opaque |

The registry (§9) records the kind. Opacity is the defining property: a resource
is something whose representation the compiler hides and whose cleanup it drives
exactly once. The dropped source design tried to give resources a *visible*
MFBASIC representation; without it, both remaining kinds share one opaque path.

## 5. Native Resource Declaration

The overhaul defines the resource *model* but no syntax for **introducing** a new
resource type. That declaration form is native-specific and lives here. A binding
package introduces a resource at **package scope** with:

```basic
EXPORT RESOURCE Db CLOSE BY sqliteLink::close
```

`[visibility] RESOURCE <Name> CLOSE BY <closeFn>`:

- `Db` is an opaque unique native handle. Its hidden representation is a `CPtr`.
  Source code cannot inspect, cast, compare, serialize, print, copy, capture in a
  closure, store in a collection, do arithmetic on it, or name its `CPtr`.
- A `CPtr` may exist **only** as the hidden representation of a declared
  `RESOURCE` (`mfbasic.md`). It must never escape into an ordinary MFBASIC API.
  The retained native pointer is produced and freed exclusively by the `LINK`
  functions named below.
- **`CLOSE BY <closeFn>`** names the resource's registered close op — a native
  `LINK` function (e.g. `sqliteLink::close`) whose single `RES` parameter is this
  resource type. It is the consuming close operation (overhaul invalidation event
  #1). `closeFn` must be a native `LINK` function (or a built-in close op); naming
  an ordinary MFBASIC function would reintroduce the cut source-defined-resource
  design (§3) and is rejected.
- The resource is **declared at package scope, not inside the `LINK` block.** This
  is the deliberate fix for the old naming problem (below): the type is named and
  exported exactly like any other type — bare `Db` resolves in the package's
  wrapper code, and `EXPORT` makes importers able to write `sqlite::Db`. Omitting
  `EXPORT` keeps it package-private (e.g. `RESOURCE Stmt CLOSE BY sqliteLink::finalize`).
  The declaration may forward-reference a `LINK` function defined later in the file.
- A resource is **produced** by a native function whose MFBASIC return type is the
  resource type (`AS RES Db`). The underlying ABI yields the pointer through an
  `OUT CPtr` parameter (e.g. `sqlite3_open`) or a pointer return; the compiler maps
  it into the owned handle — a LUT entry whose `Pointer` is the `CPtr` — without
  ever exposing a bare `CPtr`.
- Ownership, borrow, drop, and `STATE` all follow the overhaul. A native resource
  may carry data-only `STATE` like any resource (`RES db AS Db STATE Foo`); its
  `CPtr` lives in the LUT `Pointer` slot, never in user `STATE`.

## 5a. Naming & Re-exporting

Declaring the resource at **package scope** (§5) resolves the naming problem that
an earlier draft solved with a type self-alias. Because `EXPORT RESOURCE Db …`
introduces `Db` at package scope directly:

1. Package-level wrapper code (`EXPORT FUNC open(...) AS RES Db`) resolves the bare
   name `Db`; and
2. importers can both *use* the type by inference (`RES db = sqlite::open(...)`)
   **and** *name* it (`FUNC useDb(RES db AS sqlite::Db)`), because `sqlite::Db` is
   an ordinary exported type.

No `EXPORT TYPE Db AS sqliteLink::Db` self-alias is needed or used — that earlier
form aliased a type to itself across the link namespace and is dropped.

**Re-exporting the close op (a function alias).** The package still needs to
publish a close op under its own name so importers can close explicitly
(`sqlite::close(db)`). This must be a **function alias**, not a wrapper:

```basic
EXPORT FUNC close AS sqliteLink::close
```

`[visibility] FUNC alias AS qualified::func` re-exports a `LINK` function under the
package name as a transparent alias — same function, same signature, and, for the
close op, the **same registered close op**. It is required, not cosmetic: the
registered close op consumes its argument (invalidation event #1), but a
hand-written wrapper `FUNC close(RES db AS Db) … sqliteLink::close(db)` cannot —
its parameter is a *borrow*, and a borrow may never invalidate (overhaul §3.3),
with no `MOVE` annotation to make a parameter consuming. So calling `sqlite::close(db)`
is invalidation event #1 on `db` exactly as the native close op is. (Surfaced by
the `bindings/sqlite` stress test.)

## 5b. Native Binding ABI Surface (proposed)

The current spec maps wrapper parameters to ABI parameters **positionally**, lets
a single `OUT` become the result implicitly, and gates success on a **literal**
(`SUCCESS_ON 0`). This plan proposes a more explicit surface, motivated by the
`bindings/sqlite` example:

```basic
ABI (path CString, return OUT CPtr) AS status CInt32
SUCCESS_ON status = 0
```

- **Named slots.** Each ABI slot is `name type`. Input names bind to wrapper
  parameters by name (`path` ↔ `open`'s `path`); `OUT` slots and the native
  return get their own names. This removes positional coupling and the drift a
  separate name list (`ARGS (…)`) would reintroduce.
- **Named return marker.** One ABI slot is marked as the wrapper's result. The
  spelling `return OUT CPtr` is provisional; `return` is a poor word because the
  *actual* C return here is the status code, not that out-param — `RESULT` reads
  better. Open Question §17.
- **`SUCCESS_ON <expression>`** over named slots, replacing literal-only
  `SUCCESS_ON`/`ERROR_ON`. Any Boolean expression is allowed, including compound
  conditions: `SUCCESS_ON status = 0`, `SUCCESS_ON status >= 0`, and
  `SUCCESS_ON status = 100 OR status = 101`. Comparisons bind tighter than
  `AND`/`OR` (mfbasic.md §11 — precedence 8 vs 10/11), so the compound form needs
  no parentheses. `ERROR_ON` is the De Morgan complement
  (`ERROR_ON status <> 100 AND status <> 101`); the two collapse to one condition,
  so a wrapper states one, not both. (`=` is MFBASIC equality, `mfbasic.md:134`;
  `AND`/`OR` operands must be Boolean — there is no integer truthiness, so
  `status = 100 OR 101` is a type error, not shorthand.)
- **`RETURN_OUT` by name.** Multi-out results reference slot names
  (`RETURN_OUT DivModResult[quotient, remainder]`) instead of positions.

The example exposed four ABI-vocabulary needs. Three are now **resolved**; one
declarative gap remains.

Resolved by `CONST` (§5c):

- **Constant / literal arguments** — `sqlite3_prepare_v2`'s and
  `sqlite3_bind_text`'s `nByte = -1`.
- **NULL / optional pointer arguments** — `sqlite3_exec`'s callback/arg/errmsg,
  `sqlite3_prepare_v2`'s `pzTail`, `sqlite3_open_v2`'s `zVfs`, and special
  sentinel pointers such as `sqlite3_bind_text`'s `SQLITE_TRANSIENT` (`(void*)-1`).

Resolved otherwise:

- **C-string return marshaling** — `sqlite3_column_text`'s `const char*` → `String`
  is generated-thunk codegen (the mirror of the existing `String` → `char*` input),
  not new ABI vocabulary. Specified in `plan-linker.md` §12.4 (copy-and-leave
  ownership, NULL handling, UTF-8 validation).
- **Multi-valued result codes — error half** — `sqlite3_step`'s row (100) / done
  (101) / error three-way no longer needs a special form to *avoid auto-propagating*
  the non-error codes: the compound `SUCCESS_ON status = 100 OR status = 101` gates
  it (above).

Still open (one declarative gap):

- **Result *value* mapping (`RESULT`)** — surfacing *which* non-error code occurred
  into the wrapper's result. `step` returns `AS Boolean` meaning "a row is ready"
  (`TRUE` on 100, `FALSE` on 101); the success gate decides error-vs-not but not
  that value. It needs an expression-valued result marker — `RESULT status = 100` —
  distinct from a plain slot passthrough (`AS return CInt32`). (`bindParameterIndex`
  needs no such mapping: it returns its raw index straight through `AS return CInt32`,
  a passthrough with no gate, so it is already expressible.)

This surface is part of the native-binding **frontend** whose ownership is still
open (§14, §17). This plan specifies the *surface design*; the parsing/lowering
of the full frontend is coordinated with `plan-linker.md`.

## 5c. Pinned Constant & NULL Arguments (`CONST`)

The `ABI (...)` line always states the **true native signature** — every C
argument in C order. A wrong-arity `ABI` does not "omit" arguments; it generates a
broken call frame, with the function reading the unstated slots out of whatever
registers happen to hold. But some C arguments are not values the caller supplies
— they are fixed constants the binding must pin: `nByte = -1`, a NULL callback,
the `SQLITE_TRANSIENT` destructor sentinel. `CONST` pins them so the `ABI` line can
stay honest while the wrapper signature stays clean:

```basic
FUNC bindText(RES stmt AS Stmt, iCol AS Integer, zVal AS String) AS Nothing
  SYMBOL "sqlite3_bind_text"
  ABI (stmt CPtr, iCol CInt32, zVal CString, nByte CInt32, destructor CPtr) AS status CInt32
  CONST nByte = -1            ' bind up to the terminating NUL
  CONST destructor = -1       ' SQLITE_TRANSIENT (void*)-1: copy the bytes now, do not alias
  SUCCESS_ON status = 0
END FUNC
```

`CONST <slot> = <value>` pins one ABI slot to a fixed value and **removes it from
the wrapper's parameter list**. This completes the slot-binding model: every ABI
slot is satisfied by exactly one of —

- a wrapper parameter, matched by name (`iCol` ↔ the `iCol` parameter);
- the `OUT` / result marker (the produced handle or a `RESULT` value);
- a `CONST` pin.

Rules:

- **The slot owns the type.** `CONST` names only the slot and the value; the value
  is checked against the slot's declared ABI type (`nByte`'s `CInt32`,
  `destructor`'s `CPtr`). `CONST` does not redeclare a type, so a pin cannot drift
  from its slot.
- **`NOTHING` pins a C NULL / void pointer** on a pointer slot — the form for
  optional/absent pointers (`prepare`'s `pzTail`, `openV2`'s `zVfs`, `exec`'s
  callback/arg/errmsg).
- **A pointer-sized integer literal pins a sentinel pointer** — e.g.
  `CONST destructor = -1` for `SQLITE_TRANSIENT` (`(void*)-1`). This is the single
  place a non-NULL pointer constant may be named in source.
- **CPtr containment is preserved.** A `CONST` pin is call metadata baked into the
  native frame at lowering; it never materializes as a source-level value, is
  never bound, stored, printed, or returned, and so cannot forge or leak a `CPtr`
  (§5/§11). The containment rule governs *values* — a pin is not one.
- A `CONST` slot is **input-only**: it may not also be marked `OUT` or be the
  result marker.

## 6. Close & Drop (defers to the overhaul)

Ownership, drop, exactly-once close, borrow-by-default, and use-after-close are
all the overhaul's (`plan-resource-overhaul.md` §3.3, §5, §6). Native specifics:

- The `LINK` `CLOSE` wrapper **is the registered close op** for the native
  resource — i.e. overhaul invalidation event #1. Its re-export alias (§5a) is
  the same close op. Nothing else native consumes the handle.
- **Close can fail** (nonzero native status). Explicit close auto-propagates the
  failure as a `Result`; a drop-close failure goes to the overhaul's runtime
  **cleanup-failure ledger** (log-on-exit) and is reported by `mfb audit` (§13).
  Whether a native resource's close may fail is a registry/`RESOURCE_TABLE` flag
  (§10).
- Calling a native wrapper with an already-closed handle yields the overhaul's
  runtime `ErrResourceClosed` backstop.

Because calls borrow by default (overhaul), native operations like
`busyTimeout`/`exec`/`step` borrow without any per-call inference, and the old
"package-wrapper borrow vs consume" problem disappears: `createTable`/`listTables`
borrow `db`, a re-exported `close` is an alias of the registered close op. No
`BORROW`/`MOVE` annotations are introduced.

## 7. Borrow & Consume → see the overhaul

Borrow-by-default and the four invalidation events are
`plan-resource-overhaul.md` §3.3/§7. The only native-specific facts (the `CLOSE`
wrapper is the registered close op; everything else borrows) are stated in §6
above; there is nothing further to define here. This section is retained only to
keep section numbers stable for cross-references.

## 8. Thread Sendability

The send *mechanics* — `thread::send` as the LUT cross-table move, move on
success / stay on failure, queued resources owned by the runtime queue — are the
overhaul's (`plan-resource-overhaul.md` §7). This plan owns only how a **native**
resource **opts in**, and getting that opt-in to round-trip:

- Resources are **not sendable by default** (a native handle is usually not safe
  to move across threads).
- A native resource opts in through its declaration; the exact surface is an open
  question (§17) — most likely a `THREAD_SENDABLE` keyword on the
  `RESOURCE … CLOSE BY …` declaration. The compiler must reject an opt-in it cannot honor.
- Sendability is a per-resource registry property, serialized via the
  `RESOURCE_TABLE` sendable bit (§10) so it survives import. (The registry and
  the bit-2 fix land with the overhaul; this plan ensures native declarations set
  it.)

## 9. Registering Native Resources

The **registry itself is the overhaul's** (`plan-resource-overhaul.md`): a single
dynamic table, keyed by resolved type name, that every compiler stage consults
instead of the old hardcoded matching. It is seeded with built-ins and populated
from imported `RESOURCE_TABLE`s.

This plan's job is to **register native `LINK` resources** into it during resolve.
For each `RESOURCE … CLOSE BY …` declaration, add an entry with:

- close operation = the `LINK` `CLOSE` wrapper (the registered close op);
- thread-sendable flag from the declaration (§8);
- close-may-fail flag (derived from the native close wrapper);
- `kind = native`.

The close-op re-export and the package-scope declaration (§5a) do not add a second
registry entry — the resource has exactly one entry, keyed by its type name.

## 10. Package Metadata

`.mfp` packages must preserve resource metadata for imported resources.

`RESOURCE_TABLE` encodes per entry: `type_id`, `close_function_id`, `flags`
(`src/binary_repr.rs`). Flags as actually implemented:

- bit 0 = native resource (`RESOURCE_FLAG_NATIVE`)
- bit 1 = standard resource (`RESOURCE_FLAG_STANDARD`)
- bit 3 = close may fail (`RESOURCE_FLAG_CLOSE_MAY_FAIL`)

The overhaul already makes `RESOURCE_TABLE` round-trip: it consumes the table on
import (decoding entries into the registry) and adds `bit 2 = sendable to thread`.
This plan's job is to make a **native `LINK` resource serialize correctly** into
that table:

- set `bit 0` (native), `bit 2` (sendable, per §8), and `bit 3` (close may fail)
  appropriately;
- point `close_function_id` at the `LINK` `CLOSE` wrapper.

(No "source resource" flag is needed — that was for the dropped design. Native
resources are distinguished by the existing native flag.)

The ABI hash for exported resource types must include: resource type name, close
wrapper signature, ownership/borrow/consume behavior, thread-sendability flag,
and whether close may fail.

## 11. Verification (native-specific)

The general ownership verifier — no copy, closed/moved/returned exactly once,
drop on every exit path, no resource inside a `Type`, thread-boundary rules — is
the overhaul's (`plan-resource-overhaul.md` §11). Native resources reuse it
unchanged; their lifetime is implicit (owned by the `RES` binding's scope, closed
by lexical drop via the registry close op), with no explicit resource ops in the
Binary Representation.

This plan adds only the native-specific check:

- a **`CPtr` cannot escape** into an ordinary value or API — it exists solely as
  the LUT `Pointer` of a declared resource (§5). This is the one verification
  unique to native resources.

## 12. Diagnostics

Add/keep diagnostics for:

- duplicate resource declaration name
- `CLOSE` names a function that is missing, not in the same `LINK` block, has the
  wrong arity, or has a parameter type that is not the resource
- `CPtr` escaping into an ordinary MFBASIC API
- attempted copy of a resource
- attempted field access on a resource
- attempted ordinary collection storage of a resource
- attempted thread send of a non-sendable resource
- thread-sendable opt-in the compiler cannot honor
- use after move or close
- control-flow path that can lose or double-close a resource

If any new error codes are added, update `specifications/error_codes.md`,
`specifications/mfbasic.md`, and `specifications/standard_package.md` in the same
change.

## 13. Audit Behavior

`mfb audit` reports native resources the same way it reports standard resources.
For each resource type: declaring package, source location when available,
whether it is standard or native, the close operation, whether close can fail,
thread-sendability, the cleanup regions and exits that trigger lexical drop, and
whether secondary drop-close failures were retained (§6a). Audit output must not
reveal native pointer values.

## 14. Boundary With plan-linker.md

These two plans are layered and must not overlap:

- **`plan-linker.md`** owns the **object-file linking** of native libraries: ELF
  `GLOB_DAT`, symbol versioning, multi-library bind opcodes, Mach-O dylib
  ordinals, load-time initializers. It does not touch `LINK`-block grammar, ABI
  mapping, or resource semantics.
- **This plan** owns the **resource model**: the registry, the
  `RESOURCE … CLOSE BY …` declaration semantics, drop/close/borrow/consume wiring,
  sendability, `RESOURCE_TABLE`, verification, and audit.
- **The `LINK`-block frontend — parsing and lowering — is not yet owned by either
  plan.** This plan now specifies the resource-facing *surface design*: the
  `RESOURCE … CLOSE BY …` declaration and close-op re-export (§5/§5a), producing a handle
  from `OUT`/pointer return, the close wrapper, and the proposed named-slot ABI /
  `SUCCESS_ON`-expression shape (§5b). What remains unowned is the *implementation*
  of the general native-binding frontend — block parsing to AST, the full
  `ABI`/`OUT`/`REF`/`RETURN_OUT` mapping and its vocabulary gaps (§5b), and native
  call lowering. That should be specified separately — recommended as an addition
  to `plan-linker.md` — and is called out as Open Question §17.

## 15. Implementation Phases

Prerequisite: `plan-resource-overhaul.md` is implemented (RES/LUT/four-event
model, registry, `RESOURCE_TABLE` round-trip, sendable bit). These phases assume
that foundation.

### Phase 1: LINK Parse To AST

- Make `LINK` blocks parse into a real AST node, including
  package-scope `RESOURCE … CLOSE BY …` declarations and native function
  declarations. (The broader ABI/native-call frontend is the dependency noted in
  §14; coordinate scope with `plan-linker.md`.)

### Phase 2: Resolve And Typecheck

- Resolve native resource names as `LINK`-scoped declarations and **register them
  into the overhaul's registry** (§9) as `kind = native`.
- Enforce the `CLOSE` wrapper rules (same `LINK` block, arity, parameter type).
- Register the package-scope `RESOURCE … CLOSE BY …` declaration and wire the
  close-op function alias (§5a).
- Enforce `CPtr` containment (cannot escape an ordinary API).
- Set thread-sendability from the declaration (§8).
- (Ownership/borrow/`STATE`/use-after-close fall out of the overhaul — no
  per-native work.)

### Phase 3: Produce And Close Lowering

- Lower native functions that produce a resource (map `OUT CPtr` / pointer return
  into a LUT entry whose `Pointer` is the `CPtr`).
- Register the `LINK` `CLOSE` wrapper as the resource's registered close op so the
  overhaul's drop/close machinery invokes it (no new structured resource ops).
- Native close failures flow into the overhaul's cleanup-failure ledger.

### Phase 4: Package Metadata

- Serialize native resources into `RESOURCE_TABLE` with the right flags (§10):
  native, sendable, close-may-fail; `close_function_id` → the `CLOSE` wrapper.
- Include native resource behavior in ABI hash inputs.

### Phase 5: Verification

- Add the native-specific `CPtr`-escape check (§11). General ownership checks come
  from the overhaul.

### Phase 6: Runtime Validation

- Compile and execute programs proving a native resource: opens/closes; auto-
  closes on normal and error exit; is borrowed across multiple wrapper calls (no
  use-after-move); explicit close prevents a double close; is rejected at a thread
  boundary unless declared sendable; and (if sendable) transfers through a thread
  queue.

## 16. Test Requirements

Add acceptance tests under both valid and invalid directories for every
function involved in resource production, close, import, and thread transfer.

Required valid scenarios:

- a binding package opens and closes a native resource
- an importer receives and uses the resource through exported wrappers
- explicit close succeeds and prevents lexical double-close
- lexical drop closes on success return
- lexical drop closes on error propagation
- a resource returned from a function is not closed by the callee
- a thread-sendable resource moves through `thread::send` (once a sendable native
  resource exists to test with)

Required invalid scenarios:

- a `CPtr` is made to escape into an ordinary API
- a resource is copied then used twice
- a resource is stored in a `List`
- a `CLOSE` wrapper has the wrong arity or parameter type
- a non-sendable resource is sent to a thread
- a resource is used after explicit close
- a resource is lost or double-closed on a control-flow path

After compiler work, run:

```text
scripts/test-accept.sh target/debug/mfb target/accept-actual
```

Runtime features must also be validated by executing generated programs and
observing close behavior through a concrete side effect.

## 17. Open Questions

(Ownership-model questions — borrow/consume inference, the cleanup-failure ledger
scope — are resolved or owned by `plan-resource-overhaul.md`. What remains here is
native-`LINK`-specific.)

1. **Sendability opt-in surface.** How does a native resource declare
   thread-sendability — a `THREAD_SENDABLE` keyword on the `RESOURCE … CLOSE BY …` declaration, or
   a flag elsewhere? (Default stays non-sendable.)
2. **Native-binding frontend ownership.** Where is the `LINK` ABI/native-call
   frontend specified — extend `plan-linker.md`, or a new plan? This plan depends
   on it (§14).
3. **ABI surface ratification (§5b).** Adopt the named-slot ABI +
   `SUCCESS_ON`-expression form? Settle the result-marker keyword (`return` vs
   `RESULT`).
4. **ABI vocabulary gaps (§5b).** Constant/NULL/sentinel args are resolved by
   `CONST` (§5c); C-string return marshaling is generated-thunk codegen
   (`plan-linker.md` §12.4); a multi-valued result code's *error* classification is
   resolved by the compound `SUCCESS_ON`/`ERROR_ON` form (§5b). The sole remaining
   declarative gap is the result *value* mapping — an expression-valued `RESULT`
   marker (e.g. `RESULT status = 100` for `step`'s row-vs-done Boolean).

## 18. Recommendation

Implement `plan-resource-overhaul.md` first — it establishes the resource model
and registry. Then add native `LINK` resources as the single user-facing way to
declare a new resource type, slotting into that model:

```basic
EXPORT RESOURCE Db CLOSE BY sqliteLink::close

LINK "sqlite3" AS sqliteLink
  ' ... native open/close wrappers ...
END LINK

EXPORT FUNC close AS sqliteLink::close   ' publish the close op (§5a)
```

A native resource then behaves exactly like any other resource — `RES`-bound,
borrowed at calls, auto-closed via the LUT, closed early through its registered
close op, optionally `STATE`-carrying — with the only native-specific machinery
being the `CPtr` representation, the close-wrapper-as-registered-close, the ABI
surface (§5b), and `RESOURCE_TABLE` serialization. The previously-thorny
borrow/consume question for binding wrappers is resolved upstream by
borrow-by-default calls.
