# sec-01: `LINK` `FREE` block on an `AS RES` producer frees the live resource handle (UAF + double-free)

Last updated: 2026-07-23
Effort: small (<1h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: (to add) a syntaxcheck rejection fixture, e.g.
`tests/syntaxcheck/` `native-free-on-resource-producer`, plus an `ir::verify`
twin.

A native `LINK` function that both **produces a resource** (`... AS RES <T>`,
setting `return_resource`) and carries a **`FREE` block** compiles cleanly but
emits a thunk that calls the native deallocator on the very handle it just
stored into the live resource record. The handle in the returned resource's
`FD@0` is freed before the wrapper returns, so:

- every later use of the resource (`exec`/`query`/`.state`/any package call that
  passes `FD@0` to the C library) is a **use-after-free**, and
- the scope-drop `CLOSE BY <fn>` at end of the resource's lifetime passes the
  same already-freed handle to the native close function — a **double-free**
  into the native allocator (heap corruption, likely crash or exploitable).

The single correct behavior a fix produces: the compiler must **reject** the
combination `FREE` + `AS RES` at declaration time (`FREE` is defined only for a
caller-owned value that is *copied out* — a `String`/buffer — not for a handle
the wrapper keeps alive by pointer), OR codegen must skip the `FREE` deallocation
when `return_resource` is set. Rejecting at the frontend is preferred: the
combination is semantically contradictory, so silently "fixing" it in codegen
would hide an author error.

This is reachable only via a package/binding declaration (not arbitrary end-user
MFBASIC code), but a package is often third-party, compiles without warning, and
its clean compile is a false guarantee of memory safety for every app that links
it. No package shipped in-tree currently trips it; the sqlite `FREE` fixture uses
`FREE` correctly on a `String`-returning function, which is why this has stayed
latent.

References:

- `src/docs/man/link/package.md` §"FREE" — "`FREE return` runs a declared native
  deallocator after a successful copy from a … return"; `FREE` is documented as a
  *copy-then-free* mechanism, incompatible with a kept-alive resource handle.
- `src/docs/spec/language/17_native-libraries.md` — `AS RES` producer / `FREE`
  clause semantics.
- Memory: `[[link-thunk-never-reclaims-the-record]]` (the redundant 2nd close is
  load-bearing — but that invariant assumes the first "release" was a CLOSED_BIT
  set, NOT an actual `free`; this bug violates that assumption).
- Found during the 2026-07-23 runtime security audit (arena/lifetime sweep).

## Failing Reproduction

A minimal binding package accepted by `syntaxcheck` and `ir::verify` today. The
producer's handle is the ABI **return** itself (so `return_ctype == "CPtr"`,
`return_name` names it, and `RETURN` surfaces it — the exact conditions the
`FREE` validator at `src/syntaxcheck/link.rs:732-746` requires), *and* it is
declared `AS RES`:

```
RESOURCE Db CLOSE BY foo::free

LINK "libfoo" AS foo
  FUNC open(path AS String) AS RES Db
    SYMBOL "foo_open"
    ABI (path CString) AS handle CPtr
    RETURN handle
    FREE handle
      SYMBOL "foo_free"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
  FUNC free(RES db AS Db) AS Nothing
    SYMBOL "foo_free"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main() AS Integer
  RES db AS Db = foo::open("x")   ' foo_open's handle is foo_free'd inside the thunk
  ' ... any use of db here reads freed FD@0 (UAF)
  RETURN 0                        ' scope-drop CLOSE BY foo::free → double-free
END FUNC
```

- Observed: compiles without diagnostic; the emitted `open` thunk frees the
  live handle; first use of `db` is a UAF; end-of-scope close is a double-free.
- Expected: `syntaxcheck` emits a `NATIVE_FREE_INVALID`-class diagnostic
  rejecting `FREE` on a `return_resource` function (or codegen omits the free).

Contrast case that is correct today and must stay accepted:
`tests/rt-behavior/native/native-link-free-rt/src/main.mfb` — `expandedSql(...)
AS String` with `FREE text`. Here the return type is `String` (not `AS RES`),
`return_resource == false`, and the wrapper genuinely copies the C string into an
owned `String` before freeing the original. `FREE` is correct there; only the
`AS RES` combination is the bug.

## Root Cause

The `FREE` deallocation and the `return_resource` wrapping are emitted
independently in the thunk, and no validator forbids their coexistence.

Codegen, `src/target/shared/code/link_thunk.rs`:

- `:1321-1396` (`if function.return_resource`) — the native handle, currently in
  `RESULT_VALUE_REGISTER` (loaded from `CRET_OFF` by the CPtr `emit_return_passthrough`
  arm), is parked, an 80-byte resource record is arena-allocated, and the handle
  is stored into the record at `FILE_OFFSET_FD` (`:1334`). `RESULT_VALUE_REGISTER`
  then becomes the record pointer (`:1393-1394`). The record is now a **live**
  resource whose `FD@0` is the handle.
- `:1404-1416` (`if let Some(free_slot)`) — reloads the **same** handle from
  `CRET_OFF` (`:1407`) into arg0 and `blr`s the deallocator (`:1413`). This frees
  the handle that `:1334` just stored live into `FD@0`.

Missing guards:

- `src/syntaxcheck/link.rs:726-761` (`NATIVE_FREE_INVALID`) validates the `FREE`
  block's slot/return-ctype/deallocator-signature but never inspects
  `function.return_resource`. A `CPtr`-returning `AS RES` producer with a
  well-formed `FREE` block passes.
- `src/ir/verify/link.rs:466-476` re-checks only that `free.symbol` is non-empty.
- The other `return_resource` sites (`src/ir/verify/link.rs:377,500,573`,
  `src/target/shared/code/validation.rs:324`) register resource names / validate
  STATE types; none cross-checks `FREE`.

The contrast case is immune because its return type is `String`, so
`return_resource == false`: the `:1321` block never runs, the value in
`RESULT_VALUE_REGISTER` is an independent owned `String` copy, and freeing the
original `CRET_OFF` pointer is exactly correct.

## Goal

- A binding package that declares `FREE` on an `AS RES <T>` producer is rejected
  at compile time with a clear diagnostic (or, if the alternative is chosen, the
  thunk provably does not free the handle it stored into `FD@0`).
- The existing correct `FREE`-on-`String` fixture still compiles and runs
  (thousands of copy-then-free cycles, no corruption).

### Non-goals (must NOT change)

- The semantics/ABI of a correct `FREE` block on a copied-out `String`/buffer
  return (the sqlite `expandedSql` path) — must stay byte-for-byte identical.
- The `return_resource` record layout (`FD@0`/`CLOSED@8`/`STATE@16`) and the
  `CLOSE BY` scope-drop path.
- Do NOT "fix" this by weakening the `native-link-free-rt` runtime test — it
  exercises the *correct* `FREE` path and is not the broken case.

## Blast Radius

- `link_thunk.rs:1321-1416` (`emit_native_link_thunk` resource-wrap + FREE) —
  the site that emits the bad free; fixed by this bug (guard makes it
  unreachable, or skip the free when `return_resource`).
- `src/syntaxcheck/link.rs:726-761` — the natural place to add the rejection
  (`&& !function.return_resource`, or a dedicated diagnostic).
- `src/ir/verify/link.rs:466-476` — add the twin check so a decoded/binary
  package (which skips syntaxcheck) is also rejected. **Important:** packages are
  distributed as compiled `.mfp`; a malformed one reaching a consumer bypasses
  `syntaxcheck`, so the `ir::verify` guard is load-bearing, not redundant.
- All other `AS RES` producers in-tree (sqlite `open`/`prepare`, net/fs/audio
  builtins) — unaffected: none declares a `FREE` block, and the sqlite producers
  return the handle via an `OUT` slot with a `CInt32` ABI return, which already
  fails the `FREE` validator's `return_ctype == "CPtr"` gate.

## Fix Design

Preferred: reject at the frontend. In `src/syntaxcheck/link.rs`, extend the
`FREE` validation (around `:736`) so a `FREE` block on a function with
`return_resource == true` reports `NATIVE_FREE_INVALID` (message: a resource
producer keeps the native handle alive in its record and must not free it).
Mirror the rejection in `src/ir/verify/link.rs` so binary-package consumers are
covered. The codegen path at `link_thunk.rs:1404` then becomes unreachable for
`return_resource` functions; optionally add a `debug_assert!(!function.return_resource)`
there to lock the invariant.

Rejected alternative — silently skip the `FREE` when `return_resource`
(`if free_slot.is_some() && !function.return_resource`): this makes a
contradictory declaration compile to *something*, hiding the author's error and
leaving a resource whose handle the author believes is freed. Rejecting is
safer and matches how `FREE` is documented (copy-then-free only).

The change shifts no generated output for any currently-valid package (none
combines the two features), so no goldens move; only new negative-diagnostic
fixtures are added.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a `syntaxcheck` fixture: the repro package above must produce a
      `NATIVE_FREE_INVALID` (or new code) diagnostic. Confirm it currently
      compiles clean (the failing state).
- [ ] Add an `ir::verify` unit test asserting the same rejection on the lowered
      `LinkFunction` (decoded-package path).
- [ ] Confirm the audit list above is complete: grep every `return_resource` and
      every `function.free` site; verify none other emits a free of a kept handle.

Acceptance: both new tests fail today (package compiles / verifier accepts); the
audit list has a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] `src/syntaxcheck/link.rs` — reject `FREE` when `return_resource`.
- [ ] `src/ir/verify/link.rs` — mirror the rejection.
- [ ] (optional) `link_thunk.rs:1404` — `debug_assert!(!function.return_resource)`.

Acceptance: Phase 1 tests pass; `native-link-free-rt` still compiles and runs
clean; no golden output moves.
Commit: —

### Phase 3 — validation

- [ ] Run the full syntaxcheck + ir::verify suites and the native-link runtime
      test.
- [ ] Confirm no `.mfp`/golden deltas (no valid package used the combination).

Acceptance: full suite green; zero unintended output changes.
Commit: —

## Validation Plan

- Regression test(s): the new syntaxcheck rejection fixture + the `ir::verify`
  twin (both fail-then-pass).
- Runtime proof: `native-link-free-rt` still passes (correct `FREE` path intact);
  a hand-built `AS RES` + `FREE` package now fails to compile instead of emitting
  a thunk that double-frees.
- Doc sync: note in `src/docs/man/link/package.md` that `FREE` is invalid on an
  `AS RES` producer.
- Full suite: the project's syntaxcheck/ir acceptance gates.

## Summary

The engineering risk is entirely in the guard placement: the fix is a two-line
rejection in `syntaxcheck` + `ir::verify`, but it MUST be added in **both** so a
precompiled `.mfp` package cannot smuggle the combination past the source-level
check. No valid program changes behavior; the only observable delta is that a
contradictory (and memory-unsafe) declaration is now rejected instead of
silently producing use-after-free / double-free code.
