# bug-595: a `STATE` type name is never resolved — an unknown name gets an unlocated or misleading error

Last updated: 2026-09-12
Effort: small-to-medium
Severity: LOW — every shape is rejected (no miscompile found), but the diagnostic is unlocated or names the wrong problem
Class: Diagnostics

Status: **FIXED** — landed on main in `4dbf42c03` (fix `ee4d8b011`): every STATE position is now a located SYMBOL_UNKNOWN_TYPE
Regression Test: — (a `tests/syntax/resources/` invalid fixture per shape, once fixed)

## How it was found

The bug-472 man-example gate, run over the whole corpus on box 2223, failed
`http::finish` example 1. The example wrote `STATE PendingState` unqualified, which
stopped being legal when bug-480 Phase 4b (`363b85696`) required the package prefix
on an imported type. The example itself is corrected on the bug-472 branch. What
the compiler said about it is this bug.

## Reproduction (release binary at round-3 `d2e923967`, macOS)

Each of these is a one-file `mfb init` project; the error text is verbatim.

1. **An unqualified imported STATE: the right rejection, the wrong reason.**

   ```
   IMPORT net
   IMPORT http
   IMPORT io
   SUB main()
     RES s AS http::Stream STATE PendingState = http::startRead(net::toUrl("http://example.com/"))
     io::print("bound")
   END SUB
   ```
   → `TYPE_STATE_MISMATCH: binding `s` declares `STATE PendingState` but its initializer carries STATE `http.PendingState``

   The same mistake in a plain type position, `LET r AS Response = http::ok("x")`,
   gets `SYMBOL_UNKNOWN_TYPE: Type `Response` is not a built-in or top-level
   project type`. In the STATE position the name is never resolved, so the
   user is told about a mismatch instead of a missing `http::` prefix. The message
   also prints the internal dotted spelling `http.PendingState`, which is not
   writable source.

2. **A nonexistent STATE name gets a mismatch, not "unknown type".**
   `RES s AS http::Stream STATE Nonexistent = http::startRead(...)`
   → `TYPE_STATE_MISMATCH … declares `STATE Nonexistent` but its initializer carries STATE `http.PendingState``.

3. **A nonexistent STATE name at the attach point is UNLOCATED.**

   ```
   IMPORT fs
   IMPORT io
   SUB main()
     RES f AS fs::File STATE Nonexistent = fs::createTempFile()
     io::print("bound")
   END SUB
   ```
   → `error: TYPE_STATE_INVALID: binding `f` STATE type `Nonexistent` must be a copyable, defaultable data type.`

   There is no file, no line and no rule number. This is the bug-466 class of
   unlocated diagnostic. Adding `io::print(toString(f.state))` produces a located
   `TYPE_CALL_ARGUMENT_MISMATCH` on the read instead, with the unresolved name
   `Nonexistent` shown as if it were a type.

## The single correct behaviour

A `STATE T` clause resolves `T` exactly as every other type position does:
- an unknown name is `SYMBOL_UNKNOWN_TYPE`, located at the clause;
- an imported type written without its package prefix gets bug-480's
  "write `pkg::Name`" treatment;
- no message shows the internal `pkg.Name` spelling.

## Phase 1

1. Add the three shapes above as `tests/syntax/resources/*-invalid` fixtures; RED
   means the current wording.
2. Find where a `STATE` clause's type is parsed (`ParameterType::Stateful`) and why
   type-name resolution skips it. The `stateful` constructor note in `src/types.rs`
   and `check_binding_state_agreement` in `src/ir/verify/calls.rs` are where to
   start reading, not conclusions.
3. POSITIVE pin: a qualified imported STATE (`STATE http::PendingState`), a
   project-local STATE record, and a package-internal `STATE PendingState` inside
   `http`'s own helpers must all still build.
