# bug-448: an inline TRAP on a function-value (indirect) call segfaults at runtime

Last updated: 2026-08-22
Effort: unknown (codegen; fallible indirect-call ABI)
Severity: MEDIUM
Class: Correctness (valid source miscompiles into a crashing binary)

Status: Open
Regression Test: to be added (an rt-behavior fixture that traps a function-value call)

Calling a **function value** (a `FUNC(...) AS T` parameter/binding, i.e. an
indirect `blr` call) inside an **inline TRAP** produces a binary that segfaults
(SIGSEGV, exit 139) at the call. The identical function-value call *without* the
inline TRAP works, and the identical inline TRAP on a *direct* (named `FUNC`)
call works, so the defect is specifically the fallible-call handling of an
indirect call under a trap — most likely the trapped-call desugaring / result
register (success/error tag) ABI is not wired for the `blr` indirect form the way
it is for a direct `bl`.

This surfaced while writing arithmetic acceptance probes: a generic
`codeOf(f, a, b)` helper that took the arithmetic op as a `FUNC` value and trapped
it crashed, forcing per-op direct-call wrappers instead.

## Failing Reproduction

```mfbasic
IMPORT io
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC callIt(f AS FUNC(Integer, Integer) AS Integer, a AS Integer, b AS Integer) AS Integer
  LET r AS Integer = f(a, b) TRAP(e)
    RETURN e.code
  END TRAP
  RETURN r
END FUNC
SUB main()
  io::print("result = " & toString(callIt(add, 2, 3)))
END SUB
```

- Observed: the executable builds, then SIGSEGVs at runtime (exit 139), no output.
- Expected: prints `result = 5` (the call succeeds, the handler is never entered).

Bounding cases that WORK today (so the fault is the trap+indirect combination):

- The same function-value call with no inline TRAP —
  `FUNC callPlain(f AS FUNC(Integer,Integer) AS Integer, a, b) AS Integer RETURN f(a, b)` —
  prints `5`, exits 0.
- An inline TRAP on a *direct* named call — `LET r = add(a, b) TRAP(e) …` — works
  (used throughout the acceptance suite).

| Environment | arch | Result |
| --- | --- | --- |
| macOS | aarch64 (release `mfb`) | segfault ✗ |

Not yet checked on x86_64 / linux; the indirect-call + fallible-result ABI is
arch-specific, so re-probe per backend.

## Root Cause (hypothesis, unverified)

The trapped-call lowering (§8.8: `call g(x)` → `MATCH g(x) …`) reads the callee's
fallible result (success/error/exit tags, `./mfb spec memory fallible-call-abi`).
For a direct call this is emitted correctly; for an indirect `blr` through a
function-value pointer the result handling appears wrong (uninitialized or
mis-placed result register, or a clobbered scratch used for the call target),
so the handler dispatch jumps through garbage. Localize by dumping `-nir`/`-ncode`
for the repro's `callIt` and comparing the trapped indirect call against a
trapped direct call.

## Non-goals

- Do not "fix" by rejecting function-value calls under inline TRAP — the code is
  legal (a function value is a call, §8.6 rule 11) and must run.
