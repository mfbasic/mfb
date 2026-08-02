# bug-422: `json::parse` has no structural nesting-depth cap → deeply-nested JSON overflows the native stack (uncatchable SIGSEGV) — the depth cap bug-302 required was never landed

Last updated: 2026-08-02
Effort: small (<1h)
Severity: HIGH
Class: Security / Robustness (DoS — uncatchable SIGSEGV on untrusted input)

Status: FIXED (052ad51de)
Regression Test: tests/rt-behavior/json/json-parse-deep-nesting-rt — `json::parse`
of a 5000-deep array (and a 5000-deep object) now returns a clean, catchable error
`77050003` instead of exit 139; nesting within the cap still parses.

## STATUS: FIXED (052ad51de)

Reproduced on macOS-aarch64 (`target/debug/mfb`) exactly as documented: the exact
new fixture built against the pre-fix binary exits **139 (SIGSEGV)**; against the
fixed binary it prints clean `err:77050003` lines and exits 0. Threshold matched
the doc (N=500 arrays parsed cleanly, N≥1000 crashed).

Fix: threaded a `depth` counter through the five recursive parser functions
(`__json_parseValue`, `__json_parseArray`, `__json_parseArrayItems`,
`__json_parseObject`, `__json_parseObjectItems`) and added a cap check at the top
of `__json_parseValue` — `IF depth > __JSON_DEPTH_LIMIT THEN FAIL ...`, with
`__JSON_DEPTH_LIMIT = 256`. This mirrors the regex engine's `__REGEX_DEPTH_LIMIT`
guard (bug-315). Measured boundary: ~257 nested levels parse, deeper fails cleanly
(safe margin under the ~800–1000-frame native-stack crash point). Both the array
and object structural-recursion paths were confirmed fixed.

Also regenerated the 5 json-importing `.ir` goldens (the embedded json source
shifted by the added param + cap; the delta was verified to be *only* the depth
threading and line-number shifts, nothing else) and corrected the `json::parse`
man page and the json spec, both of which claimed nesting depth was bounded only
by the runtime call stack. Full `cargo test`: 4107 passed, 0 failed.

Deviation from the doc: the doc suggested a 128–256 cap; chose **256** (the upper
end) — generous for any realistic document while keeping a ~3× margin under the
crash point.

`__json_parseValue` (`src/builtins/json_package.mfb:331`) recurses structurally on
nested arrays/objects — `parseValue → parseArray (:355) → parseArrayItems (:373) →
parseValue` (and the object equivalent) — one native frame group per nesting level,
with **NO depth cap**. A JSON document of ~1000 nested `[` (≈1 KB, far under the
64 MiB HTTP body cap) overflows the native stack and kills the process with SIGSEGV.

`json::parse` is a public stdlib entry that parses **untrusted input** (HTTP
request/response bodies, files). A caller-side inline `TRAP` does **not** help — a
SIGSEGV is uncatchable. The HTTP server dispatches handler bodies through
`json::parse`, so this is a remote crash.

bug-302 (FIXED) loop-ified the linear *scalar scanners*, but its own plan/task list
explicitly required "Add a nesting-depth cap to `__json_parseValue` for structural
depth" — the depth cap was never landed (bug-302's Resolution only covers the five
scanners). The regex sibling engine added exactly this guard
(`__REGEX_DEPTH_LIMIT`) for the same crash class; json has none. Distinct from
bug-398 (the Rust `tinyjson` parser of build files, not the MFBASIC-source
`json_package`).

References:

- `src/builtins/json_package.mfb:331` (`__json_parseValue`), `:355`
  (`parseArray`), `:373` (`parseArrayItems`) — the unbounded structural recursion.
- bug-302 (`bugs/completed/`) — scanners fixed, structural depth cap not landed.
  Distinct from bug-398. Found during goal-07.

## Failing Reproduction

```
mfb init /tmp/jsondos
# /tmp/jsondos/src/main.mfb:
IMPORT json
FUNC main() AS Integer
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 2000
    s = s & "["
    i = i + 1
  END WHILE
  LET v = json::parse(s)
  RETURN 0
END FUNC
mfb build /tmp/jsondos && /tmp/jsondos/build/*.out ; echo $?
```

- Observed (verified 2026-07-28, `target/debug/mfb`, macOS-aarch64): N=2000 → exit
  **139 (SIGSEGV)**, no output. Agent measured the threshold at ~800–1000 frames
  (N=500 → clean error `77050003`; N≥1000 → 139).
- Expected: a bounded error (e.g. "JSON nested too deeply"), no crash.

## Root Cause

`__json_parseValue` recurses once per structural nesting level with no depth
counter; MFBASIC has no TCO, so recursion depth = nesting depth, which exhausts the
native stack.

## Goal

- `json::parse` rejects input nested beyond a fixed structural depth (e.g. 128–256)
  with a clean error, matching the regex engine's `__REGEX_DEPTH_LIMIT` guard —
  never a SIGSEGV, for any input.

### Non-goals (must NOT change)

- Parsing of legitimately-nested documents within the cap. The scalar-scanner
  loop-ification (bug-302) is correct and stays.

## Blast Radius

- `src/builtins/json_package.mfb` — thread a depth counter through
  `__json_parseValue`/`parseArray`/`parseObject` (mirror `__REGEX_DEPTH_LIMIT`).
- Every `json::parse` caller (HTTP server bodies, file reads) benefits.
