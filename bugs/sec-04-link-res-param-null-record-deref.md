# sec-04: native `LINK` `RES`-parameter closed-guard dereferences the record pointer with no NULL check (potential NULL deref / crash)

Last updated: 2026-07-23
Effort: small (<1h)
Severity: LOW
Class: Memory-safety

Status: Open
Regression Test: (to add) — but see the reachability caveat below: the test must
first establish that a null native-resource record can actually reach a native
`RES`-parameter call. If it cannot, this is closed as unreachable and the guard is
optional hardening.

The thunk for a native `LINK` function that takes a resource parameter
(`RES x AS T` where `T` is a native resource) emits a closed/moved guard at the
top of the thunk: it loads the record pointer from the parameter's frame slot,
loads the CLOSED flag word at `[record + FILE_OFFSET_CLOSED]`, and traps if it is
non-zero. It does **not** first test whether the record pointer itself is null
(`record == 0`). If a null record ever reaches this guard, the
`load [record + FILE_OFFSET_CLOSED]` dereferences null and the app takes a SIGSEGV.

The single correct behavior a fix produces (if reachable): a null resource record
passed to a native `RES`-param function is rejected with the same
closed/invalid-resource runtime error as a closed one, not a segfault.

That null native-resource records **can** occur at runtime is established: the
scope-drop cleanup path guards exactly this case (`RES x = <fallible>` whose
initializer trapped leaves the slot at 0, and a null read there would SIGSEGV —
bug-246). What is **not** confirmed is that such a null record can be routed into a
native `RES`-parameter call before it is either initialized or cleaned up — the
type/flow checker may make that unreachable. This is filed LOW and explicitly
caveated: the impact if reachable is a crash only (no memory corruption, no
control-flow hijack), and reachability is unproven.

References:

- `src/target/shared/code/link_thunk.rs:656-670` — the resource closed/moved guard
  loop: `load record; load [record + FILE_OFFSET_CLOSED]; compare 0; branch_ne
  closed` with no prior `record == 0` test. One emission covers every `RES`-taking
  LINK function.
- `src/target/shared/code/builder_resource_cleanup.rs:223-227` (bug-246) — the
  scope-drop path that DOES null-check the record before reading it, proving null
  records are a real runtime state: "a `RES x = <fallible>` whose initializer
  trapped … leaves the slot at 0 … a null read would SIGSEGV."
- Memory: `[[link-thunk-never-reclaims-the-record]]`,
  `[[trap-desugar-hides-producers-as-locals]]` (resource-init trap desugaring —
  the mechanism that can leave a slot null).
- Found during the 2026-07-23 runtime security audit (FFI marshaling sweep);
  reported by the auditor as an observation to verify, not a confirmed exploit.

## Failing Reproduction

No confirmed reproduction today — the reachability of a null record at a native
`RES`-param call is unproven (Phase 1 must resolve it). The suspected shape:

```
RESOURCE Db CLOSE BY foo::free
LINK "libfoo" AS foo
  FUNC open(path AS String) AS RES Db ...        ' fallible producer
  FUNC exec(RES db AS Db, sql AS String) AS ...  ' takes the resource
END LINK

FUNC main() AS Integer
  RES db AS Db = foo::open("does-not-open")  ' initializer traps → slot may be 0
  ' IF control ever reaches a native RES-param call with db still null:
  foo::exec(db, "SELECT 1")                  ' load [null + FILE_OFFSET_CLOSED] → SIGSEGV
  RETURN 0
END FUNC
```

- Observed (IF reachable): SIGSEGV on the guard's flag load, rather than a clean
  invalid-resource runtime error.
- Expected: a null record is treated as invalid/closed and traps with the standard
  resource error, matching the scope-drop path's null handling.

Contrast: the scope-drop cleanup for the same resource
(`builder_resource_cleanup.rs:223-227`) null-checks the record before dereferencing
it, so end-of-scope cleanup of a trapped-init resource is already safe. Only the
native `RES`-param guard omits the check.

## Root Cause

`link_thunk.rs:656-670` assumes the record pointer in the parameter slot is always
a valid, non-null record (it only distinguishes live vs. closed/moved via the
CLOSED flag). It does not account for the null-slot state that a trapped fallible
initializer can leave behind — the state the cleanup path was specifically fixed to
handle in bug-246. Whether the compiler's flow analysis lets a null record reach
this call site is the open question; the guard itself is unconditionally missing
the null test.

## Goal

- Either: prove a null record cannot reach a native `RES`-param call (close as
  unreachable), OR add a `record == 0 → closed/invalid` branch to the guard so the
  worst case is a clean trap, not a segfault.

### Non-goals (must NOT change)

- The closed/moved semantics or the `FILE_OFFSET_*` record layout.
- The scope-drop cleanup null-check (already correct — the reference behavior).
- The FD@0 handle marshaling for valid records (`link_thunk.rs:937-954`).

## Blast Radius

- `src/target/shared/code/link_thunk.rs:656-670` — the single guard emission; one
  fix covers every native `RES`-taking LINK function.
- Built-in resource operations (non-LINK) — verify whether their `RES`-param
  handling shares the same assumption; if a built-in resource op reads the record
  flag without a null check, it is the same latent hazard and in scope.
- The resource-init trap-desugar path (`[[trap-desugar-hides-producers-as-locals]]`)
  — the source of null slots; understanding it is what settles reachability.

## Fix Design

Phase 1 is decisive: determine reachability. If a null record demonstrably reaches
the guard, add a `compare_immediate(record, "0"); branch_eq(&resource_closed)`
before the flag load in the guard loop — cheap, and routes null to the existing
closed/invalid error path. If unreachable, either close as won't-fix or add the
same branch as belt-and-suspenders hardening (the guard runs once per `RES`-param
call; the cost is one compare) and note the invariant.

Rejected alternative — assuming reachability without proof and shipping a test that
fabricates a null via a proxy: this repo's culture requires reproducing the actual
shape, not a stand-in (`[[reproduce-the-actual-shape-not-a-proxy]]`). Phase 1 must
use the real trapped-initializer path.

## Phases

### Phase 1 — reachability + failing test (no behavior change)

- [ ] Determine whether a null native-resource record can reach a native
      `RES`-param call: trace the flow/type checker and the trap-desugar path for a
      fallible `RES x = <native producer>` followed by a use of `x`. Does anything
      forbid the use, or is the null observable at the guard?
- [ ] If reachable: add a test that produces the SIGSEGV (real trapped-init path,
      not a proxy). If unreachable: document the invariant that makes it so.

Acceptance: a definitive reachability verdict, backed by a repro or by the
specific check that forbids it.
Commit: —

### Phase 2 — the fix (only if reachable, or as hardening)

- [ ] Add the `record == 0 → resource_closed` branch to the guard loop at
      `link_thunk.rs:656-670`.
- [ ] Apply the same guard to any built-in resource op sharing the pattern.

Acceptance: the Phase 1 repro (if any) now traps cleanly; valid `RES`-param calls
unchanged; no golden output moves for valid programs.
Commit: —

### Phase 3 — validation

- [ ] Run the native-link + resource runtime suites.
- [ ] Confirm no golden deltas for valid resource usage.

Acceptance: full suite green; the only new behavior is a clean trap on a null
record (if that state is reachable).
Commit: —

## Validation Plan

- Regression test(s): the reachability repro from Phase 1 (or the documented
  unreachability proof).
- Runtime proof: a trapped-init resource passed onward traps cleanly instead of
  segfaulting; normal resource ops unaffected.
- Doc sync: none expected.
- Full suite: the project's native-link + resource acceptance gates.

## Open Decisions

- Reachability — is a null record observable at a native `RES`-param call?
  (Phase 1.) If no, this is optional hardening rather than a bug fix.

## Summary

Low-severity, crash-only, reachability-unconfirmed: the native `LINK`
`RES`-parameter closed-guard dereferences the record pointer without first testing
for null, while null native-resource records are a known runtime state (bug-246)
that the scope-drop cleanup already guards. The fix — if Phase 1 shows the null can
reach this call site — is a one-instruction null branch routing it to the existing
closed/invalid error path. If unreachable, it is optional hardening. No memory
corruption is possible; worst case is a segfault instead of a clean resource error.
