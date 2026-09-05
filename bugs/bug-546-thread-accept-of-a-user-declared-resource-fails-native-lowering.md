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

## Root Cause (not yet confirmed)

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
