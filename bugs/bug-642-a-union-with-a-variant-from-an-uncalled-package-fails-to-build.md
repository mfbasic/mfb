# bug-642: a resource union with a variant from a package the program never calls fails to build

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (build failure on a valid program)

Status: Open
Regression Test: none yet — see Phase 1

A program that declares a resource union with a variant from a package it never calls
directly (here `fs::File`), and binds that union through an inline `TRAP`, fails to build
with an internal NIR error. The program is valid; the failure is in the compiler.

**The single correct behavior a fix produces:** the program below builds and prints `ok=1`.

References:

- `src/docs/spec/language/` resource unions and inline TRAP.
- Found by bug-623's union-alias fix work (subagent report), confirmed on the main thread.

## Failing Reproduction

```
IMPORT io
IMPORT udp
IMPORT fs

UNION Chan
  udp::Socket
  fs::File
END UNION

FUNC openOne(port AS Integer) AS Integer
  RES c AS Chan = udp::bind("127.0.0.1", port) TRAP(e)
    RETURN 0
  END TRAP
  RETURN 1
END FUNC

SUB main()
  io::print("ok=" & toString(openOne(0)))
END SUB
```

`mfb build <project>`:

- Observed: `error: NIR runtime call requires undeclared helper 'fs'`, build fails.
- Expected: builds; prints `ok=1`.

| Binary | Result |
| --- | --- |
| main `9b5e5b55f` | fails ✗ |
| integration `38e620ddb` | fails ✗ |

Contrast (subagent-reported): adding any real `fs::` call elsewhere in the program (or a
`tcp::` call for a `tcp` variant) makes it build.

## Root Cause

Hypothesis, to confirm in Phase 1: the runtime helpers a module declares are derived from the
packages it CALLS, but the TRAP-path drop of a resource union emits a tag-dispatched close
for EVERY variant, including `fs.close` for the `fs::File` variant. A variant whose package
is never called therefore reaches NIR as a runtime call to an undeclared helper. Confirm by
locating where the declared-helper set is computed and whether union variant close
operations feed it.

## Goal

- The reproduction builds and runs; the same with the variant order swapped and with a
  `tcp::Socket` variant in a program that never calls `tcp::`.

### Non-goals (must NOT change)

- Programs that already build keep their helper sets.
- **Tempting wrong fix:** requiring an `IMPORT`-side dummy call, or pruning the union
  variant's close from the drop.

## Blast Radius

- Every drop that dispatches a union variant close: the plain union binding drop, the TRAP
  closed default, the owned-list drain — audit in Phase 1.

## Phases

### Phase 1 — failing test + audit

- [ ] A build test for the reproduction (source form); confirm RED.
- [ ] Locate the declared-helper computation; record the root cause here.

Commit: —

### Phase 2 — the fix

- [ ] Declare the close helpers of every resource-union variant a module can drop.

Commit: —

### Phase 3 — full validation

- [ ] Goldens (if helper tables shift); full suite.

Commit: —

## Summary

A declaration-set gap between what lowering emits and what the module declares.
