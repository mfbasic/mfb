# bug-828: json::parse depth cap admits 257 nested containers when the innermost is empty

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/json/json-parse-deep-nesting-rt (extend with the 256/257 boundary)

The structural depth guard is documented as "256 levels of arrays and objects
combined", rejecting anything deeper with `77050024`. In fact the guard fires on
the *value parsed* at depth 257, not on entering the 257th container: 257 nested
arrays (or objects) whose innermost container is empty parse successfully, while
the same 257 containers with any value inside are rejected. The accepted depth
therefore depends on the content of the innermost container. The stack-safety
purpose is not threatened (one extra level), but the documented boundary is wrong
by one for this shape. Correct behaviour: exactly 256 nested containers are
accepted and the 257th container is rejected with `77050024` regardless of its
contents (or the spec states the limit as "257 for an empty innermost container").

References:

- src/docs/spec/stdlib/04_json.md §"Parse acceptance grammar": "caps structural
  nesting at an explicit fixed depth (256 levels of arrays and objects combined) and
  rejects anything deeper with `77050024`"; error table row "well-formed but nested
  deeper than 256 levels".
- `mfb man json parse`: "nested beyond a fixed structural depth (256 levels …)".
- bugs/completed/bug-422-json-parse-structural-recursion-sigsegv.md ("Measured
  boundary: ~257 nested levels parse").

## Failing Reproduction

```
IMPORT io
IMPORT json
IMPORT strings

FUNC tryParse(s AS String) AS String
  LET v AS json::Json = json::parse(s)
  RETURN "ok"
TRAP(e)
  RETURN "err:" & toString(e.code)
END TRAP
END FUNC

FUNC main() AS Nothing
  io::print("257 empty-innermost arrays: " & tryParse(strings::repeat("[", 257) & strings::repeat("]", 257)))
  io::print("257 arrays around a 1:      " & tryParse(strings::repeat("[", 257) & "1" & strings::repeat("]", 257)))
  io::print("257 arrays, innermost [1]:  " & tryParse(strings::repeat("[", 256) & "[1]" & strings::repeat("]", 256)))
  io::print("256 arrays around a 1:      " & tryParse(strings::repeat("[", 256) & "1" & strings::repeat("]", 256)))
  io::print("258 empty-innermost arrays: " & tryParse(strings::repeat("[", 258) & strings::repeat("]", 258)))
  io::print("257 objects, innermost {}:  " & tryParse(strings::repeat("{\"a\":", 256) & "{}" & strings::repeat("}", 256)))
END FUNC
```
`/tmp/hardening/mfb build /tmp/hardening/B02/min2 && /tmp/hardening/B02/min2/build/min2.out`

- Observed:
```
257 empty-innermost arrays: ok
257 arrays around a 1:      err:77050024
257 arrays, innermost [1]:  err:77050024
256 arrays around a 1:      ok
258 empty-innermost arrays: err:77050024
257 objects, innermost {}:  ok
```
- Expected: lines 1 and 6 are `err:77050024` (257 containers > 256).

## Root Cause

`src/codegen/builtins/json/helper_parse_value.rs:__json_parseValue` checks
`IF depth > __JSON_DEPTH_LIMIT` at its own entry and passes `depth + 1` into
`__json_parseArray`/`__json_parseObject`. A container at depth 257 is entered without
a check; only a `parseValue` call for one of its items (depth 257) trips the guard.
An empty container never calls `parseValue`, so it is never checked. Fix: check
`depth + 1 > __JSON_DEPTH_LIMIT` before descending into `[`/`{` (or check in
parseArray/parseObject).

## Goal

- `[`×257 `]`×257 and `{"a":`×256 `{}` `}`×256 raise `77050024`; 256 levels of any
  shape still parse.

### Non-goals (must NOT change)

- The limit value 256, the error code, scalar/sibling handling.

## Blast Radius

- `__json_parseValue` → `__json_parseArray` / `__json_parseObject` — fixed by this bug.
