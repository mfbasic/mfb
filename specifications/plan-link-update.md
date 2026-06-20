# MFBASIC Native LINK Resource Plan

Last updated: 2026-06-20

This document plans how binding packages declare new resource types through the
native `LINK` `TYPE ... AS RESOURCE` form.

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
>   `TYPE ... AS RESOURCE` declaration and its `CPtr` representation (§5), the
>   transparent re-export (§5a), the native ABI surface (§5b), `RESOURCE_TABLE`
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
LINK "sqlite3" AS sqliteLink
  TYPE Db AS RESOURCE
    CLOSE close
  END TYPE

  FUNC open(path AS String) AS Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

' Publish the link-private resource type under the package's public name,
' so importers can write `sqlite::Db` (§5a).
EXPORT TYPE Db AS sqliteLink::Db
```

The `ABI (...)` / `SUCCESS_ON` shape above is the **proposed named-slot surface**
(§5b), not the current spec's positional form. The `EXPORT TYPE … AS …` line is
the **transparent re-export** (§5a).

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
  its `TYPE ... AS RESOURCE` declaration do not reach a usable AST or any later
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

A binding package introduces a native resource with the `LINK` form:

```basic
TYPE Db AS RESOURCE
  CLOSE close
END TYPE
```

Rules:

- `Db` is an opaque unique native handle. Its hidden representation is a `CPtr`.
  Source code cannot inspect, cast, compare, serialize, print, copy, capture in a
  closure, store in a collection, do arithmetic on it, or name its `CPtr`.
- A `CPtr` may exist **only** as the hidden representation of a declared
  `RESOURCE` (`mfbasic.md`). It must never escape into an ordinary MFBASIC API.
  This is exactly why the resource declaration lives inside `LINK`: it is the
  only legal home for a retained native pointer.
- `CLOSE close` names a native wrapper function **in the same `LINK` block** that
  releases the handle. It is the consuming close operation.
- A resource is **produced** by a native function whose MFBASIC return type is
  the resource type. The underlying ABI yields the pointer through an `OUT CPtr`
  parameter (e.g. `sqlite3_open`) or a pointer return; the compiler maps it into
  the owned handle — a LUT entry whose `Pointer` is the `CPtr` — without ever
  exposing a bare `CPtr`.
- Ownership, borrow, drop, and `STATE` all follow the overhaul. A native resource
  may carry data-only `STATE` like any resource (`RES db AS Db STATE Foo`); its
  `CPtr` lives in the LUT `Pointer` slot, never in user `STATE`.

## 5a. Naming & Re-exporting Resource Types

A `LINK` block declares its types inside the link's package-like namespace, so a
resource declared as `Db` in `LINK "sqlite3" AS sqliteLink` is named
`sqliteLink::Db`. Two problems follow:

1. Package-level wrapper code outside the `LINK` block (e.g.
   `EXPORT FUNC open(...) AS Db`) needs the bare name `Db` to resolve.
2. Importers can only write two-part qualified names (`package::identifier`,
   `mfbasic.md:1504`); they can reach `sqlite::Db` but never the package-internal
   `sqliteLink::Db`. Today nothing surfaces the link type under the package name,
   so importers can only *use* the type by inference (`LET db = sqlite::open(...)`)
   and can never *name* it (`FUNC useDb(db AS sqlite::Db)` fails to resolve).

Resolution: a **transparent type re-export**, spelled with the existing `TYPE`
vocabulary because a resource **is a type** (consistent with `File`/`Socket` and
with `TYPE … AS RESOURCE`):

```basic
EXPORT TYPE Db AS sqliteLink::Db
```

Rules:

- This is a **general type alias** — `[visibility] TYPE Alias AS QualifiedType`,
  usable for any type, not a resource-specific construct. Resources get no
  parallel export grammar; they are exported like any other type.
- It is **transparent**: `Db` and `sqliteLink::Db` are the **same nominal type**
  — same identity, same registry entry, same close op, same sendability. It is
  **not** a newtype; there is no conversion, because there is nothing to convert.
  All resource metadata is inherited from the target.
- It introduces the name `Db` at package scope, which (a) makes bare `Db` resolve
  in the package's wrapper code, and (b) exports it so importers see `sqlite::Db`.
- Visibility is per-alias, giving **per-type export control**: re-export `Db`,
  leave `Stmt` un-aliased and therefore package-private.

Decision record: an earlier idea spelled the declaration `RESOURCE Db … END
RESOURCE` and the export `EXPORT RESOURCE Db AS …`. Both were rejected in favor
of staying under `TYPE`, because resource-ness is an *attribute of a type* in
MFBASIC and is never re-stated at use sites; re-stating it at the export site
would be the inconsistency. Keep `TYPE … AS RESOURCE` for declaration and
`EXPORT TYPE … AS …` for re-export.

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
  `SUCCESS_ON`/`ERROR_ON`. `SUCCESS_ON status = 0` and `SUCCESS_ON status >= 0`
  unify the two existing keywords into one condition (`=` is MFBASIC equality,
  `mfbasic.md:134`).
- **`RETURN_OUT` by name.** Multi-out results reference slot names
  (`RETURN_OUT DivModResult[quotient, remainder]`) instead of positions.

Known ABI-vocabulary gaps the example exposes (none expressible today; each needs
a decision before a real binding compiles):

- **NULL / optional pointer arguments** — `sqlite3_exec`'s callback/arg/errmsg,
  `sqlite3_prepare_v2`'s `pzTail`. No NULL form exists.
- **Constant / literal arguments** — `sqlite3_prepare_v2`'s `nByte = -1`. ABI
  args can only come from wrapper params or `OUT`; there is no pinned constant.
- **Multi-valued result codes** — `sqlite3_step` returns row (100) / done (101) /
  error. `SUCCESS_ON`/`ERROR_ON` cannot express a three-way outcome; it needs the
  raw code surfaced to the wrapper (e.g. a `RESULT (status = 100)` mapping).
- **C-string return marshaling** — `sqlite3_column_text` returns `const char*`;
  `CString` is input-only today, so there is no return-a-`String` mapping.

This surface is part of the native-binding **frontend** whose ownership is still
open (§14, §17). This plan specifies the *surface design*; the parsing/lowering
of the full frontend is coordinated with `plan-linker.md`.

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
  `TYPE ... AS RESOURCE` form. The compiler must reject an opt-in it cannot honor.
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
For each `TYPE ... AS RESOURCE`, add an entry with:

- close operation = the `LINK` `CLOSE` wrapper (the registered close op);
- thread-sendable flag from the declaration (§8);
- close-may-fail flag (derived from the native close wrapper);
- `kind = native`.

The transparent re-export (§5a) does not add a second entry — it publishes another
*name* for the same registry entry.

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
  `TYPE ... AS RESOURCE` declaration semantics, drop/close/borrow/consume wiring,
  sendability, `RESOURCE_TABLE`, verification, and audit.
- **The `LINK`-block frontend — parsing and lowering — is not yet owned by either
  plan.** This plan now specifies the resource-facing *surface design*: the
  `TYPE ... AS RESOURCE` declaration and re-export (§5/§5a), producing a handle
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
  `TYPE ... AS RESOURCE [CLOSE ident]` declarations and native function
  declarations. (The broader ABI/native-call frontend is the dependency noted in
  §14; coordinate scope with `plan-linker.md`.)

### Phase 2: Resolve And Typecheck

- Resolve native resource names as `LINK`-scoped declarations and **register them
  into the overhaul's registry** (§9) as `kind = native`.
- Enforce the `CLOSE` wrapper rules (same `LINK` block, arity, parameter type).
- Wire the transparent re-export (§5a) to publish the package-level name.
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
   thread-sendability — a `THREAD_SENDABLE` keyword on `TYPE ... AS RESOURCE`, or
   a flag elsewhere? (Default stays non-sendable.)
2. **Native-binding frontend ownership.** Where is the `LINK` ABI/native-call
   frontend specified — extend `plan-linker.md`, or a new plan? This plan depends
   on it (§14).
3. **ABI surface ratification (§5b).** Adopt the named-slot ABI +
   `SUCCESS_ON`-expression form? Settle the result-marker keyword (`return` vs
   `RESULT`).
4. **ABI vocabulary gaps (§5b).** Decide forms for NULL/optional pointer args,
   pinned constant args, multi-valued result codes (`RESULT` mapping), and
   C-string return marshaling — none are expressible today.

## 18. Recommendation

Implement `plan-resource-overhaul.md` first — it establishes the resource model
and registry. Then add native `LINK` resources as the single user-facing way to
declare a new resource type, slotting into that model:

```basic
LINK "sqlite3" AS sqliteLink
  TYPE Db AS RESOURCE
    CLOSE close
  END TYPE
  ' ... native open/close wrappers ...
END LINK

EXPORT TYPE Db AS sqliteLink::Db
```

A native resource then behaves exactly like any other resource — `RES`-bound,
borrowed at calls, auto-closed via the LUT, closed early through its registered
close op, optionally `STATE`-carrying — with the only native-specific machinery
being the `CPtr` representation, the close-wrapper-as-registered-close, the ABI
surface (§5b), and `RESOURCE_TABLE` serialization. The previously-thorny
borrow/consume question for binding wrappers is resolved upstream by
borrow-by-default calls.
