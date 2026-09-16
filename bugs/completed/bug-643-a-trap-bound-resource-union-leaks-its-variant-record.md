# bug-643: a resource bound through inline TRAP leaks 96 B per bind (concrete and union)

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed
Regression Test: `tests/runtime/rt_debug_soak.rs` —
`a_trap_bound_resource_union_keeps_live_bytes_constant`,
`a_trap_bound_concrete_resource_keeps_live_bytes_constant`, and the handler-path control
`a_trap_bind_whose_producer_fails_keeps_live_bytes_constant`

> **STATUS: FIXED (61a688e31)** — `materialize_current_result` now carries the producer's
> record pointer into the `Result` for a sendable resource this frame owns, instead of
> deep-copying it into a second record and tombstoning the original. Both shapes flat
> (`live_bytes 0`, `free_calls == alloc_calls`, `double_free_skips 0`) and the handler-path
> control unchanged. **Deviation:** the filed root cause was wrong twice over — it is not the
> ownership pass and not the closed default record, but a thread hand-over lowering reached in
> a thread-free program. See Root Cause, which records how the two 96 B candidates were told
> apart. **Residual, filed separately:** `fs::File` under inline `TRAP` still leaks 192 B per
> bind (bug-647), and the borrowed-`poll` shape is left on the copy path deliberately
> (bug-648).

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

(`tcp::listen` works around bug-642.) `mfb build --debug`, integration `38e620ddb`:

- Observed: N=100 `ok=100`, `alloc_calls 504`, `free_calls 404`, `live_bytes 9600`; N=200
  `ok=200`, `1004`/`804`, `live_bytes 19200` — one 96 B block per call, `double_free_skips 0`.
  The TRAP never fires in this run, so the leak is on the success path.
- Expected: equal `live_bytes` at both N.

Contrast: `RES c AS Chan = udp::bind("127.0.0.1", 0)` without TRAP is flat
(`a_resource_union_bound_from_a_producer_keeps_live_bytes_constant`).

## Root Cause

**Confirmed: the orphan is the PRODUCER's record, dropped by a thread hand-over lowering
reached in a thread-free program.**

`materialize_current_result` (`codegen/memory/arena/builder_arena_transfer.rs`) runs the raw
success value through `copy_value_to_current_arena`, whose sendable-resource arm
`copy_resource_to_current_arena` allocates a **second** 96 B record, copies the canonical
header into it, and tombstones the source `moved|closed`. That is the thread-transfer
lowering; the plan-114-C4 comment beside it already recorded that "the `Result` wrap is
reached by any `TRAP` in a thread-free program". In the same arena the source has no other
owner, and the moved bit makes `emit_resource_block_reclaim` skip it — so `udp::bind`'s own
record leaked, once per bind. A **non-sendable** resource (`tls::Socket`, audio) already took
the pointer-carry arm and never leaked, which is why the leak looked resource-kind specific.

### Correction: the closed default record is NOT the leak

An earlier reading on the main thread (recorded here, and wrong) blamed the closed default
record that `bind $trap_valN` with no initializer materializes, on the argument that the
success-path assign orphans it. It does not: `--ncode` on the reproduction shows the
`NirOp::Assign` arm's `emit_resource_cleanup_call` already closes **and** reclaims that record
at the instant the assign overwrites the slot (`resource_cleanup_reclaim_*` →
`bl _mfb_arena_free`, size 96). Only one 96 B block leaks per bind, so it cannot be both.

The decisive discriminator is the alloc/free signature of the fix, not the leak size — both
candidate records are 96 B. Measured on the concrete probe: before `alloc_calls 502` /
`free_calls 402` at N=100, after **`402`/`402`**. The fix removed one **allocation**; frees
did not move. Freeing an orphaned default record would have had to raise `free_calls` to 502
instead. The "always-failing TRAP is flat" control is consistent with BOTH accounts — on the
handler path there is no success value, so neither the default is orphaned nor the copy is
made — so that control localizes the leak to the success path but does not choose between
them. It was over-read.

For the record, the desugar's shape (`ir/lower.rs::lower_inline_trap`) is:

```
bind $trap_res0 : Result OF T = callResult producer(...)
bind $trap_val1 : T                       <-- NO initializer
if resultIsOk($trap_res0) { assign $trap_val1 = resultValue($trap_res0) }
else                      { <handler> }
bind c : T = local $trap_val1
```

A `Bind` with no initializer is lowered through `lower_default_value`
(`builder_value_semantics.rs:184`, whose doc comment names exactly this site), which for a
resource type **materializes a closed default resource record** — a fresh 96 B block, which is
duly reclaimed at the assign, as above.

Measured shapes, all at `1963472c6`:

| shape | N=100 / N=200 `live_bytes` |
| --- | --- |
| success path (`TRAP` never fires) | 9600 / 19200 — leaks 96 B per bind |
| handler path (producer genuinely fails) | 0 / 0 — flat |
| no `TRAP` at all | 0 / 0 — flat |

Two further corrections to the original hypothesis:

- The ownership pass is **not** the problem. `record_ownership.rs::classify` already has an
  explicit arm treating a no-value `Bind` as `Source::Fresh`, commented as "the closed default
  record the inline-TRAP desugar materializes, which this binding allocated".
- `udp::bind("127.0.0.1", -1)` **succeeds**, so a negative port does not exercise the handler
  path — the doc's reproduction never fires its `TRAP`. An unresolvable host (`"300.0.0.1"`)
  does. Anything reasoning about "the failing-bind path" from the reproduction as written is
  reasoning about the success path.

Also noted: `$trap_valN` never appears in the function's NIR `resourceOwners` map (the escape
analysis in `ir/resource_escape.rs` runs on the AST, where these desugar temps do not exist
yet). That is real but not the cause here.

## Goal

- The reproduction flat at N and 2N; the TRAP-firing path also flat and still closing nothing
  it did not open.

### Non-goals (must NOT change)

- No double free on the TRAP path (the closed default record is the binding's own).

## Phases

### Phase 1 — failing test + audit

- [x] Soak tests for the reproduction, union and concrete, plus a genuinely-failing-bind
      control (the filed reproduction's `port = -1` does NOT fail).
- [x] Confirm the ownership hypothesis — **refuted**, twice; see Root Cause.

Commit: `60aac1623` (tests), `61a688e31` (audit)

### Phase 2 — the fix

- [x] ~~Resolve the TRAP-bound union as the owner of a fresh producer's record.~~ Not the
      fix. `materialize_current_result` carries the producer's record pointer into the
      `Result` for a sendable resource this frame owns, instead of deep-copying it into a
      second record and tombstoning the original — matching what the non-sendable-resource
      and `ThreadHandle` arms beside it already do.

Commit: `61a688e31`

### Phase 3 — full validation

- [x] Goldens; full suite — see the integration commit.

Commit: `61a688e31`

## Widened scope (2026-09-15): a concrete resource bound through TRAP leaks the same 96 B

Not specific to unions. Measured after bug-641's sibling-RETURN fix, so no other leak mixes
in:

```
IMPORT io
IMPORT net
IMPORT udp

FUNC risky(bad AS Boolean) AS RES udp::Socket
  RES keep AS udp::Socket = udp::bind("127.0.0.1", 0)
  RES spare AS udp::Socket = udp::bind("127.0.0.1", 0)
  MUT port AS Integer = 0
  IF bad THEN
    port = -1
  END IF
  RES tried AS udp::Socket = udp::bind("127.0.0.1", port) TRAP(e)
    RETURN spare
  END TRAP
  IF port = 0 THEN
    RETURN keep
  END IF
  RETURN tried
END FUNC

SUB main()
  MUT ok AS Integer = 0
  FOR i = 1 TO {n}
    RES s AS udp::Socket = risky((i MOD 2) = 0)
    LET addr AS net::Address = udp::localAddress(s)
    IF addr.port > 0 THEN
      ok = ok + 1
    END IF
  NEXT
  io::print("ok=" & toString(ok))
END SUB
```

- Observed (`mfb build --debug`, integration branch with bug-641): N=100 `ok=100`,
  `alloc_calls 1202`, `free_calls 1102`, `live_bytes 9600`; N=200 `ok=200`, `2402`/`2202`,
  `live_bytes 19200` — one 96 B block per call, `double_free_skips 0`. Before bug-641's fix
  the same program leaked 240 B per call (the sibling-path leak on top).
- Hypothesis (shared with the union shape): the inline-TRAP desugar's result temp and closed
  default record (`$trap_valN` / `$trap_res`) leave a 96 B record the binding never owns, on
  the success path as well as the handler path; `record_ownership` does not resolve the
  TRAP-bound binding as the owner of the fresh producer's record. Confirm in Phase 1 for both
  the concrete and the union shape.
