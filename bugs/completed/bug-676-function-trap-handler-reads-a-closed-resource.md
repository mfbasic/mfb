# bug-676: a function-level TRAP handler that reads the function's resource segfaults

Last updated: 2026-09-21
Effort: small (<1h)
Severity: HIGH
Class: Correctness (a missing diagnostic lets a use-after-free compile)

Status: Fixed
Regression Test: `tests/syntax/resources/resource-closed-in-function-trap-invalid`

**STATUS: FIXED.** Found by plan-145-C Phase 3: the mixed-`WITH` atomicity test
read `h.state` in a function-level `TRAP` handler after a failed `STATE` update, and
the program crashed.

## Failing Reproduction

```basic
IMPORT fs
IMPORT io

TYPE P
  hp AS Integer
END TYPE

FUNC bump(big AS Integer) AS Integer
  RES h AS fs::File STATE P = fs::openFile("/dev/null")
  h.state.hp = h.state.hp + big
  RETURN 0

  TRAP(e)
    io::print("trapped hp=" & toString(h.state.hp))
    RETURN 1
  END TRAP
END FUNC

FUNC main AS Integer
  RETURN bump(9223372036854775807)
END FUNC
```

- Observed (at `4e0c50a8b`): it compiles, and exits 139 (SIGSEGV).
- Expected: a compile error. The same read after `fs::close(h)` is
  `TYPE_USE_AFTER_MOVE`.

A handler that does not name `h` runs correctly ("trapped"), and so does one that
reads a plain record local — only the function's resources are affected.

## Root Cause

An error routed to the function-level `TRAP` runs `trap_route_cleanups`
(`src/codegen/engine/control/builder_exits.rs`) before jumping to the handler. By
design it keeps the function's owned arena values live for the handler but still
closes its resources ("Trap-shared resources *are* still dropped here"), and a
thread is closed but left readable (bug-622). So inside the handler every resource
the function owns is already closed; reading its `STATE` reads the freed record.

The use-after-move pass (`check_resource_moves`, `src/ir/verify/resources.rs`)
checked the handler (`IrOp::Trap`) as an ordinary branch that inherited the body's
moves, and did not know the route closes the resources.

## Fix

The pass checks the handler with every resource the function owns marked closed:
a local whose type has a registered close op, that is not `non_owning` (a `RES`
parameter, a `FOR EACH` element — not the function's to close). Threads have no
close op here and stay readable, as bug-622 requires. A read is
`TYPE_USE_AFTER_MOVE` with the detail "Resource `h` is closed when an error reaches
the function's TRAP, so the handler cannot use it." The spec's error-model rule 3
now says so.

## Validation

- RED: `test-accept.sh <main's release mfb> … resource-closed-in-function-trap-invalid`
  → "3 mismatch(es)" (it compiled); GREEN with the fix.
- Every acceptance fixture whose source has both `TRAP(` and `RES ` (53, found by
  script) still passes: "acceptance tests passed (53 test(s) ran)".
- `cargo test --bin mfb spec` → 43 passed.
- The full suites run in plan-145's final gate (plan-145-I Phase 3).
