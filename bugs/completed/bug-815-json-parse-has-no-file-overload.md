# bug-815: `json::parse` has no `fs::File` overload

Last updated: 2026-10-04
Effort: small–medium (guess)
Severity: LOW
Class: API gap

Status: Fixed — see Resolution
Regression Test: `tests/runtime/rt_json_parse_file.rs` (both overloads over a
real on-disk file and an `fs::openData` file, including a malformed document,
a closed file, a non-UTF-8 file, and reading from a non-zero starting
position).

## Problem

`json::parse` takes only text:

1. `json::parse(value AS String) AS json::Json`
2. `json::parse(value AS String, reviver AS FUNC(String, json::Json) AS json::Json) AS json::Json`

A program holding an open file must read it fully into a `String` first
(`fs::readAll`) and then parse. `csv::parse` is gaining an `fs::File` overload in
bug-staging-8 (bug-809), and `fs::openData` there makes an in-memory value
available as an `fs::File`. `json` should accept a file the same way.

## Goal

Add, with the owner-specified signatures:

```
json::parse(RES f AS fs::File) AS json::Json
json::parse(RES f AS fs::File, reviver AS FUNC(String, json::Json) AS json::Json) AS json::Json
```

- Parses one complete JSON document from the file's current position to end of
  input. The result, every error and the reviver behave exactly as the `String`
  overloads do for the same text.
- Uses the caller's open `File`: it does not close it. The file's position ends at
  end of input. A closed or moved file raises the usual resource error.
- Declares exactly the errors it raises: the `String` overloads' set, plus the file
  read errors it can reach. The bug-698/807 declared-errors guards must stay green.
- Man page and spec (`stdlib` json topic) updated. Works on `fs::openData` files once
  bug-809 lands.

## Phases

- [x] RED test: both overloads over a real file and an `openData` file, including a
      malformed document (same error as `String`) and a closed file.
- [x] Implement, ideally sharing the `String` overloads' parser rather than copying it.
- [x] Docs, guards, targeted fixtures; goldens in the staging-8 gate.

## Resolution

Fixed on `bug-staging-8`, after bug-809 landed `fs::openData`/`fs::readAll`.
`json::parse` gained two overloads, in
`src/codegen/builtins/json/func_parse.rs`:

```
json::parse(value AS fs::File) AS json::Json
json::parse(value AS fs::File, reviver AS FUNC(String, json::Json) AS json::Json) AS json::Json
```

**Parameter name.** The goal's sketch named the parameter `f`, and bug-809
named its own File parameter `file` (`csv::readRow(file AS fs::File, ...)`,
which has no `String` sibling to match). `json::parse` is different from
`readRow`: it is the *same* function, overloaded, with an existing `String`
form already named `value`/`text`. `csv::parse`'s own File overload (landed by
bug-809, in the same tree) kept its File parameter named `value` rather than
renaming it to `file`, to match its `String` sibling by the same function
name rather than by type — so `json::parse` follows that precedent and also
calls it `value`, not `file`.

**Implementation — one parser, shared, not duplicated.** Unlike `csv::parse`,
which shares one *tokenizer* between a String and a File input because
`csv::readRow` needs to stream a record at a time, `json` has no streaming
sibling: a JSON document is always built as one whole tree. There was
therefore nothing to gain by threading a `File` through the parser itself.
Instead, both new overloads read the file's remaining text with `fs::readAll`
(one decode pass, exactly `fs::readAll`'s own contract: from the file's
current position to its end) and hand that `String` straight to the
pre-existing, byte-for-byte UNCHANGED `__json_parse`/`__json_parseRevive`
bodies:

```
FUNC __json_parseFile(RES value AS fs::File) AS Json
  LET text AS String = fs::readAll(value)
  RETURN __json_parse(text)
END FUNC

FUNC __json_parseFileRevive(RES value AS fs::File, reviver AS FUNC(String, Json) AS Json) AS Json
  LET text AS String = fs::readAll(value)
  RETURN __json_parseRevive(text, reviver)
END FUNC
```

Because `__json_parse`'s own source is untouched, the `String` overloads'
generated code, and therefore their speed, is unaffected by this change —
there is no 6–9% "full round-trip" cost to avoid here the way bug-809 had to
avoid one for `csv::parse(String)`: there was never a round trip through a
`File` for `json`'s `String` form, only an added sibling pair of functions
elsewhere in the package.

**Errors.** The File overloads declare the `String` overloads' set
(`ErrInvalidFormat`, `ErrInvalidSurrogate`, `ErrDepthExceeded`, `ErrOverflow`,
`ErrOutOfMemory`) plus `fs::readAll`'s own (`ErrEncoding`, `ErrReadFailed`,
`ErrResourceClosed`, `ErrResourceMoved`). The `mfb_body_declared_errors` guard
(`cargo test --release --bin mfb declared_errors`) passed with no change to
its false-positive allowlist — the File bodies' only calls are `fs::readAll`
(trusted via its own descriptor) and `__json_parse`/`__json_parseRevive`
(already covered by the existing `("json", "parse", ...)`
`UNREACHABLE_VIA_CALLEE` entries, which apply per-member across all of
`parse`'s implementations).

**Behavior, verified by `tests/runtime/rt_json_parse_file.rs` (6 tests, all
green):** a real disk file and an `fs::openData` file each parse to the same
result as the equivalent `String`, including through the reviver; a malformed
document raises the same code (`77050003`) through either overload; a closed
file raises `ErrResourceClosed` (`77030004`); a non-UTF-8 file raises
`ErrEncoding` (`77020004`); parsing starts at the file's current position
(after an `fs::readLine`, say), not its start; and the file ends at end of
input (`fs::eof` true) and is left open in every case.

RED-first: the same six tests, run against the registry before this change
(`json::parse(f)` with `f` an `fs::File`), fail to COMPILE — `TYPE_CALL_ARGUMENT_MISMATCH`,
"expected String or String, FUNC(String, Json) AS Json" — confirming the
overloads did not exist.

**Timing (macos-aarch64, release build, `/usr/bin/time -p`, best-of-4 excluding
a cold first run, 15 in-process repetitions of `json::parse` over a 300,000-element,
~2 MB numeric array per run, same process so build/startup cost is amortized
away):**

| | before | after |
| --- | --- | --- |
| wall time | 1.36 s | 1.38 s |

Within run-to-run noise; the `String` overload's body is unchanged source, so
this is the expected result rather than a result that needed tuning.

**Docs.** `mfb man json parse` renders all four overloads, their four
`value`/`reviver` parameter rows, and the merged Errors table (new codes shown
against overloads 2/4); `scripts/man-census.sh --memory-scope json` and
`--scope json` both report 0 unclassified hits. `src/docs/spec/stdlib/04_json.md`
gained a "File input: `parse(File)`" subsection explaining the shared-parser
design and contrasting it with `csv::parse`'s shared-tokenizer design.

**Expected golden moves** (not regenerated here, per the staging gate's
targeted-test discipline — `scripts/test-accept.sh` was run scoped to `json*`
fixtures only):

- `tests/byte-identity/json/golden/json_codegen_cover_rt.ir` (new
  `#json_parseFile`/`#json_parseFileRevive` function blocks; `.ast` unaffected)
  and its five `*.ncodesum` siblings (not regenerated; cross-target codegen
  was not run in this pass).
- `tests/rt-behavior/json/json-behavior/golden/json_behavior.ir`
- `tests/rt-behavior/json/json-number-rendering-rt/golden/json_number_rendering_rt.ir`
- `tests/rt-behavior/json/json-number-roundtrip-rt/golden/json_number_roundtrip_rt.ir`
- `tests/rt-behavior/json/json-parse-deep-nesting-rt/golden/json_parse_deep_nesting_rt.ir`
- `tests/rt-behavior/json/json-parse-deep-scalar-scan-rt/golden/json_parse_deep_scalar_scan_rt.ir`
- `tests/rt-error/json/func_json_stringify_invalid_runtime/golden/func_json_stringify_invalid_runtime.ir`

  (all seven: the assembled `json` package source now carries two more
  functions, shifting every subsequent line number the `.ir` dump records —
  confirmed by diffing each against its golden: every changed line is a
  `"line": N` renumbering or a wholly new `#json_parseFile`/
  `#json_parseFileRevive` block; no existing function's logic differs.)
- `tests/syntax/json/func_json_parse_invalid/golden/build.log` (the
  `TYPE_CALL_ARGUMENT_MISMATCH` message's "expected ..." clause now lists all
  four overloads: `expected String or File or String, FUNC(String, Json) AS
  Json or File, FUNC(String, Json) AS Json`).
