# bug-808: `http::finish` re-raises whatever code is sitting in `state.err`, including codes outside its own Errors table

Last updated: 2026-10-04

Effort: medium (1h–3h)

Severity: MEDIUM

Class: Correctness

Status: Open
Regression Test: proposed `tests/runtime/rt_http_finish_state_err_validation.rs`

`http::finish`'s own man page and registry `errors` list promise a closed set of
eleven codes (`ErrOutOfMemory`, `ErrResourceClosed`, `ErrResourceMoved`,
`ErrIndexOutOfRange`, `ErrInvalidArgument`, `ErrInvalidFormat`, `ErrNotFound`,
`ErrTimeout`, `ErrOverflow`, `ErrNetworkFailed`, `ErrTlsFailed`) for "If a
transport failure was recorded into `stream`'s STATE". The implementation does
not check that `state.err` actually holds one of those codes — or any valid
error code at all — before `FAIL`ing with it. `http::PendingState.err` is a
plain, caller-writable `Integer` field (`RecordProp` with no access control), so
any program that reaches into `s.state.err` (directly, or via a future internal
bug that stores the wrong value there) makes `http::finish` raise an arbitrary
code under a message that claims "http stream transport failed" — breaking the
documented closed contract a `TRAP(e) ... e.code` dispatch relies on.

References:

- `src/codegen/builtins/http/func_finish.rs:10-29` (`DESC`) — the documented
  closed set of eleven codes `finish` promises to raise.
- `src/codegen/builtins/http/func_finish.rs:68` — the registry `errors` list,
  the same eleven codes, checked by the man page's Errors table.
- `src/codegen/builtins/http/mod.rs:260-282` — `PendingState`'s `err` field:
  `ParameterType::Integer`, "0 = ok; else a captured transport failure code",
  no range or domain validation anywhere.
- Found during bug-807 follow-up work (bug-staging-7) reproducing findings
  reported against `http`/`canvas`/`audio`/`csv`/`mfb audit`.

## Failing Reproduction

From the repository root, after building the release binary:

```sh
mkdir -p /tmp/mfb-f1/src
cat > /tmp/mfb-f1/project.json <<'JSON'
{"name": "f1", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-f1/src/main.mfb <<'BASIC'
IMPORT net
IMPORT http
IMPORT io

FUNC main() AS Nothing
  RES s AS http::Stream STATE http::PendingState = http::startRead(net::toUrl("http://example.com/"))
  s.state.err = 77050021
  LET r AS http::Response = http::finish(s) TRAP(e)
    io::print("raised " & toString(e.code) & " msg=" & e.message)
    RETURN
  END TRAP
  io::print("no error: " & toString(r.status))
END FUNC
BASIC
target/release/mfb build /tmp/mfb-f1
/tmp/mfb-f1/build/f1.out
```

- Observed (macos-aarch64, `bug-staging-7` release build at `/tmp/bs7-target/release/mfb`):

  ```
  raised 77050021 msg=http stream transport failed
  ```

  `77050021` is `ErrBadPixelCount` — a `canvas` package error, nowhere in
  `finish`'s own Errors table. A second run with `s.state.err = 123456789` (not
  a valid error code in `errorCode` at all) raises that same unassigned number
  with the identical message, equally uncaught by anything matching a real
  code:

  ```
  raised 123456789 msg=http stream transport failed
  ```

- Expected: `finish` raises only a code from its own documented Errors table, or
  rejects a `state.err` outside that set with a defined, documented failure
  (e.g. `ErrInvalidArgument`) rather than propagating it verbatim.

## Root Cause

`__http_finish`'s body (`src/codegen/builtins/http/func_finish.rs:54-60`):

```
FUNC __http_finish(RES s AS Stream STATE PendingState) AS Response
  IF s.state.err <> 0 THEN
    FAIL error(s.state.err, "http stream transport failed")
  END IF
  RETURN __http_parseResponse(s.state.raw)
END FUNC
```

`error(code, message)` is `general::error` (`src/codegen/builtins/general/func_error.rs`),
which is deliberately unchecked — "Building an `Error` does not raise it — it
just makes the value, and accepts any `code` and `message`" — because it is the
sanctioned escape hatch for *user* `FAIL error(...)` statements naming any code
they choose. `__http_finish` reuses that same unchecked constructor on a value
it did not just validate: `state.err` is set only by `__http_pump`
(`func_pump.rs:68,74`) and `__http_waitReadable` (`helper_wait_readable.rs:27`)
today, both of which only ever store one of the eleven documented codes — but
nothing enforces that invariant at the point `finish` reads it back, and
`state.err` is a plain record field a program can overwrite directly (shown
above) because `PendingState` carries no access control on any field.

## Goal

`http::finish` raises only a code from its own documented Errors table for any
reachable `state.err`. Either (a) validate `state.err` against that closed set
before `FAIL`ing with it, raising a documented code (e.g.
`ErrInvalidArgument`) for anything else, or (b) make `state.err` not
independently writable by ordinary program code, so the only values `finish`
ever sees are the ones `__http_pump`/`__http_waitReadable` store. Either fix
must not change `finish`'s behavior for the two call sites that set `state.err`
today.

### Non-goals (must NOT change)

- No change to the eleven documented codes `finish` raises for an actual
  transport failure.
- No change to `general::error`'s own deliberately permissive contract — it is
  correct for ordinary user `FAIL error(...)` statements; the fix belongs in
  `__http_finish` (or in how `state.err` is written/read), not in `error`.

## Blast Radius

- `http::finish` (`src/codegen/builtins/http/func_finish.rs`) — the fix site.
- `http::PendingState.err` readers/writers: `__http_pump`
  (`func_pump.rs:68,74`), `__http_waitReadable` (`helper_wait_readable.rs:27`),
  `__http_done` (`func_done.rs:44`), `__http_ready` (`func_ready.rs`) — audit
  whether any other reader also trusts `state.err` unchecked.
- Any program that reads `s.state.err` directly today (measured: none in
  `tests/` or `examples/` via `grep -rn "state.err" tests/ examples/` reporting
  only the `http` builtin sources themselves) — low external blast radius.

## Phases

### Phase 1 — failing test

Add `rt_http_finish_state_err_validation.rs`: write `s.state.err` to an
in-table code (expect that code back, unchanged behavior) and to an
out-of-table code (currently raises verbatim; pin the fixed behavior once
chosen). Confirm red against the documented contract.

### Phase 2 — the fix

Implement option (a) or (b) from Goal; update `func_finish.rs`'s `DESC`/`errors`
only if the fix adds a new documented code (e.g. `ErrInvalidArgument` for an
out-of-table `state.err`).

### Phase 3 — validation

`cargo test --release --test rt_http_finish_state_err_validation`; re-run both
reproduction cases above; `scripts/man-run-examples.sh http --run`.
