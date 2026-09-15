# bug-634: a resource union bound through inline TRAP leaks 96 B per bind

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

`RES c AS Chan = udp::bind(...) TRAP(e) ... END TRAP` leaves one 96 B block live per bind,
even when the TRAP never fires. A function that opens a union handle per call leaks forever.

**The single correct behavior a fix produces:** the reproduction reports equal `live_bytes`
at N=100 and N=200, with `double_free_skips 0`.

References:

- bug-623 B (`record_ownership`): a union bound straight from a producer owns and frees its
  variant record; the TRAP-bound shape does not.
- Found by bug-623's union-alias fix work (subagent report), measured on the main thread.

## Failing Reproduction

```
IMPORT io
IMPORT udp
IMPORT tcp

UNION Chan
  udp::Socket
  tcp::Socket
END UNION

FUNC openOr(bad AS Boolean) AS Integer
  MUT port AS Integer = 0
  IF bad THEN
    port = -1
  END IF
  RES c AS Chan = udp::bind("127.0.0.1", port) TRAP(e)
    RETURN 0
  END TRAP
  RETURN 1
END FUNC

SUB main()
  RES keep AS tcp::Listener = tcp::listen("127.0.0.1", 0)
  MUT ok AS Integer = 0
  FOR i = 1 TO {n}
    ok = ok + openOr(i MOD 2 = 0)
  NEXT
  io::print("ok=" & toString(ok))
END SUB
```

(`tcp::listen` works around bug-633.) `mfb build --debug`, integration `38e620ddb`:

- Observed: N=100 `ok=100`, `alloc_calls 504`, `free_calls 404`, `live_bytes 9600`; N=200
  `ok=200`, `1004`/`804`, `live_bytes 19200` — one 96 B block per call, `double_free_skips 0`.
  The TRAP never fires in this run, so the leak is on the success path.
- Expected: equal `live_bytes` at both N.

Contrast: `RES c AS Chan = udp::bind("127.0.0.1", 0)` without TRAP is flat
(`a_resource_union_bound_from_a_producer_keeps_live_bytes_constant`).

## Root Cause

Hypothesis, to confirm in Phase 1: the inline-TRAP desugar binds a `$trap_res` result temp
and then the union binding from `resultValue` of it, so
`resource/cleanup/record_ownership.rs:classify` sees a `ResultValue` of a local whose store is
a `UnionWrap` of a call-result, which it may not resolve as fresh. The union binding is then
not an owner, and its drop closes without freeing the 96 B variant record (fail-safe
direction). Confirm by dumping `record_owning_locals` for `openOr`.

## Goal

- The reproduction flat at N and 2N; the TRAP-firing path also flat and still closing nothing
  it did not open.

### Non-goals (must NOT change)

- No double free on the TRAP path (the closed default record is the binding's own).

## Phases

### Phase 1 — failing test + audit

- [ ] Soak test for the reproduction (both the success and the failing-bind path); confirm RED.
- [ ] Confirm the ownership hypothesis.

Commit: —

### Phase 2 — the fix

- [ ] Resolve the TRAP-bound union as the owner of a fresh producer's record.

Commit: —

### Phase 3 — full validation

- [ ] Goldens; full suite.

Commit: —
