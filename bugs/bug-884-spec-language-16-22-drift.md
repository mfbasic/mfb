# bug-884: language spec §16–§22 drift — dangling citations, incomplete surface lists, and grammar/test-framework claims the compiler contradicts

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none (doc); citation sweep

Doc-only defects found while exercising `mfb spec language threads … test-framework`. Each item is
a SPEC bug against correct-looking code unless marked (CODE)/(UNCLEAR). Every item was measured
with the command or program named.

References:

- src/docs/spec/language/16_threads.md, 17_native-libraries.md, 18_builtin-functions.md, 19_grammar.md, 22_test-framework.md.
- Found during B05 hardening. Not covered by bug-840 / bug-857 / bug-873 (checked by grep).

## Items

### §16 threads
1. Dangling citation `[[src/ir/verify/resources.rs:is_thread_sendable]]`: the symbol exists only in a
   comment ("plan-114-A: this replaced the boolean `is_thread_sendable`"); the implementation is
   `thread_unsendable_cause` / `require_thread_sendable` (`grep -n is_thread_sendable src/ir/verify/resources.rs`).
2. "`process::Process`, the `audio` streams and `canvas::Image` are not [sendable]" omits
   `canvas::Font`, which is `sendable: false` (`src/codegen/builtins/canvas/mod.rs`, second resource
   row) and is rejected `TYPE_THREAD_NOT_SENDABLE` on a RES plane (measured: `ThreadWorker OF RES canvas::Font TO Integer`
   under `mfb build -app`). `mfb man thread` lists all five.
3. "The `thread` package exposes:" omits `thread::openStdIn` / `thread::closeStdIn`, which are callable
   (`thread::openStdIn()` compiles), have man pages, and are what `ErrInvalidContext` (02_error-codes)
   refers to (measured: `io::readLine(0)` in a worker without it → `77050019`).
4. "When a thread ends, its inbound queue is closed and further parent-side sends fail" — the code is
   `ErrInterrupted` (77050009, measured `thread::send` after `waitFor`), which the spec otherwise
   reserves for cancellation; state the code.

### §17 native libraries
5. "Naming one [a CSTRUCT] in a wrapper's MFBASIC-facing signature is `NATIVE_CSTRUCT_ESCAPE`" — on
   the source path the wrapper parameter `v AS S` is `SYMBOL_UNKNOWN_TYPE` ("Type `S` is not a
   built-in or top-level project type"); `NATIVE_CSTRUCT_ESCAPE` fires only for decoded packages
   (`src/ir/verify/link.rs:check_cstruct_escape` comment: "On the source path the resolver usually
   catches this first"). Same for `libc::S` outside the block (`SYMBOL_UNKNOWN_IDENTIFIER`).
6. "partial coverage is rejected (`NATIVE_STRUCT_FIELD_MISMATCH`)" — only when the CSTRUCT is used in
   an ABI slot; an unused `CSTRUCT S AS R` covering one of R's two fields builds (measured). (UNCLEAR)
   **Owner decision (2026-10-05):** reject always. Partial coverage is `NATIVE_STRUCT_FIELD_MISMATCH` at the `CSTRUCT` declaration, whether or not it is used (CODE change).
7. "Diagnostics specific to native bindings use codes `1-102-0008`…`0009`, `2-203-0089`…`0098`, and
   `2-203-0123`" — incomplete: `2-203-0099` (`NATIVE_FREE_INVALID`), `0114`–`0128`, `0130`, `0132`
   (`NATIVE_LIBRARY_*`, `NATIVE_CSTRUCT_*`, `NATIVE_BIND_*`, `NATIVE_BUFFER_INVALID`) are also native.

### §18 built-in functions
8. Dangling citations `[[src/codegen/builtins/math/mod.rs:is_math_call]]` and `[[…:is_math_constant]]`:
   both functions were deleted; the names survive only in module comments ("`is_math_call` is
   `owning_package == "math"`").
9. "the compile-time constants `math::pi`, `math::piFixed`, `math::e`, `math::eFixed`" — the package
   registers 14 (`CONSTANTS`, "seven `Float` and seven `Fixed`").
10. §18.3 enumerates the overridable names as "`toString`, `len`, `typeName`, the `to*` conversions
    (`toInt`, `toFloat`, `toFixed`, `toByte`), and the `is*` predicates" — `toMoney` and `toScalar` are
    overridable too (`is_overridable` = every general name but `error`, and §18.1 says "All eighteen
    except `error`").
11. §18.4 "Every built-in that can **wait** takes an optional trailing `timeoutMs`" — `tcp::read`,
    `tls::read` and `udp::receive` take none (`mfb man tcp read`: `tcp::read(sock, maxBytes)`), yet
    are in the "Conforming functions" list; the producing-call bullet's "(all under a socket
    read/write timeout)" is the true rule. Reword the opening sentence or the list.

### §19 grammar
12. "The parser restricts the head to a bare ident or qualifiedIdent — only `f(...)` and `T[...]` are
    accepted, not `(expr)(...)`" — `(f)(1)` with `LET f = LAMBDA(a AS Integer) -> a` is accepted and
    prints `1`; `(mk())(4)` is rejected. A parenthesized bare identifier collapses to the identifier.
13. `lambda` comment "the `ident = expr` body mutates a captured MUT binding" — a lambda capturing a
    `MUT` is `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` except in the `collections::forEach` non-escaping
    position (06_functions); say so here.
14. `docBlock` comment "anything else makes the lexer roll back" — `DOC FUNC answer` on one line is
    *not* rolled back: the words are taken as header-line attributes and the next line is reported
    `DOC_BAD_HEADER` ("`DESC` is not a valid DOC header"); `DOC FUNC add(Integer, Integer)` does roll
    back and yields a cascade of `MFB_PARSE_UNEXPECTED_STATEMENT`. (UNCLEAR: a targeted "header goes on
    the next line" diagnostic would serve better.)
   **Owner decision (2026-10-05):** a targeted diagnostic (CODE change). A DOC header and its body on one line give one located error saying the header goes on its own line, with the fix shown and no `MFB_PARSE_UNEXPECTED_STATEMENT` cascade. The same goes for the `DOC FUNC answer` case that today reports `DOC_BAD_HEADER` on the next line.

### §22 test framework
15. "printable for the failure message: a scalar, `String`, `Byte`, or `List OF Byte`" — `List OF Byte`
    can never pass the comparability requirement (no `List` is comparable, 12_collections), so
    `expectNEqual([toByte(1)], [toByte(2)])` is always `TESTING_EXPECT_INCOMPARABLE` (measured); the
    printable clause is unreachable for it.
16. (CODE) Several `TESTING_*` rejections are accompanied by a second error from the desugared
    comparison: `expectTrap(toInt("x"), "code")` → also `TYPE_BINARY_OPERATOR_MISMATCH`;
    `expectEqual("a", 1)` → also `TYPE_BINARY_OPERATOR_MISMATCH`; `expectNEqual` on lists → also
    `TYPE_REQUIRES_COMPARABLE`, pointing at user line numbers for code the user did not write.
17. "`mfb test --coverage` … emits `coverage.html`" — it also leaves `coverage.covdata`,
    `coverage.covfail` and `coverage.covmap.json` in the project directory (measured `ls`); document
    or clean them.
   **Owner decision (2026-10-05):** move the intermediates (`coverage.covdata`, `.covfail`, `.covmap.json`) under the build output directory (CODE change). Only `coverage.html` is the documented project-level artifact.

## Goal

- Each item above corrected in the spec (or, for 16, the duplicate companion diagnostic suppressed).

## Blast Radius

- `mfb man` thread/general pages already list the omitted members (2, 3) — spec-only fix.
