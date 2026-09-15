# bug-623: http::read leaks a 64 KiB read buffer per plain-TCP read and ≈384 B per connection

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (to add, Phase 1)

Repeated `http::read` calls grow the arena. Over plain HTTP each call leaks ≈62 KB; over
HTTPS ≈384 B. A long-running client (a crawler, a poller, the browser example following links)
accumulates this forever; a server built on `tcp::read` has the same buffer leak per read.

**The single correct behavior a fix produces:** `http::read` (and `tcp::read`) leave nothing
live once the returned `http::Response` / byte list and the connection are dropped, so a loop
of reads reports equal `live_bytes` at N and 2N over both HTTP and HTTPS.

References:

- `src/docs/spec/memory/04_arenas.md` (scope drop); the `tcp` / `tls` / `http` man pages.
- Found by plan-133-A Phase 2 (the `fetch` stage).
- bug-261 (capped the `tcp::read` buffer size; added no free).

## Failing Reproduction

`/tmp/plan-133-a/stages/h_local.mfb`, `{N}` = 20 and 40, against a local
`python3 -m http.server 8765 --bind 127.0.0.1` serving a 6,839-byte file;
`target/release/mfb build --debug`, macOS, main `14c9fc1ca`:

```
IMPORT io
IMPORT http
IMPORT net

SUB main()
  MUT bytes AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET resp AS http::Response = http::read(net::toUrl("http://127.0.0.1:8765/css2.css"))
    bytes = len(resp.body)
    i = i + 1
  END WHILE
  io::print("bytes=" & toString(bytes))
END SUB
```

- Observed: N=20 `live_bytes 1393296`, `alloc_calls 4472`, `free_calls 4369`; N=40
  `live_bytes 2642000`, `alloc_calls 8804`, `free_calls 8602` — 62,435 B per call.
- Expected: equal `live_bytes` at both N.

HTTPS (`/tmp/plan-133-a/stages/fetch.mfb`, `https://en.wikipedia.org/wiki/BASIC`, a
603,614-byte body): N=1 / 2 / 4 `live_bytes` 13,904 / 14,288 / 15,056 — 384 B per call,
≈5 blocks. Contrast: `net::toUrl` alone in the same loop is flat (`nt_tourl`, 13,520 →
13,520), and the 603 KB body and its intermediates are freed (otherwise HTTPS would leak ≥1 MB
per call).

## Root Cause

Hypotheses from reading the code (plan-133-A, not yet confirmed by a patched build), most
likely first:

1. **`tcp::read` never frees its read buffer.** `lower_net_read_helper`
   (`src/codegen/builtins/tcp/gen_io.rs`) allocates a buffer of `min(maxBytes, 1 MiB)` (65,536 B
   for `http`'s `__http_pump`), reads into it, then allocates an exact-size `List OF Byte`,
   copies and returns that. There is no `emit_arena_free` in `gen_io.rs`: the buffer is dropped
   on success and on every failure label. HTTPS reads go through `lower_tls_read_macos`
   (`src/codegen/builtins/tls/gen_macos/client.rs`), which allocates exactly N bytes — hence
   the 62 KB vs 384 B split. Open: 65,536 B is ≈3 KB above the measured 62,435 B.
   Confirm: shrink the pump's read size (`func_pump.rs`, 65536 → 8192) in a throwaway build;
   the leak must follow it.
2. **Per-connection records are never freed on close.** The TLS ctx block (`CTX_SIZE` 208 B,
   `gen_macos/client.rs`: "reclaimed with the arena" at close), the 96 B socket resource record
   (`src/codegen/os/socket/shared.rs` for tcp, `client.rs` for tls), and probably the resource
   union's `{tag, record}` box (`ResourceUnionCleanup`, `src/codegen/engine/builder/mod.rs`,
   frees only the STATE). 208 + 96 + 16 ≈ 320 B of the 384 B. Confirm: `tcp::connect` +
   `tcp::close` and `tls::connect` + `tls::close` loops, N vs 2N.

## Goal

- The two reproductions report equal `live_bytes` at N and 2N.
- A `tcp::connect`/`read`/`close` loop and a `tls::connect`/`close` loop do too.

### Non-goals (must NOT change)

- `tcp::read` / `tls::read` results and error behavior; the bug-261 buffer cap.
- **Tempting wrong fix:** reusing one static buffer across calls — reads can run on several
  threads (per-thread arenas).

## Blast Radius

- `tcp::read` users: `http` client pump, `http` server `__http_readRequestNet` and
  `__http_lingerNet`, and user programs — fixed by the buffer free.
- `tls::close` ctx, socket records for `tcp`/`tls`/`udp` — audit in Phase 1.
- `lower_net_poll_list_helper` (`src/codegen/os/socket/poll.rs`) pollfd array — no free found
  in that file; audit (not used by `http`'s single-socket poll).

## Fix Design

Free the read buffer after the copy and on every failure label (size = the capped length).
Free the connection's records in the close helpers (and the union box in its cleanup), each
exactly once, zeroing the handle slot.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] `rt_scope_drop_leaks.rs`: loopback `tcp` server + `tcp::read` loop; `http::read` over
      loopback; `tcp`/`tls` connect-close loops. Confirm each fails; confirm hypothesis 1 with
      the throwaway read-size change.
- [ ] Audit udp socket records and the poll list.

Acceptance: cases fail for the documented reason; hypotheses confirmed or replaced.
Commit: —

### Phase 2 — the fix

- [ ] Buffer free in `lower_net_read_helper`; record frees in the close paths.

Acceptance: Phase 1 cases flat; net suites green.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate shifted goldens; full suite; `scripts/test-accept.sh`; Linux box run of the
      loopback case.

Acceptance: full suite green; the loops flat on macOS and Linux.
Commit: —

## Validation Plan

- Regression tests: Phase 1 cases.
- Runtime proof: plan-133-A `h_local` and `fetch` stages.
- Doc sync: none expected.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- None.

## Summary

Two small frees in native transport code; the care is freeing on every failure label exactly
once.

## Phase 1 findings (fix-bug, 2026-09-15)

Measured at main `9b5e5b55f`, `target/release/mfb build --debug`, macOS:

- `h_local` loop over a loopback `python3 -m http.server`: N=20 `live_bytes 1314240`
  (`alloc 4402` / `free 4302`), N=40 `2628480` (`8802` / `8602`) — **65,712 B and 5 blocks per
  call**. That is exactly the 65,536 B `__http_pump` read buffer + 176 B, so hypothesis 1 is
  confirmed without the throwaway read-size build (the doc's 62,435 B figure predates it).
- `tcp::connect` + `tcp::close` loop (accept-and-close loopback peer): N=200 `19200`
  (`400` / `200`), N=400 `38400` — **96 B, one block per connection**: hypothesis 2's socket
  record, confirmed.
- `udp::bind` + `udp::close` loop: N=200 `19200`, N=400 `38400` — the same 96 B record per
  socket. The udp audit finds the same leak.
- RED tests (`tests/runtime/rt_debug_soak.rs`, live_bytes, not RSS — the doc named
  `rt_scope_drop_leaks.rs`, whose helpers measure RSS and cannot see a 96 B block):
  `an_http_read_loop_keeps_live_bytes_constant` (its `#[ignore]` removed),
  `a_tcp_read_loop_keeps_live_bytes_constant`, `a_tcp_connect_close_loop_keeps_live_bytes_constant`,
  `a_udp_bind_close_loop_keeps_live_bytes_constant`, `a_tls_connect_close_loop_keeps_live_bytes_constant`
  (an OpenSSL `s_server` loopback peer; skipped where only LibreSSL is present).
