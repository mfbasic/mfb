# bug-246: an error during a `RES x = <fallible>` binding SIGSEGVs when the function TRAP handler scope-drops the never-initialized resource

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness / Security

Status: Fixed
Regression Test: tests/rt-behavior/resources/bug246_res_bind_error_plain_trap

A resource binding written in the plain form — `RES x = <fallible call>` with a
function-level `TRAP` handler (as opposed to the inline `RES x = <fallible> TRAP`
form) — crashes with `SIGSEGV` when the initializer raises an error. The error
routes to the function's `TRAP` handler, whose scope-drop closes every
function-scope resource local, including `x`. But `x` was never assigned (the
initializer trapped before storing a handle), and its stack slot was never
initialized, so the resource-close inline dereferences a garbage pointer
(`ldr x8, [ptr+8]` reading the closed-flag, then `ldr x0, [ptr]` reading the fd)
and faults. The crash is layout-sensitive — it reproduces under ASLR but not
under a debugger that disables it — which is the signature of reading stack
garbage.

This is the most natural way to use every fallible resource constructor
(`net::accept(listener, timeoutMs)`, `net::connectTcp(host, port, timeoutMs)`,
`fs::open(path, mode)`, `net::listenTcp`, …): bind the result to a `RES` and let
a function-level `TRAP` catch the failure. It was discovered while validating
bug-185 (`net::accept` timeout), whose fix returns `ErrTimeout` — a failure the
natural `RES client = net::accept(listener, ms)` form then crashed on.

## Failing Reproduction

```
IMPORT fs
IMPORT io
FUNC openMissing() AS Integer
  RES f = fs::open("/no/such/dir/missing.txt", "read")   ' fails
  RETURN 0
  TRAP(e)
    RETURN 1
  END TRAP
END FUNC
FUNC main AS Integer
  io::print("caught=" & toString(openMissing() = 1))
  RETURN 0
END FUNC
```

- Observed (before fix): `SIGSEGV` (exit 139), no output — the run crashes in the
  `TRAP` handler's scope-drop of `f`. `x0`/`x21` hold the garbage slot value.
- Expected: the error is caught; `caught=TRUE`, exit 0.

`connectTcp`/`accept`/`listenTcp` reproduce identically; the inline-TRAP form
(`RES f = fs::openFile(...) TRAP(e) … END TRAP`, covered by
`resources/closed-default-drop-rt`) does *not*, because it materializes the
plan-38 closed-default record explicitly.

## Root Cause

Two zero-initialization mechanisms guard owned flat values against exactly this
hazard, and **both excluded resource types**:

- The per-binding pre-init zero (`builder_control.rs`, guarded by
  `owns_freeable_value` → `is_freeable_flat_value`) never fires for a resource.
- The prologue zeroing (`function_lowering.rs`, over `owned_value_slots`) never
  covers resource-cleanup slots — only `OwnedValue` cleanups push to
  `owned_value_slots`.

So a resource slot is zero-initialized nowhere, while its close cleanup
(`ActiveCleanup::Resource` / `ResourceUnion`) is registered as live for the
function scope. The `TRAP` handler's scope-drop then closes a slot holding stack
garbage. Additionally, the resource close inline had no null-guard, so it would
dereference the pointer unconditionally.

## Fix

- Zero-initialize a resource binding's slot before its (possibly fallible)
  initializer, and record the slot for prologue zero-init too — mirroring the
  owned-flat-value path (`builder_control.rs`). `owned_value_slots` is consumed
  solely as the entry-zeroing list, so recording a resource slot there does not
  turn it into an `arena_free`.
- Null-guard the resource-close and resource-union-close emitters
  (`builder_codegen_primitives.rs`): skip the close entirely when the slot is 0,
  so a never-initialized (entry-zeroed) resource is a no-op rather than a fault.

With both, an uninitialized resource slot reads 0 and its close is skipped; an
already-closed real record still runs its close and short-circuits on the
closed-flag as before.

## Validation

- `tests/rt-behavior/resources/bug190_res_bind_error_plain_trap` — plain failing
  `RES` bindings (with and without a prior live resource in scope) are caught, not
  crashed.
- Verified `net::accept`/`connectTcp`/`fs::open` failure forms return their
  error and exit 0; the success and blocking paths are unchanged.
- Full acceptance suite green (the change adds a zero-store + null-guard to every
  resource-bearing function; native goldens re-synced).

## Note on discovery

Found while validating bug-185 (`net::accept` timeout); the natural
`RES client = net::accept(listener, ms)` form crashed on the `ErrTimeout` that
fix introduces, so this is a prerequisite for bug-185 to be usable.
