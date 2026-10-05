# bug-880: a native `LINK` wrapper with more than eight MFBASIC parameters fails the build with an internal aarch64 error, regardless of its ABI slot count

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/native/native-wrapper-nine-params-rt (new) — name suggested

A `LINK` wrapper `FUNC` declaring nine parameters fails `mfb build` on macOS AArch64 with the
unstructured `error: aarch64 code plan cannot pass argument 8; stack arguments are not implemented`.
This happens even when the C call has a **single** ABI slot (nine parameters all feeding one
`IN` struct through `BIND IN`) — a call the spec's slot-count rule allows. When the wrapper does
declare nine integer slots, the designed, explanatory rejection ("native function `f` declares 9
integer ABI slots, but this target passes only 8 …", which x86-64 does print for seven slots) is
never reached; the internal error preempts it. Ordinary MFBASIC functions with nine parameters
compile and run (`45` for a nine-argument sum), so this is specific to the thunk.

Correct behavior: a wrapper's MFBASIC parameter count is not limited by the external C register
count; only the integer ABI-slot count is, and exceeding that yields the designed message (ideally a
located rule diagnostic).

References:

- src/docs/spec/language/17_native-libraries.md "Integer-slot limit (target-dependent)": "a native function declaring more integer (non-`CDouble`) ABI slots than the target passes in registers is **rejected at build time** … The limit comes from the selected target's external register model".
- Found during B05 hardening of language/17_native-libraries.

## Failing Reproduction

project.json `libraries`: `"c": [{"os":"macos","type":"system","source":"libSystem.dylib"}]`.

```
IMPORT io
TYPE R
  a AS Integer
  b AS Integer
  c AS Integer
  d AS Integer
  e AS Integer
  f AS Integer
  g AS Integer
  h AS Integer
  i AS Integer
END TYPE
LINK "c" AS libc
  CSTRUCT S AS R
    a CInt32
    b CInt32
    c CInt32
    d CInt32
    e CInt32
    f CInt32
    g CInt32
    h CInt32
    i CInt32
  END CSTRUCT
  FUNC firstWord(a AS Integer, b AS Integer, c AS Integer, d AS Integer, e AS Integer, f AS Integer, g AS Integer, h AS Integer, i AS Integer) AS Integer
    SYMBOL "abs"
    ABI (s IN S) AS r CInt32
    BIND IN s
      a = a
      b = b
      c = c
      d = d
      e = e
      f = f
      g = g
      h = h
      i = i
    END BIND
    RETURN r
  END FUNC
END LINK
FUNC main() AS Nothing
  io::print(toString(libc::firstWord(1, 2, 3, 4, 5, 6, 7, 8, 9)))
END FUNC
```

- Observed: `error: aarch64 code plan cannot pass argument 8; stack arguments are not implemented` (build rc=1).
- Expected: builds (one pointer-sized ABI slot).

Matrix (measured):

| Case | Target | Result |
| --- | --- | --- |
| 9 params, 1 struct slot | macos-aarch64 | internal error ✗ |
| 9 params, 9 CInt64 slots | macos-aarch64 | internal error ✗ (designed message never shown) |
| 8 params, 8 CInt64 slots | macos-aarch64 | builds ✓ |
| 7 params, 7 CInt64 slots | `--target linux-x86_64` | designed "declares 7 integer ABI slots … only 6" error ✓ |

## Root Cause

`src/codegen/link/thunk/link_thunk.rs:lower_link_thunk` begins by spilling every incoming wrapper
argument with `abi::argument_register(index)?` for `index in 0..n_params` ("Save incoming wrapper
arguments before any clobbering call"). `src/target/shared/abi.rs:argument_register` errors for
`index >= REGISTER_ARGUMENT_COUNT` (8). The thunk does not use the compiler's convention for
incoming arguments past the eighth, which ordinary functions handle; the external slot-count check
(`int_slot_count > external_int_registers`) comes much later in the same function.

## Goal

- A wrapper with N > 8 MFBASIC parameters builds whenever its external integer-slot count fits; the slot-limit rejection is the only native argument-count error.

### Non-goals (must NOT change)

- The external-slot limit itself (stack staging for C calls is out of scope).

## Blast Radius

- Same pattern at `link_thunk.rs` ~1853 (`location: abi::argument_register(idx)?`) — check.
