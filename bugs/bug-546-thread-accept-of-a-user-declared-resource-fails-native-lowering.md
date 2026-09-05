# bug-546: `thread::accept` of a user-declared `THREAD_SENDABLE` resource fails native lowering

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet.

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

**Recommended next step:** determine empirically what record a user-declared
`RESOURCE` allocates (canonical `RESOURCE_RECORD_SIZE` with the closed flag at
+16, or something the `LINK` block shapes), then pick the arm that follows from
it. Until that is answered this is not a 1-2h fix, and the effort estimate above
is optimistic.

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
