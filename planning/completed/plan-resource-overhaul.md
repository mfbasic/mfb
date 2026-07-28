# MFBASIC Resource Model Overhaul Plan

Last updated: 2026-06-20 (all phases implemented; see §10 Phases status)

This plan redefines how resources work in MFBASIC. It replaces the current model —
where a resource is a non-copyable *type* threaded through the same copy/move
machinery as ordinary data, with borrow/consume decided per call — with a
dedicated binding kind (`RES`), a small fixed set of ownership-transfer events,
and a runtime handle table (LUT).

This plan is **foundational and lands first.** It owns the resource model end to
end — the `RES` binding kind, the four-event invalidation model, the LUT, `STATE`,
and the data-driven **resource registry** (including `RESOURCE_TABLE` round-trip
and the sendable-bit fix). `plan-link-update.md` (native `LINK` resources) is
built **on top of** this plan and depends on it — not the other way around.

It revises `mfbasic.md` §14 (Memory Semantics) and §15 (Resource Management), and
**supersedes the resource-ownership sections** of `plan-link-update.md` (§5–§7,
§11), which defer here.

It complements:

- `specifications/mfbasic.md`
- `specifications/standard_package.md`
- `specifications/package_format.md`
- `specifications/threading.md`
- `specifications/plan-link-update.md` (native `LINK` resources — built on this plan)

## 1. Why

The current model fits a uniquely-owned, must-close-once *handle* through a type
system built for copy/move of *data*. That mismatch produces two concrete
problems:

1. **A borrow/consume footgun.** The compiler hardcodes borrow-vs-consume for
   built-ins (`fs.close`/`net.close` consume, every other op borrows), but for
   *user* functions it falls back to "consume any non-copyable argument"
   (`typecheck.rs:4725` `argument_mode_for_type` → `Transfer`;
   `deactivate_moved_resource_arguments` `else => true`). The result: **you
   cannot write a borrowing user function over a resource.** A helper like
   `logLine(f AS File, msg AS String)` consumes `f`, so the caller can never use
   the file again. This already bites `File`/`Socket` today and makes a multi-
   operation binding API (`createTable` then `dropTable`) impossible.

2. **Resource-ness leaks through the whole type system.** Because resources are
   types, every stage carries "is this (transitively) a resource" special-casing
   (copyability, collection bans, drop sensitivity), and the user can't see at a
   binding site whether a value is a unique handle or plain data.

The core fact the overhaul leans on: **a resource is globally unique — exactly
one live owner, closed exactly once.** All the move machinery exists only to
enforce that. If ordinary calls simply *borrow*, the set of events that can end a
resource's life shrinks to a tiny fixed list, the footgun disappears, and the
model stays statically checked.

## 2. Current State (compiler reality)

- Binding/argument ownership uses three modes, `ExprMode::{Read, Transfer,
  Borrow}` (`typecheck.rs:95`). `Borrow` is behaviorally identical to `Read`:
  both "require owned, don't consume." There is no `OwnershipState::Borrowed` —
  the states are `Available | Moved | MaybeMoved` (`typecheck.rs:49`).
- `fs.close`/`net.close` arg 0 → `Transfer` (consume); all other `fs`/`net` ops →
  `Borrow` (`typecheck.rs:3697`, `:3760`).
- User-function resource args → `Transfer` (consume) by the non-copyable rule —
  the footgun (`typecheck.rs:4725`).
- Resources are recognized by **hardcoded type name** —
  `builtins::is_resource_type` / `resource_close_function` /
  `is_thread_sendable_resource_type`, matched at ~8 call sites across
  `typecheck.rs`, `binary_repr.rs`, and `target/shared/*`. There is no registry;
  `RESOURCE_TABLE` is written for the three built-ins but never read back, and the
  spec's sendable bit is never written. This plan introduces the registry (§10).
- A `File` is a `Type`; `LET f = fs::open(...)` binds it like any value.

The user-facing memory model in `mfbasic.md` §14 is copy/move (+freeze for
collections); "borrow" is documented only for resource operations (§15) and is
not a source-level keyword.

## 3. The New Model

### 3.1 Three binding kinds

| keyword | holds | semantics |
|---------|-------|-----------|
| `LET` | copyable data (primitives, records, containers) | immutable binding |
| `MUT` | copyable data | mutable binding |
| `RES` | a resource (optionally carrying a data `STATE`) | uniquely owned, move-only, auto-closed on drop |

- `LET`/`MUT` is the **mutability** axis. `RES` is the **ownership** axis: a
  uniquely-owned value has no aliases, so mutability control is moot — `RES`
  needs no immutable/mutable sub-distinction.
- All three share **one identifier namespace**. `RES a` then `LET a` is a
  redefinition error.
- A resource **cannot** be bound with `LET`/`MUT`, and a copyable value cannot be
  bound with `RES`. The binding keyword is required and enforced; it *surfaces* a
  type property, it does not choose it.

### 3.2 Resources vs. data types (records can't hold them; unions can't mix)

`File`, `Socket`, `Listener`, and `LINK`-declared handles are **resources**. They
appear in `RES` positions — binding, parameter, and return — and **never inside a
data type** (the one exception being the variants of a *resource union*, §4a,
which is itself a resource, not data):

```basic
RES f AS File = fs::open("app.db")                  ' binding
FUNC exec(RES db AS File, sql AS String) AS Nothing ' parameter
FUNC open(path AS String) AS RES File               ' return
```

The rule is **product vs. sum**, and it removes the "ownership contagion" footgun:

- **A record (product) may never contain a resource**, directly or transitively.
  A record owns all its fields *at once*, so a resource field would either trap
  copyable data (silently making the record move-only — contagion) or, with two
  resource fields, own several resources as one value. Records stay pure copyable
  data, always. Data that belongs *with* a resource travels in its `STATE` (§4),
  not in a record.
- **A union (sum) is either all-data or all-resource — never mixed.** A union owns
  exactly *one* variant at a time:
  - every variant a data type → an ordinary copyable `Type`;
  - every variant a resource → itself a **resource union** (§4a): move-only,
    `RES`-bound, dropped by dispatching on the tag to the active variant's close
    op. Because it owns exactly one resource at any moment, it does **not** violate
    "no value owns multiple resources" — it is a *choice* among resources, not a
    *bundle*;
  - a **mixed** union (some data, some resource variants) is **rejected**: its
    copyability would depend on the runtime tag, which is the contagion +
    conditional-drop problem.

This keeps resources **atomic**: a resource never owns another resource, and no
value ever owns several resources at once.

### 3.3 The fixed invalidation-event set (the core rule)

**Ordinary calls borrow.** Stated plainly:

> Passing a `RES` binding to an ordinary function creates an **exclusive,
> call-scoped borrow**. The callee may use the handle and **mutate its `STATE`**.
> The callee does **not** take ownership, and the caller's binding remains **live
> after the call** — unless the call is one of the fixed invalidation events
> below, which are visible at the call site.

The borrow is *exclusive* and safe to mutate through because the resource is
uniquely owned and the call is synchronous: while the callee holds the borrow,
the caller cannot touch the binding, so there is no aliasing. A `RES` binding is
**invalidated only by** this closed list:

1. the resource's **registered close op** (built-in or `LINK` `CLOSE`) and its
   re-export aliases — e.g. `fs::close(f)`;
2. **`thread::transfer`** of the resource (the resource handoff op, §7);
3. **`RETURN`** of the resource (move out to the caller);
4. **scope-drop** at the end of the binding's lexical scope (auto-close).

A borrow grants *use* (operate the handle, mutate `STATE`) but never the right to
*invalidate*: a callee that only borrowed a resource cannot close it, `RETURN`
it, or `thread::transfer` it — those events require ownership. Nothing else ends a
resource's life. There is **no per-function borrow/consume
inference** and there are no `BORROW`/`MOVE` annotations: a call either is one of
the four invalidating events or it borrows. This removes the footgun by
construction while remaining fully static (the four events are all
compiler-visible).

Worked example (the case that is impossible today):

```basic
RES f AS File = fs::open("app.db")
exec(f, "CREATE TABLE t (...)")   ' borrow — f still live
exec(f, "DROP TABLE t")           ' borrow — f still live
fs::close(f)                      ' registered close → f invalidated
exec(f, "...")                    ' COMPILE ERROR: f used after close
' (no scope-drop close — f already closed)
```

## 4. Resource State

Rather than wrapping a resource in a record (which would make the record
move-only — the contagion footgun, §3.2), a resource **carries its associated
data inside itself**, as `STATE`. The data lives where the uniqueness already is,
so nothing that was copyable ever becomes unique.

```basic
TYPE FileState        ' a normal, copyable data record
  pos AS Integer
  len AS Integer
END TYPE

RES s AS File STATE FileState = fs::open("app.db")  ' state default-initialized
s.state.pos = 10                                     ' owner mutates in place
LET p = s.state.pos                                  ' copy data out (it is copyable)

FUNC seek(RES s AS File STATE FileState, to AS Integer) AS Nothing
  s.state.pos = to
END FUNC
```

Rules:

- `STATE T` attaches a state of type `T` to the resource. `T` must be an ordinary
  **copyable data type** — and since no `Type` may contain a resource (§3.2), `T`
  is automatically resource-free. The state is **atomic** with the resource.
- `STATE` is optional. `RES f AS File = fs::open(...)` (no state) is fine; `STATE`
  is opt-in, and the state type is part of the resource binding's static type, so
  it rides through signatures (`FUNC seek(RES s AS File STATE FileState, …)`).
- The state is **owned by the resource** and lives alongside the handle in the LUT
  entry (§6). It is **auto-cleaned-up when the resource drops or is closed** — no
  user cleanup hook runs; it is just data that is freed.
- Access follows the borrow rule (§3.3). Both the **owner** and a **borrower** of
  `s` may read and **mutate `s.state` in place** and copy data out — the borrow is
  exclusive, so mutation is safe (no aliases). The difference is ownership, not
  access: only the owner may *invalidate* `s` (close / `RETURN` / `thread::transfer`);
  a borrower may use and mutate but not end it.
- The state default-initializes when the resource is produced; `T` must therefore
  be defaultable.
- `thread::transfer` moves the state with the resource; the state type must be
  thread-sendable for the resource to be transferable.

Because resources are atomic (§3.2), a resource's `STATE` can never itself contain
a resource. To work with several resources at once, hold several `RES` bindings —
there is no single value that owns more than one resource.

## 4a. Resource Unions

A union whose **every** variant is a resource is itself a resource — a *resource
union* — and is `RES`-bound like any other:

```basic
UNION Stream            ' every variant is a resource → Stream is a resource
  Tcp(Socket)
  Tls(TlsSocket)
  Local(File)
END UNION

RES s AS Stream = ...
read(s, buf)            ' borrow — dispatches to the active variant
' scope end → drop closes the active variant via its registered close op
```

- A resource union owns **exactly one** resource at a time (the active variant),
  so it is atomic — a *choice* among resources, not a bundle. It is move-only and
  cannot be copied.
- **Drop is tag-dispatched**: cleanup reads the union tag and calls the active
  variant's registered close op. Every variant has one (built-in or `LINK`), so
  this is a registry lookup, not new machinery.
- It follows the §3.3 borrow/invalidation rules unchanged; a borrow dispatches to
  the active variant.
- A union may **not mix** data and resource variants (§3.2).
- **Variants are bare resource types; a resource union carries no `STATE`.** `STATE`
  belongs to one concrete resource, but a union abstracts over *which* resource, so
  a union-level `u.state` is undefined — it would vary by tag and be absent for
  stateless variants. A variant is therefore a plain resource type (`File`,
  `Socket`), never `File STATE T`. If you need state, use a concrete stateful
  resource, not a union.

Open for v1 (§12): whether resource unions are implemented in v1, or the rule is
stated now and the capability deferred.

- The **registered close op consumes** (invalidates) the binding — this is one of
  the four invalidation events. There is exactly one close op per resource type
  (built-in symbol, or the `LINK` `CLOSE` function), plus any re-export aliases.
- **Manual close** (calling the close op early, e.g. `fs::close(f)`) invalidates
  the binding at **compile time** — reuse is a compile error, as today. The LUT
  entry (§6) persists marked-closed so the eventual scope-drop **safely no-ops**
  rather than double-closing.
- **Drop-close** (auto-close at scope exit) runs the close op on any still-live
  resource. Close failure during drop cannot become a source-level error; it is
  recorded in this plan's runtime **cleanup-failure ledger** (log-on-exit,
  structured to allow a later retry pass) and drained on exit. Explicit close
  auto-propagates its failure as a `Result`.
- **Runtime `ErrResourceClosed`** remains the backstop for the cases the compiler
  cannot statically track (use across an abstraction boundary), not the normal
  path.

User types never get a close op. Resources are the only values with declared
cleanup behavior; the cut source-defined-resource feature is **not** reintroduced
(see §8).

## 6. Runtime: the Resource LUT

Resources are tracked in a runtime handle table rather than as inline values.

Each live resource has a LUT entry, conceptually:

```
{ Resource Type, Pointer, State, Owner Scope }
```

- **Pointer** is the underlying native handle (fd / host pointer / `CPtr`), never
  exposed to source.
- **State** is the resource's owned `STATE` data payload (§4), or empty. It is
  freed when the entry is closed/removed — there is no nested resource ownership,
  because resources are atomic (§3.2) and `Type`s cannot contain resources.
- **Owner Scope** drives auto-close: it is the lexical scope of the owning `RES`
  binding. When that scope unwinds on any exit path, its still-live resources are
  closed in the existing lexical-drop order. Ownership is always a single `RES`
  binding — never a record or another resource.
- A `RES` binding holds a key into the table, not the raw pointer. The key cannot
  be copied (resources are non-copyable), so the single-owner invariant holds.

This representation gives natural homes for: close-exactly-once (entry removed on
close), use-after-close detection (entry marked closed), the cleanup-failure
ledger, and — most importantly — thread transfer.

## 7. Thread Transfer

There are **two planes** across a thread boundary, because resources cannot ride
the data channel.

**Data plane — `thread::send` / `thread::receive` / `thread::poll`.** The message
type `Msg` carries copyable data. `Msg` is routinely a record or union for
signaling/batching, and `thread::poll` reports readiness as data — none of which a
resource permits. So **`Msg` must be resource-free**: a resource (including a
resource union, §4a) cannot be a message. *(This changes today's rule that a
`File` is sendable via `thread::send`; see §9.)*

**Resource plane — `thread::transfer` / `thread::accept`.** A dedicated channel
moves a resource between threads:

- `thread::transfer(t, res)` — **moves** `res` to `t`'s incoming resource channel.
  This is invalidation event #2 (§3.3): the sender binding is invalidated and the
  resource's LUT entry leaves the sender's per-thread table.
- `thread::accept(worker)` — receives a transferred resource and binds it with
  `RES`. It blocks up to a timeout and is fallible, so it yields `Result OF
  <resource>` (auto-unwrapped) — `Result` is the one wrapper allowed to carry a
  resource.

Transfer semantics are the prior ones: move on success, ownership stays with the
sender on failure, an in-flight resource is owned by the runtime resource queue
and closed exactly once if never accepted. The channel is symmetric
(`ThreadWorker` ↔ `Thread`), like `send`/`receive`. Only thread-sendable resource
types may cross; sendability is a per-resource **registry** property (§10),
serialized via the `RESOURCE_TABLE` sendable bit, replacing the hardcoded list.
(How a *native* `LINK` resource opts in is `plan-link-update.md`'s concern.)

The resource channel is **typed** by an optional third thread type parameter,
`RES Res` — reusing the `RES` resource marker for consistency with the rest of the
model:

```basic
Thread OF Msg RES Res TO Out
ThreadWorker OF Msg RES Res TO Out
```

`thread::transfer(t, r AS Res)` is checked against `Res`, and
`thread::accept(worker) AS Result OF Res` yields it. `RES Res` is **optional** — a
data-only thread is just `Thread OF Msg TO Out`. A thread with only a resource
channel and no data channel is spelled `Thread OF RES Res TO Out` (the message
slot defaults to `Nothing`). To carry several resource kinds, make `Res` a
**resource union** (§4a); `transfer` moves "a `Res`", `accept` yields it, and you
match to the concrete handle. The names `transfer`/`accept` are **kept**.

**Implementation note (settled).** This is implemented end to end: the data plane
(`send`/`receive`) and the resource plane (`transfer`/`accept`) run on
**separate per-thread queues**, so a thread can carry both at once. The resource
type slot holds a **bare** resource (`RES File`); a resource's `STATE` is declared
on the binding (`RES f AS File STATE T = thread::accept(t)`), not in the thread
type, and the runtime moves the `STATE` payload with the resource across the
boundary. `Msg` is enforced **resource-free** at the type level — a resource in
the message slot is rejected and directed to the `RES` plane.

## 8. Out of Scope (explicitly)

- **Resource-bearing records / multi-resource values.** A record cannot contain a
  resource (§3.2), and no value owns more than one resource at a time. A pure-
  resource *union* is allowed (§4a) because it owns exactly one resource; a mixed
  union and any resource-bearing record are not. Own several resources at once by
  holding several `RES` bindings; data that belongs with a resource rides its
  `STATE` (§4).
- **User-declared move-only / linear *data* types.** Decided no: the only unique
  values are resources. `RES` stays resource-scoped; it is not a general "unique
  value" binding kind.
- **Custom drop/cleanup hooks for user types.** That is the source-defined
  resource feature already cut; reintroducing it would re-create it. `STATE` data
  is freed structurally on resource drop, with no user code.
- **Resources in ordinary collections.** Deferred: the cleanup model is
  per-binding lexical drop, not per-element, and collections have no per-element
  drop machinery. A future plan would need element-wise close at every
  drop/overwrite/remove site and per-slot ownership tracking.
- **Borrow as a value / reference type.** Borrow remains a call-scoped,
  un-nameable access mode, not a storable reference. No lifetimes, no
  `BORROW`/`MOVE` keywords.

## 9. Relationship to Other Specs

- `mfbasic.md` §14 gains `RES` as a third binding kind alongside `LET`/`MUT`, with
  the ownership axis described in §3.1 here. §15 (Resource Management) is rewritten
  to the fixed-invalidation-event model (§3.3) and the LUT (§6).
- `mfbasic.md` §16 and `standard_package.md`'s thread sections change: the message
  channel becomes **resource-free** (resources no longer cross via
  `thread::send`/`receive`), and `thread::transfer`/`thread::accept` (§7) are added.
  `File`'s "sendable via `thread::send`" becomes "transferable via
  `thread::transfer`". The thread-sendability metadata rule keeps unions sendable
  only when uniform (a data union of sendable types; a resource union is carried by
  the resource plane).
- `plan-link-update.md` **depends on this plan** and is implemented after it. Its
  resource-ownership sections (§5–§7, §11) defer here; it adds only what is
  specific to native `LINK` resources, including **how a new resource type is
  declared** — this plan establishes the resource *model* but not the declaration
  syntax. New resources are introduced at package scope with
  `[EXPORT] RESOURCE Name CLOSE BY closeFn` (`plan-link-update.md` §5), naming a
  native `LINK` close op; plan-link-update also owns the `CPtr` representation, the
  ABI surface, the close-op re-export (a function alias), and *writing* native
  entries into the registry/`RESOURCE_TABLE` that this plan establishes.
- A native `LINK` resource names its close op via `CLOSE BY`; that close op is the
  registered close (invalidation event #1), and a re-exported close op (a function
  alias) is an alias of it. From this plan's perspective a native resource is just
  another registry entry — same `RES`/LUT/four-event behavior as a built-in.

## 10. Implementation Impact

This is a breaking surface change (`RES` keyword) plus an internal model change.

- **Parser:** `RES` in binding, parameter, and return positions, with optional
  `STATE T`. Reject resources in `LET`/`MUT`, copyable values in `RES`, and
  **resource-typed fields in any `Type`**. One-namespace redefinition checks.
- **Resolver/typecheck:** classify bindings and types — a **record** with any
  resource field is rejected; a **union** is classified as a data type (all
  variants data), a resource union (§4a, all variants resources), or **rejected as
  mixed**. Tie a `STATE` type to a resource binding and type `s.state` access.
  Replace the non-copyable→`Transfer`
  argument rule with **borrow-by-default for resources**; track `RES`-binding
  liveness against the four invalidation events only (no per-function inference).
  Keep compile-time use-after-close/move. A borrower may read and **mutate**
  `s.state` (the borrow is exclusive) but may not invalidate the resource
  (close / `RETURN` / `thread::transfer` require ownership).
- **Codegen:** represent resources as LUT keys with an attached `STATE` payload;
  emit LUT insert (state default-init) on production, borrow (no transfer) on
  ordinary calls, invalidate+close on the four events (freeing the state),
  auto-close walk on scope unwind, `thread::transfer` as cross-table move (state
  moves with the entry). A resource union's drop **dispatches on the tag** to the
  active variant's registered close op. Manual close marks the entry closed; drop
  no-ops a closed entry.
- **Verifier:** enforce the four-event rule, single ownership, no resource copy,
  no resource field in a record, no mixed union, state-freed-on-close,
  resource-free message types, and thread-plane separation (`transfer`/`accept`
  for resources, `send`/`receive` for data). The `CPtr`/pointer-escape check is
  **deferred to `plan-link-update.md`**: `CPtr` is the hidden representation of a
  *native* `LINK` resource and does not exist in this plan's surface (built-in
  resources expose fds/host pointers only through the LUT, never as a source
  type), so there is nothing to leak until native resources are introduced. The
  escape check lands together with the `CPtr` type that it guards.
- **Registry (this plan's foundation, lands first):** replace hardcoded
  `builtins::is_resource_type` matching with a single dynamic table keyed by
  resolved type name, consulted at every call site. Each entry records the close
  op, sendable flag, close-may-fail, and kind (built-in / native / …). Seed it
  with the built-ins (`File`, `Socket`, `Listener`) and populate it from each
  imported package's `RESOURCE_TABLE`. Make `RESOURCE_TABLE` round-trip — consume
  it on import (it is currently written but never read) and add the `bit 2 =
  sendable to thread` flag (currently never written/read). Native `LINK` resources
  register into this table later (`plan-link-update.md`).

### Phases

**Status: all phases (0–7) implemented and covered by the acceptance suite.**
The data-plane message slot is enforced resource-free at the type level; the
resource plane (`transfer`/`accept`) runs on a dedicated per-thread queue with
its `STATE` payload moved across the boundary; and `s.state.field = value`
performs in-place single-field `STATE` mutation (§4). The only deliberately
deferred item is the `CPtr`/pointer-escape verifier check, which lands with
native `CPtr` in `plan-link-update.md` (Phase 6 note above).

0. **Resource registry & `RESOURCE_TABLE` round-trip** — the table-driven
   recognition above; rewrite the ~8 hardcoded call sites; consume the table on
   import; write/read the sendable bit. Everything else builds on this.
1. **`RES` syntax** — parse binding/parameter/return + optional `STATE`;
   binding-kind/namespace checks; reject resource fields in records.
2. **Ownership rules** — borrow-by-default (exclusive, mutable); four-event
   invalidation; compile-time use-after-close; `s.state` mutable by owner and
   borrower; only the owner may invalidate.
3. **LUT runtime** — handle table with state payload, production/borrow/close
   lowering, state default-init and free-on-close, manual-close-marks-entry.
4. **Resource unions (§4a)** — classify unions as data / resource / rejected-mixed;
   make an all-resource union a `RES` type; tag-dispatched drop to the active
   variant's registered close op; borrow dispatches to the active variant.
5. **Thread transfer** — per-thread LUTs; `thread::transfer`/`thread::accept` as
   the resource plane; the optional `RES Res` thread type parameter; resource-free
   `Msg` enforcement on `send`/`receive`.
6. **Verification** — four-event rule, no resource field in a record, no mixed
   union, resource-free messages, thread-plane separation. (`CPtr`-escape defers
   to `plan-link-update.md`, where `CPtr` is introduced.)
7. **Migration & tests** — see §11/§12.

## 11. Migration

`RES` is mandatory for resources, so existing programs that bind a `File`/`Socket`
with `LET`/`MUT` stop compiling. Options to decide (§13):

- a mechanical rewrite (`LET f = fs::open` → `RES f = fs::open`) the compiler can
  suggest via a diagnostic fix-it;
- a transition window where `LET`-binding a resource is a deprecation warning
  before becoming an error.

Acceptance tests must cover: borrow across multiple ops (the previously-impossible
case), manual close + use-after-close compile error, drop-close on every exit
path, return-moves-ownership, `STATE` read/mutate by both owner and borrower
(exclusive borrow) with the mutation visible to the owner after the call, `STATE`
auto-freed on resource drop, `thread::transfer` cross-table move (with state), plus
resource-union drop dispatching to the active variant's close op, and the
invalid cases (copy a resource, `LET` a resource, store a resource in a
collection, **a record with a resource-typed field**, **a mixed data/resource
union**, **a resource as a `thread::send` message**, and **invalidating a resource
through a borrow** — close / `RETURN` / `thread::transfer` of a borrowed handle).

## 12. Resolved Decisions

All open questions are now resolved and implemented:

1. **Return-position spelling — `AS RES File`.** The `RES` marker is **required** in
   a return position (`FUNC open(...) AS RES File`); ownership moves out to the
   caller (invalidation event #3). A returned resource's `STATE` rides on the
   caller's binding (`RES f AS File STATE T = open(...)`), not on the return type,
   mirroring the thread resource plane (§7).
2. **`Result OF` a resource — auto-unwrap.** A fallible producer such as
   `fs::open`/`fs::openFile` yields `Result OF <resource>`; `RES f = fs::open(...)`
   auto-unwraps to bind the live resource (or propagates the `Error`). `Result` is
   the one wrapper allowed to carry a resource (also used by `thread::accept`).
3. **`STATE` initialization — default-only for v1.** `T` must be defaultable; the
   `STATE` default-initializes when the resource is produced. There is no explicit
   initial-state literal; set fields afterward with in-place field assignment
   (`s.state.pos = 10`) or a whole-state `WITH` update. (No `STATE T[…]` form.)
4. **Migration policy — hard break.** `RES` is mandatory for resources; binding a
   resource with `LET`/`MUT` is an error (`TYPE_RESOURCE_REQUIRES_RES`) with a
   message pointing at `RES`. No deprecation window.
5. **Naming — keep `RES`.** The binding kind is "resource," not a general unique
   kind, so `RES` reads accurately.
6. **`thread::transfer`/`thread::accept` names — kept.**

Also folded into the body: resource unions are **implemented in v1** (§4a, Phase
4), carry **no `STATE`**; the thread resource channel is the optional **`RES Res`**
type parameter on a **dedicated per-thread queue** (§7); and `s.state.field =
value` is in-place single-field `STATE` mutation (§4).

## 13. Recommendation

Adopt the `RES` binding kind with the fixed four-event invalidation model, the
LUT runtime, and atomic resources that carry data-only `STATE`. It removes the
borrow/consume footgun *and* the ownership-contagion footgun by construction
(`Type`s never contain resources), keeps static safety and pure-data records, and
folds built-in, native, and imported resources into one uniform ownership model —
without a borrow checker, lifetimes, user linear types, or user cleanup hooks. The
deliberate trade is that resources are atomic: no value owns more than one
resource; bundle by holding multiple `RES` bindings.

Start with the registry & `RESOURCE_TABLE` round-trip (Phase 0) — it replaces the
hardcoded recognition every later phase relies on — then build the `RES` model on
top. Native `LINK` resources (`plan-link-update.md`) come afterward and slot into
this foundation.
