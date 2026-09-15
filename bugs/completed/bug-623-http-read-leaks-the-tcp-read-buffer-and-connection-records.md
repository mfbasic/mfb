# bug-623: http::read leaks a 64 KiB read buffer per plain-TCP read and ≈384 B per connection

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed
Regression Test: tests/runtime/rt_debug_soak.rs (tcp/udp/tls/http live_bytes loops, union and passthrough ownership, transferred-socket and sibling-RETURN cases); tests/codegen/codegen_helper_scratch_release.rs

> **STATUS: FIXED (29a902e03)** — `tcp::read` and the OpenSSL / Schannel `tls::read` free their read buffers on every exit; tcp/udp/tls socket and listener records are freed by the OWNING binding's drop (`resource/cleanup/record_ownership.rs`, fail-safe: unproven ownership closes and leaks, never double-frees); the macOS TLS ctx/lctx live on the C heap and are freed at close; Schannel frees STATE/WORK/OUTBUF and, on connect, the 64 KiB server name and the failure-path STATE/record/handles. Every soak case is flat at N and 2N with `double_free_skips 0`; udp/tcp flat on linux-aarch64, linux-x86_64 and windows-x86_64; Schannel failed connect flat on windows-x86_64. **Deviations and additions:** (B) the first fix double-freed a record returned through a `RES` parameter — fixed by the ownership pass, which also moves a returned union's box instead of copying it (the 112 B `http::read` residual); (C) Schannel connect leaks found by a Windows remote proof; the resource-union alias class (a union wrapping a live local closed the owner's handle — a use-after-free once records were freed) fixed in `38e620ddb`; (D) a transferred `tls::Socket` leaked 416 B — the sender's tombstone record is now freed at its drop and the TLS ctx moved to the C heap; the sibling-`RETURN` close leak (bug-632) fixed in `a481a3535`. The earlier `CTX_PEND_BUF` cross-arena finding was withdrawn: a cross-arena free is sound by design (`.ai/canvas-threading.md` §2). Spec synced (`memory/04_arenas.md`, `03_heap-values.md`). Still open, filed separately: bug-633–637 (union variant helper declaration, TRAP-bound resource leak, STATE union, owned union list drain, queued transfer copy).

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

- [x] `rt_scope_drop_leaks.rs`: loopback `tcp` server + `tcp::read` loop; `http::read` over
      loopback; `tcp`/`tls` connect-close loops. Confirm each fails; confirm hypothesis 1 with
      the throwaway read-size change.
- [x] Audit udp socket records and the poll list.

Acceptance: cases fail for the documented reason; hypotheses confirmed or replaced.
Commit: 461a20569, 87ab9cc5c, 3fda5ee87, 3434d3a80

### Phase 2 — the fix

- [x] Buffer free in `lower_net_read_helper`; record frees in the close paths.

Acceptance: Phase 1 cases flat; net suites green.
Commit: 29a902e03, d8003a394, 42a7b2374, 38e620ddb, a9a14985e, a481a3535, 24a5b20d2

### Phase 3 — expected outputs + full validation

- [x] Regenerate shifted goldens; full suite; `scripts/test-accept.sh`; Linux box run of the
      loopback case.

Acceptance: full suite green; the loops flat on macOS and Linux.
Commit: ac41ac77d, 2b3326107, 9318db53b

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

## Phase 2 findings (fix-bug, 2026-09-15)

Implemented by a fix-bug subagent; reviewed and applied on the main thread.

- **Read buffers.** `tcp/gen_io.rs:lower_net_read_helper` frees the capped read buffer
  (size at `MAX_OFFSET`, the bug-261 cap) exactly once on every exit after its allocation:
  success (the result spilled to a frame slot across the `arena_free`), `peer_closed`,
  `read_fail`, `timeout`, and a new `result_alloc_fail` (result-block allocation failure).
  The text path's `encoding_error` also frees the `N + 9` String it built
  (`os/socket/shared.rs:emit_string_result_build` allocates `N + 9`). The same leak was
  in `tls/gen_openssl.rs:lower_tls_read_openssl` (an uncapped `maxBytes` buffer) and
  `tls/gen_schannel_read_close.rs:lower_tls_read` (`OUTBUF`, allocated `NOUT` bytes) —
  both fixed the same way.
- **Socket / listener records** — freed by the owning binding's scope drop, not by `close`:
  a `close` through a `RES` parameter leaves the owner holding the record, and a second
  operation must still read the closed flag (`tests/net/rt_double_close_is_refused.rs`),
  so a free at close would be a use-after-free.
  `resource/cleanup/builder_resource_cleanup.rs:resource_record_freed_at_drop` names
  `tcp.Socket`, `tcp.Listener`, `udp.Socket`, `tls.Socket`, `tls.Listener`; the drop frees
  the 96 B record last (after the close and block reclaim), skips moved records, and zeroes
  the slot. An explicit `close` on the owner keeps that cleanup registered
  (`deactivate_moved_resource_arguments`): the drop's re-close is the existing unreported
  `ErrResourceClosed` no-op. Other resource kinds (`fs`, `audio`, `process`, LINK) keep their
  tombstone record (unaudited producers).
- **TLS per-connection blocks.** macOS: `tls::close` frees the connection ctx and listener
  lctx after the cancel drain, only when `CTX_OWNER` (new slot; `CTX_SIZE` / `LCTX_SIZE`
  208 → 216) equals the closing thread's arena, and not while a poll receive (`CTX_ARMED`)
  or timed-out send (`CTX_WARMED`) is outstanding; a failed connect frees its ctx after
  the drain. Schannel: `close` frees the socket STATE block and the listener WORK block
  (`thread::transfer` copies both into the receiver's arena). OpenSSL has no arena block
  beyond the record.
- **Resource-union box.** The single-binding union drop frees its 16 B `{tag, record}` box;
  the variant record is not freed there (`RES c AS Union = u` wraps a record `u` owns).
- **Remote proof** (`/tmp/wt604_remote_leak.py`, `mfb build --debug --target …`, N=300 vs 600,
  commit `29a902e03`): `udp::bind`+`udp::close` and an in-process `tcp::listen` /
  `tcp::connect` / `tcp::accept` / close loop report `arena.0.live_bytes` 0 → 0,
  `double_free_skips 0` on linux-aarch64 glibc (2223), linux-x86_64 glibc (2228) and
  windows-x86_64 (2230). The harness itself was shown able to fail on all three
  (`as_single` grew 48,000 B with the pre-fix binary). TLS was proved on macOS only
  (`a_tls_connect_close_loop_keeps_live_bytes_constant`); the OpenSSL and Schannel read / close
  paths are built for linux-x86_64, linux-aarch64 and windows-x86_64 but not executed remotely
  (no remote TLS peer).
- **Residual (sub-issue B, found by the fix):** `http::read` still leaves 160 B per call
  (the response's resource-union variant record, 96 B, plus 64 B in two blocks), and a
  resource union bound straight from a producer (`RES c AS Chan = udp::bind(…)`) leaks its
  96 B variant record. RED tests: `an_http_read_loop_leaves_no_block_behind` (50 vs 100
  reads, 4 KiB bound), `a_resource_union_bound_from_a_producer_keeps_live_bytes_constant`;
  guard `a_resource_union_aliasing_a_binding_keeps_live_bytes_constant`.
- **Other bugs found by the fix (separate from 623):** a union alias in an inner scope
  closes the outer handle (`RES u AS udp::Socket = …` then `IF … RES c AS Chan = u END IF`
  then `udp::localAddress(u)` exits 255 with `7-703-0004`; reproduced at `9b5e5b55f`); an
  owned `List OF RES` drain frees no records; the macOS ctx is kept after a transfer or with
  an outstanding receive/send; a Schannel STATE block leaks on a failed connect; the
  existing `CTX_PEND_BUF` free in the macOS read/close paths would `arena_free` a block from
  the sending thread's arena if a transferred socket held buffered plaintext;
  `fs::createTempFile` + `fs::close` keeps its 96 B tombstone record per file.
- **Sub-issue C (new, Windows Schannel connect):** a loop of failing `tls::connect` calls
  (an in-process `tcp::listen` that never answers the handshake, 100 ms timeout, each
  attempt trapped; all 50 attempts fail, measured by a local counting probe) grew
  5,038,400 B per 50 attempts on windows-x86_64 (2230) — **100,768 B per failed connect**
  — and 0 B on linux-aarch64 (OpenSSL, the control). The arithmetic is exact:
  `gen_schannel_io.rs:emit_wide_cstring` allocates a fixed 65,536-byte UTF-16 server-name
  buffer (`SNAMEW`) that nothing frees, plus the STATE block (`st::SIZE` = 320 + 2 ×
  `RECV_CAP` 0x4400 = 35,136 B) and the 96 B record, both allocated before the handshake and
  dropped by the `fail` / `alloc_fail` exits of `gen_schannel_impl.rs` connect:
  65,536 + 35,136 + 96 = 100,768. The `SNAMEW` buffer also leaks on every **successful**
  connect (no free anywhere; `grep -n SNAMEW`). The failure exits also leave the socket open
  and the SSPI credential and context handles unreleased (OS resources, not arena bytes).
  Fix: null `REC` / `SNAMEW` at entry; free `SNAMEW` after its last use (hostname
  verification) on success; on `fail` / `alloc_fail` release the security context and
  credential handle (guarded on STATE), close the socket, and free `SNAMEW`, STATE and the
  record.

## Integration findings (fix-bug, 2026-09-15)

- **Sub-issue B landed** as `d8003a394` (cherry-picked from the subagent's `3da06145b`):
  `resource/cleanup/record_ownership.rs` decides, per function and fail-safe, which resource
  bindings OWN their record (a fresh `tcp`/`udp`/`tls`/`thread` producer, a union wrap of one,
  a TRAP closed default, an alias chain rooted at one, a user function whose every `RETURN`
  is fresh); only an owner's drop frees the record, anything unproven closes and leaks. It
  also **fixes a regression 29a902e03 introduced**: a function returning its `RES` parameter
  made the caller free the record twice (`free_calls` 752 > `alloc_calls` 600,
  `double_free_skips` 152 at N=300; test
  `a_resource_passed_through_a_function_is_freed_once`). A returned resource union's 16 B
  box now moves instead of being copied (`returned_resource_union_owns_box`). Measured:
  `an_http_read_loop_leaves_no_block_behind` (112 B per call = 96 B union record + 16 B box)
  and `a_resource_union_bound_from_a_producer_keeps_live_bytes_constant` GREEN.
- **Sub-issue C landed** as `42a7b2374`: windows-x86_64 (2230) `tls_connect_fail`
  5,038,400 B per 50 failed connects → 0 B, `double_free_skips 0`.
- **Spec synced** (`24a5b20d2`): `memory/04_arenas.md` and `memory/03_heap-values.md` no
  longer claim every resource record survives as a tombstone.
- **`codegen_helper_scratch_release`** pins per-helper `(alloc, free, guarded)` counts; 11 rows
  moved by 623's frees were updated with proof in `3fda5ee87` (none frees a returned block;
  the guarded column is unchanged).
- **Withdrawn: the "`CTX_PEND_BUF` cross-arena free" finding.** A RED test asserted no arena
  frees more bytes than it allocated, and it failed ("arena 1 freed 160 B but allocated only
  144 B"). That invariant is **not a rule of this allocator**: `arena_free` pushes onto the
  FREEING thread's bins and never consults which arena carved the block, and no arena but
  the main one is ever destroyed (`_mfb_arena_destroy` is branched to only from
  `_mfb_shutdown`, `os/process/process_lifecycle.rs`) — `.ai/canvas-threading.md` §2, which
  bug-498's thread message hand-over (`builder_thread_cleanup.rs`) relies on. The test was
  removed and the subagent's re-home fix (`112efea5e`) was not taken.
- **Sub-issue D (new, follows from the same fact):** the `CTX_OWNER` guard 29a902e03 added to
  the macOS `tls::close` ctx free skips the free when the closing thread's arena differs from
  the allocating one. Since that free is sound, the guard leaks the 216 B ctx of every
  `tls::Socket` transferred to another thread and closed there. RED test
  `a_tls_socket_closed_on_another_thread_keeps_live_bytes_constant` measured more: 12,480 B
  of main-arena growth between 30 and 60 transferred sockets, **416 B per socket** — the ctx
  accounts for 216 B, the rest is still to be localized by the fix.
- **Found, being fixed (same resource-union alias class):** a function returning a union that
  wraps its own owned local hands back a closed handle (exit 255, `7-703-0004`; a
  use-after-free since the record free), and a union alias in an inner scope closes the outer
  handle. RED tests: `a_returned_union_wrapping_an_owned_local_stays_open_in_the_caller`,
  `a_union_alias_in_an_inner_scope_leaves_the_outer_handle_open`.
- **Remaining, documented behaviour, not fixed:** `fs::File` keeps its 96 B tombstone record per
  handle (`memory/04_arenas.md`: kinds other than `tcp`/`udp`/`tls` keep the tombstone); an owned
  `List OF RES` drain closes floated elements without freeing their records (close-only,
  fail-safe).
