# bug-678: `h.state = r` makes the resource share `r`'s block

Last updated: 2026-09-21
Effort: small (<1h)
Severity: HIGH
Class: Correctness (value semantics broken; a double free)

Status: Fixed
Regression Test: `tests/rt-behavior/resources/state-assign-from-local-copies-valid`

**STATUS: FIXED.** Found by plan-145-F's overwrite probe: a program whose last
steps were `h.state = r` and field updates exited 139 (SIGSEGV) at scope exit with
the plan-145-C compiler as well as with the plan-145-F one.

## Failing Reproduction

```basic
IMPORT fs
IMPORT io

TYPE R
  a AS Integer
  b AS Integer
END TYPE

FUNC main() AS Integer
  MUT r AS R = R[a := 1, b := 2]
  RES h AS fs::File STATE R = fs::openFile("/dev/null")
  h.state = r
  r = WITH r { a := 5 }
  io::print(toString(h.state.a) & " " & toString(r.a))
  RETURN 0
END FUNC
```

- Expected: `1 5` — `h.state = r` is an assignment of a value; `r`'s later update
  cannot reach the resource's payload.
- Observed, built `--debug`:
  - main's compiler (`4e0c50a8b`): `0 5`, `arena.0.alloc_calls 9`,
    `free_calls 10` — the rebuild of `r` freed the block the resource still names,
    and the resource read the scrubbed bytes, then freed it again at close.
  - the plan-145-C compiler (`062c50c4c`): `5 5`, `alloc_calls 8`,
    `free_calls 9` — `r`'s scalar update now runs in place (plan-145-C), so it
    writes straight into the resource's payload.

Without the later update the program prints the right value but still frees one
block twice (`alloc_calls 5`, `free_calls 6` for `h.state = r` alone); a longer
program crashed at exit.

## Root Cause

The whole-payload replace in `NirOp::StateAssign`
(`src/codegen/engine/control/builder_control.rs`) lowered its value with
`lower_value_stored_field` — the lowering for a value a record FIELD store
byte-copies into the new record. For a flat value it returns the source as-is
(only a recursive graph is copied), so `h.state = r` stored `r`'s own block
pointer into the resource's STATE slot: two owners of one block.

## Fix

The replace lowers its value with `lower_value_owned`, as a binding's reassignment
does: a register-native vector materializes to a claimed block, an aliasing flat
source (`r`, `o.inner`) is copied, and a recursive graph is copied (or handed
over at the source's last read). The rest of the arm — claiming the pending temp,
freeing the displaced payload (bug-644) — is unchanged.

## Verification

- The regression fixture on the three compilers (`/tmp/p145/runp.sh`, built
  `--debug`):
  - main's: `scalar: state 0, local 5` / `list: state 4/0, local 4/9` /
    `from state: local 7, state 7`, 35 allocated, 37 freed;
  - the plan-145-C compiler: `scalar: state 5, …` / `list: state 4/0, …`, 33
    allocated, 35 freed;
  - the fix: `scalar: state 1, local 5` / `list: state 3/1, local 4/9` /
    `from state: local 2, state 7`, 35 allocated, 35 freed, `live_bytes 0`.
