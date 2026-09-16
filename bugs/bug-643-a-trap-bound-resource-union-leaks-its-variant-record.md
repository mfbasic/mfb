# bug-643: a resource bound through inline TRAP leaks 96 B per bind (concrete and union)

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

(`tcp::listen` works around bug-642.) `mfb build --debug`, integration `38e620ddb`:

- Observed: N=100 `ok=100`, `alloc_calls 504`, `free_calls 404`, `live_bytes 9600`; N=200
  `ok=200`, `1004`/`804`, `live_bytes 19200` — one 96 B block per call, `double_free_skips 0`.
  The TRAP never fires in this run, so the leak is on the success path.
- Expected: equal `live_bytes` at both N.

Contrast: `RES c AS Chan = udp::bind("127.0.0.1", 0)` without TRAP is flat
(`a_resource_union_bound_from_a_producer_keeps_live_bytes_constant`).

## Root Cause

**Confirmed on the main thread at `1963472c6`, and it is neither the union's nor the
producer's record — it is the closed DEFAULT record the desugar allocates and then orphans.**

The desugar (`ir/lower.rs::lower_inline_trap`) emits, for `RES c AS T = producer() TRAP …`:

```
bind $trap_res0 : Result OF T = callResult producer(...)
bind $trap_val1 : T                       <-- NO initializer
if resultIsOk($trap_res0) { assign $trap_val1 = resultValue($trap_res0) }
else                      { <handler> }
bind c : T = local $trap_val1
```

A `Bind` with no initializer is lowered through `lower_default_value`
(`builder_value_semantics.rs:184`, whose doc comment names exactly this site), which for a
resource type **materializes a closed default resource record** — a fresh 96 B block. On the
success path the `assign` then overwrites the slot with the producer's record, and the default
record it displaced has no owner left.

The control is what proves it. A `TRAP` whose producer ALWAYS fails is **flat**:

| shape | N=100 / N=200 `live_bytes` |
| --- | --- |
| success path (`TRAP` never fires) | 9600 / 19200 — leaks 96 B per bind |
| handler path (producer genuinely fails) | **0 / 0 — flat** |
| no `TRAP` at all | 0 / 0 — flat |

When the producer fails the assign never runs, the slot still holds the default record, and
the drop reclaims it. The leak appears exactly when the assign orphans it.

Two corrections to the hypothesis above:

- The ownership pass is **not** the problem. `record_ownership.rs::classify` already has an
  explicit arm treating a no-value `Bind` as `Source::Fresh`, commented as "the closed default
  record the inline-TRAP desugar materializes, which this binding allocated".
- `udp::bind("127.0.0.1", -1)` **succeeds**, so a negative port does not exercise the handler
  path — the doc's reproduction never fires its `TRAP`. An unresolvable host (`"300.0.0.1"`)
  does. Anything reasoning about "the failing-bind path" from the reproduction as written is
  reasoning about the success path.

Also confirmed: `$trap_valN` never appears in the function's NIR `resourceOwners` map (the
escape analysis in `ir/resource_escape.rs` runs on the AST, where these desugar temps do not
exist yet).

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
