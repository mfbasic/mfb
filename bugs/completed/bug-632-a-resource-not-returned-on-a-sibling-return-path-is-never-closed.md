# bug-632: a resource not returned on a sibling RETURN path is never closed

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (resource lifetime)

Status: Fixed
Regression Test: tests/runtime/rt_debug_soak.rs (`a_resource_not_returned_on_a_sibling_path_is_still_closed`)

> **STATUS: FIXED (a481a3535)** — `emit_return_exit` saves and restores the cleanup list around every `RETURN <local>`, so the returned local is retired only on the path that returns it; the sibling path, loop body, TRAP routes and normal exit still close it. 600 calls under a 128-descriptor limit exit 0 with `ok=600`, flat `live_bytes`, `double_free_skips 0`; nested-IF, RETURN-in-FOR and TRAP-handler shapes probed. Audit: `EXIT`/`CONTINUE`/TRAP routing copy the list and `EXIT SUB` retires nothing. Folded into bug-623's integration branch; implemented by a subagent, reviewed on the main thread.

A function that owns two resources and returns one of them on each of two paths closes only
one of them. The resource that the fall-through path does not return is never closed, so
every such call leaks a file descriptor and a record. A long-running program exhausts its
descriptor limit.

**The single correct behavior a fix produces:** in
`IF give THEN RETURN a END IF` / `RETURN b`, the path that returns `a` closes `b`, and the
path that returns `b` closes `a`. Each returned record has exactly one closer, the caller's
binding.

References:

- `src/docs/spec/language/` resource management (close on scope exit); `src/docs/spec/memory/04_arenas.md`.
- Found by bug-623's union-alias fix (subagent report, by reading `emit_return_exit_inner`),
  then measured on the main thread.

## Failing Reproduction

```
IMPORT io
IMPORT net
IMPORT udp

FUNC pick(give AS Boolean) AS RES udp::Socket
  RES a AS udp::Socket = udp::bind("127.0.0.1", 0)
  RES b AS udp::Socket = udp::bind("127.0.0.1", 0)
  IF give THEN
    RETURN a
  END IF
  RETURN b
END FUNC

SUB main()
  MUT ok AS Integer = 0
  FOR i = 1 TO 600
    RES s AS udp::Socket = pick((i MOD 2) = 0)
    LET addr AS net::Address = udp::localAddress(s)
    IF addr.port > 0 THEN
      ok = ok + 1
    END IF
  NEXT
  io::print("ok=" & toString(ok))
END SUB
```

`mfb build --debug`, then run under `ulimit -n 128`:

- Observed: `Error: 7-707-0003`, exit 255. `udp::bind` fails after roughly 250 calls. Without
  the limit, `ok=600`, but `live_bytes` grows 14,400 B at N=300 and 28,800 B at N=600: one
  96 B block per two calls, `double_free_skips 0`.
- Expected: `ok=600`, exit 0, flat `live_bytes`.

| Binary | Result under `ulimit -n 128` |
| --- | --- |
| main `9b5e5b55f` (pre bug-623) | fails ✗ (`7-707-0003`) |
| integration `38e620ddb` (bug-623 + union-alias fix) | fails ✗ (`7-707-0003`) |

Contrast: a single `RETURN` of one owned local, and the union return
(`a_returned_union_wrapping_an_owned_local_stays_open_in_the_caller`, made path-local by
`38e620ddb`), both work.

## Root Cause

Read, not yet confirmed by a patched build: `src/codegen/engine/control/builder_exits.rs:emit_return_exit_inner`
removes the returned local's cleanup from `active_cleanups` permanently while lowering
`RETURN a`. The sibling path, which falls through to `RETURN b`, is lowered afterwards
with `a`'s cleanup already gone, so nothing closes `a` on that path. `38e620ddb` made the
same removal path-local for a resource UNION return (a snapshot/restore around the exit);
the concrete case was left as it was.

## Goal

- The reproduction exits 0 with `ok=600` under a 128-descriptor limit, flat `live_bytes`,
  `double_free_skips 0`.

### Non-goals (must NOT change)

- The returned record keeps exactly one closer (the caller's). No double close.
- `record_ownership` facts stay consistent: a returned local is fresh for the caller only
  where it is retired on that path.

## Blast Radius

- Every exit that removes cleanups while being lowered — `EXIT`/`CONTINUE`, TRAP routing,
  `RECOVER`, `EXIT SUB`, an early `RETURN` inside a loop — to be audited for the same
  permanent-removal pattern in Phase 1.

## Fix Design

Make the concrete return's cleanup removal path-local, like the union case: snapshot
`active_cleanups` before emitting the exit and restore it afterwards, so the returned local
is retired only on the path that returns it.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] `a_resource_not_returned_on_a_sibling_path_is_still_closed` (setrlimit 128 + flat
      `live_bytes`); confirm RED.
- [x] Audit every cleanup-removing exit.

Acceptance: RED for the documented reason; audit verdicts recorded.
Commit: dcfc9b73a

### Phase 2 — the fix

- [x] Path-local cleanup removal for concrete returns (and any audited sibling exit).

Acceptance: the test passes; the bug-623 soak cases and resource neighbours stay green.
Commit: a481a3535

### Phase 3 — expected outputs + full validation

- [x] Regenerate shifted codegen goldens (inspect the delta); full suite.

Acceptance: full suite green; golden deltas are only the restored closes.
Commit: 2b3326107

## Validation Plan

- Regression test: the Phase 1 case.
- Runtime proof: the reproduction under `ulimit -n 128`.
- Full suite: `cargo test --release --no-fail-fast -- --skip artifact_gate_all`; `scripts/artifact-gate.sh <mfb> all`.

## Summary

A cleanup-list bookkeeping bug at RETURN lowering; the care is in auditing every other
exit that edits the cleanup list.
