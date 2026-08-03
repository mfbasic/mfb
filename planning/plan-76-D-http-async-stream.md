# plan-76-D: http async client — Stream union + startRead/ready/pump/done/finish

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: **plan-80 (unified resource-record header — HARD PRECONDITION; fixes the D4
core-premise defect below)**, plan-76-B (scalar `tls::poll` — `http::ready`/`pump` gate TLS reads on
it), and plan-74 (uniform STATE on a resource union — landed). NOT dependent on plan-76-A or
plan-76-C (this single-stream client uses only *scalar* readiness), and NOT on plan-75 (the stream is
never transferred across threads).

> **BLOCKED on plan-80.** D's design carries plan-74 `STATE` on a `Stream = {Socket | TlsSocket}`
> union. That is unimplementable until `STATE` lives at a record offset free in the `TlsSocket`
> layout — see Corrections **D4**. plan-80 relocates `STATE` to offset 24 and adds the per-backend
> `STATE`-slot assert whose absence D4 identified. D must NOT start until plan-80's Phase 4 (the
> `STATE@24` / D4 gate) is green: `ls planning/completed/plan-80-* 2>/dev/null` OR plan-80 Phase 4
> ticked, then re-run D's own design-gate row (Prerequisites line 68).

This sub-plan turns the `http` client from blocking-only into a cooperatively-drivable one, and is
the motivating consumer of plan-74's resource-union STATE. It introduces a `Stream` resource union
(`net::Socket | net::TlsSocket`) carrying a `PendingState`, and five public entry points that let a
program advance an HTTP exchange without blocking its thread:

```
UNION Stream                                     ' every variant is a resource → Stream is a resource
  net::Socket
  net::TlsSocket
END UNION

TYPE PendingState
  sentAll AS Boolean            ' request fully written
  closed  AS Boolean            ' peer EOF observed (Connection: close terminator)
  raw     AS List OF Byte       ' accumulated response bytes
  err     AS Integer            ' 0 = ok; else a captured failure code
END TYPE

FUNC http::startRead(url, headers, method) AS RES http::Stream STATE PendingState  ' variant from url.scheme
FUNC http::ready (RES stream AS http::Stream STATE PendingState) AS Boolean         ' data available now?
SUB  http::pump  (RES stream AS http::Stream STATE PendingState)                    ' one non-blocking read; grows state.raw
FUNC http::done  (RES stream AS http::Stream STATE PendingState) AS Boolean         ' response complete?
FUNC http::finish(RES stream AS http::Stream STATE PendingState) AS Response        ' parse state.raw
```

The single behavioral outcome: a program can `RES s AS http::Stream STATE PendingState =
http::startRead(url, {}, "GET")`, then loop `IF http::ready(s) THEN http::pump(s)` (interleaving its
own work) until `http::done(s)`, and `http::finish(s)` yields the same `Response` a blocking
`http::read(url)` would — for both `http://` and `https://` URLs, with the socket closed exactly
once on scope exit. And `http::read`/`http::write` are rewritten as thin blocking wrappers over
these, producing byte-identical `Response` values to today.

References (read first):

- `mfb spec language resource-management` §15.5 (union STATE) / §15.6 (resource unions) — the
  language contract this consumes; **read the "A resource union may carry STATE" paragraph**.
- `planning/completed/plan-74-resource-union-state.md` — the delivered STATE-on-a-union feature
  (bind / parameter / return / `.state` via value|param|MATCH / drop). This sub-plan is the "bundled
  and URL-transparent non-blocking HTTP handle" that plan-74 §1 names as its out-of-scope consumer.
- `src/builtins/http_package.mfb` — the entire BASIC http implementation; the blocking client is
  `__http_read`/`__http_write` (:376/:383) over `__http_exchangeTcp`/`__http_exchangeTls`
  (:311/:340). Reuse `__http_buildRequest` (:155), `__http_parseResponse` (:260),
  `__http_frameComplete` (:578), `__http_framingLength` (:559).
- `src/builtins/http.rs` — the descriptor shim: `HTTP_FUNCTIONS` (:181), `HTTP_TYPES` (:224),
  `Implementation::Rewrite` wiring, `dispatch_resolve` (:318), `expected_arguments` (:378).
- `src/builtins/json_package.mfb:70` (`EXPORT UNION Json`) — precedent that a builtin `.mfb` may
  declare/EXPORT a UNION; here it is a **resource** union over imported net/tls variants (novel).
- Memory: imported-package resource hazards ([[imported-package-resource-two-spellings]]) — a union
  over `net::`/`tls::` variants declared in `http` must resolve each variant's close op correctly.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-74 landed (union STATE: bind/param/return/`.state`/drop) | `ls planning/completed/plan-74-*` | MET |
| plan-76-B landed (scalar `tls::poll` on all backends) | `rg -n 'POLL' src/builtins/tls.rs`; `ls tests/rt-behavior/tls/tls-poll-rt` | MET (B complete) |
| `net::poll(sock[, timeoutMs]) AS Boolean` scalar exists | `rg -n 'POLL' src/builtins/net.rs` → :163 | MET |
| Feature-wide gate (tree green, gate clean) | see plan-76-A Prerequisites | MET (tests 3757/0; net+tls gates PASSED) |
| **plan-74 union STATE works over a `TlsSocket` variant** (the DESIGN GATE this plan should have tested) | bind `RES s AS Stream STATE PendingState = tls::connect(...)`; run — must not SIGSEGV | **MET (2026-08-03) — plan-80 landed.** `ls planning/completed/plan-80-*` → present; STATE relocated to `RESOURCE_OFFSET_STATE = 24` (free in every layout) with per-backend asserts. Re-ran the bind: `tests/rt_macos_d4_union_state_tls.rs` binds `RES s AS Stream STATE PendingState = tls::accept(...)` over a live TlsSocket, mutates STATE, drives real TLS I/O — **no SIGSEGV** (was exit 139 pre-plan-80). Full gate green. |

**Explicitly NOT prerequisites (do not braid):**

| Not required | Why independent | Command |
|---|---|---|
| plan-76-A (`net::poll(List)`) | This single-stream client uses only *scalar* `net::poll`/`tls::poll`, never a list-poll. | `ls planning/plan-76-A-*` |
| plan-76-C (`tls::poll(List)`) | Same — one stream, scalar readiness only. | `ls planning/plan-76-C-*` |
| plan-75 (resource-union `thread::transfer`) | The stream is never sent across threads; it is bound, driven, and dropped in one thread. | `ls planning/plan-75-*` |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run and update
> before continuing and before stopping; report all rows if you stop.

## 1. Goal

- The `Stream` union and `PendingState` type are declared in `http_package.mfb` and registered as
  builtin types (`http::Stream`, `http::PendingState`); `Stream` is a **resource** union.
- `http::startRead(url AS net::Url, headers AS Map OF String TO String = {}, method AS String =
  "GET") AS RES http::Stream STATE PendingState` connects per `url.scheme` (https → `tls::connect`,
  else `net::connectTcp`, both with the existing 30 s connect timeout), writes the built request,
  and returns the union with `state = { sentAll: TRUE, closed: FALSE, raw: [], err: 0 }`.
- `http::ready(RES s …) AS Boolean` — `TRUE` iff a non-blocking read would return bytes or EOF now
  (MATCH → `net::poll(sock, 0)` / `tls::poll(sock, 0)`).
- `http::pump(RES s …)` — one **non-blocking** read of available bytes (gated internally on
  readiness so it never blocks), appended to `s.state.raw`; a 0-byte read sets `s.state.closed`; a
  transport failure sets `s.state.err`.
- `http::done(RES s …) AS Boolean` — `TRUE` iff `s.state.closed` OR the accumulated `s.state.raw`
  is a complete frame (`__http_frameComplete`) OR `s.state.err <> 0`.
- `http::finish(RES s …) AS Response` — if `s.state.err <> 0`, `FAIL`; else
  `__http_parseResponse(s.state.raw)`. The stream stays bound and is closed by the caller's drop.
- `http::read`/`http::write` are rewritten to: `startRead` → block-until-ready + `pump` loop until
  `done` → `finish`, producing the **same** `Response` bytes as today.

### Non-goals (explicit constraints)

- **No change to the `http::read`/`http::write` public signatures or their returned `Response`
  values.** They must remain byte-for-byte equivalent (same headers, body, chunked handling,
  size cap, control-byte rejection). This is the compatibility guardrail.
- **No async request-body streaming.** `startRead` writes the whole request up front (`sentAll` is
  set immediately). `sentAll` is reserved for a future write-pump; this plan does not implement one.
- **No thread transfer of a `Stream`.** Single-thread drive only (keeps plan-75 out of scope).
- **No new native intrinsics.** All five functions are BASIC over existing `net::`/`tls::` calls
  (`connect`, `write`/`writeText`, `read`, `poll`) + plan-74's union STATE. `http.rs` gains only
  descriptor-table rows.
- **No server-side change.** `handleRequest`/`serverSSL` and their duplicated transport branches are
  untouched (they could later share the union, but not here).
- **No `get`/`post` convenience verbs.** Method stays an argument, as today.

## 2. Current State

- `http` is a pure-BASIC source package: a thin Rust shim (`http.rs`) of descriptor tables +
  `Implementation::Rewrite(__http_*)`, and the implementation in `http_package.mfb` (1216 lines),
  injected into the user AST when `IMPORT http` is present (`augmented_project`,
  `src/builtins/mod.rs:67`; invoked at `src/ir/lower.rs:107`).
- The blocking client already does exactly the protocol work `finish` needs: `__http_buildRequest`
  (`:155`), `__http_parseResponse` (`:260`) with de-chunking (`__http_dechunkBytes` `:597`) and the
  64 MiB cap; the transport loops `__http_exchangeTcp`/`__http_exchangeTls` (`:311`/`:340`) read
  64 KiB at a time until a 0-byte read. **These functions are reused, not rewritten** — the async
  layer only changes *when* bytes are read (readiness-gated) not *how* they are parsed.
- `Connection: close` is forced in every built request (`http_package.mfb:172`), so the response
  terminator is peer EOF; `__http_frameComplete` (`:578`) already recognizes a complete framed
  response earlier (Content-Length satisfied / final chunk) — giving `done` an early-out before EOF.
- `net`/`tls` transport is native; both packages are file-scoped-imported by `http` already
  (`http_package.mfb:6-7`). `net::poll(sock, 0)` exists; `tls::poll(sock, 0)` arrives in plan-76-B.
- Union support in a builtin `.mfb`: `EXPORT UNION Json` (`json_package.mfb:70`) proves the parser
  accepts it; but that is a **data** union. A **resource** union over `net::Socket`/`net::TlsSocket`
  variants declared inside `http` (which imports both) is untried — the design risk (§3).
- Builtin type registration: names go in the module `types:` table (`HTTP_TYPES`, `http.rs:224`);
  fields/variants live only in the `.mfb`. Adding `Stream`/`PendingState` means two new `types:`
  rows + the `.mfb` declarations.

### Measured populations

| What | Count | Command |
|---|---|---|
| Public `http::` client functions today | 2 (read, write) | `rg -n 'READ\|WRITE' src/builtins/http.rs` → :182-183 |
| New public functions to add | 5 (startRead, ready, pump, done, finish) | this plan |
| Reused protocol helpers (no rewrite) | 4 | buildRequest :155, parseResponse :260, frameComplete :578, framingLength :559 |
| Builtin `.mfb` UNION precedents | 1 data union | `rg -n 'EXPORT UNION\|^\s*UNION' src/builtins/*.mfb` → json:70 (+ regex internal) |
| Resource **union**-in-builtin precedents | 0 | (none — this is first) |
| http rt-behavior/acceptance tests to keep green | UNMEASURED | `rg -rl 'http::' tests \| wc -l` — run at start |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| http is BASIC; new functions need no native lowering | **CONFIRMED** | `http.rs:1-5` header ("no new intrinsics"); every entry is `Implementation::Rewrite` (`:182-221`). |
| A builtin `.mfb` may declare & EXPORT a UNION | **CONFIRMED (data union)** | `json_package.mfb:70` `EXPORT UNION Json`. |
| plan-74 union STATE (bind/param/return/`.state` value|param|MATCH/drop) works | **CONFIRMED** | plan-74 Phases 1–4 landed (`planning/completed/plan-74-*`), fixtures `resource-union-state-{access,drop}-valid`. |
| A **resource** union over imported (`net::`/`tls::`) variants, declared in a builtin package, binds/MATCHes/drops correctly | **UNVERIFIED** — Phase 1 falsifies FIRST | No precedent. Close-op dispatch must resolve `net::close`/`tls::close` for each variant when the union is declared in `http`. The [[imported-package-resource-two-spellings]] hazard is about decoded `.mfp` packages; `http` is injected as source, so it *should* carry native_resources — but this is unproven and is the design risk. |
| `net::Socket` sendable, `tls::TlsSocket` NOT sendable — can they share a union? | **UNVERIFIED** — Phase 1 | `resource.rs:151` (Socket `sendable:true`) vs `:213` (TlsSocket `sendable:false`). Mixed sendability may trip a union check; irrelevant to drop/read but confirm the union is accepted. Since we never transfer, non-sendable is fine if the union binds. |
| `http::read`/`write` output is reproducible from `startRead`+pump-loop+`finish` | **CONFIRMED (by construction)** | Both paths call the identical `__http_buildRequest` + `__http_parseResponse`; only the read loop's blocking-vs-gated shape differs, and both accumulate the same `raw` bytes. Phase 4 proves byte-equality against saved goldens. |

## 3. Design Overview

Four pieces, layered:

1. **Types (Phase 1, design uncertainty).** Declare `Stream` (resource union) + `PendingState` in
   `http_package.mfb`; register both names in `HTTP_TYPES`. Phase 1 is a *falsification experiment*:
   a minimal internal `__http_streamProbe` that binds `RES s AS http::Stream STATE PendingState`
   from a `net::connectTcp`, MATCHes it, reads `s.state`, and drops it — proving a resource union
   over imported variants binds, states, matches, and closes. If it does not compile/run, STOP and
   resolve the imported-variant close-op wiring before building the five functions.

2. **The five BASIC functions (Phase 2).** `__http_startRead` / `__http_ready` / `__http_pump` /
   `__http_done` / `__http_finish` in `http_package.mfb`, reusing the existing protocol helpers.
   `pump` MATCHes the union and does a readiness-gated single read (§4.2). Internal (non-exported)
   helper `__http_waitReadable(RES s …)` does the *blocking* readiness wait (MATCH → `net::poll(s)`
   / `tls::poll(s)` with omitted timeout) for the blocking wrappers — keeping `pump` non-blocking.

3. **Descriptor wiring (Phase 3).** Add five `hfn(...)` rows to `HTTP_FUNCTIONS` with
   `Implementation::Rewrite(__http_*)`, plus `dispatch_resolve` return types, `call_param_names`,
   `expected_arguments`, and the two `HTTP_TYPES` rows. `ready`/`done`/`finish` return
   Boolean/Boolean/Response; `startRead` returns `RES http::Stream STATE PendingState`; `pump` is a
   SUB (Nothing). The `RES … STATE …` parameter/return spellings ride plan-74's verifier/codegen.

4. **Rewrite the blocking wrappers (Phase 4, compatibility guardrail).** `__http_read`/`__http_write`
   become: `RES s = startRead(...)`; `WHILE done(s)=FALSE { __http_waitReadable(s); pump(s) }`;
   `RETURN finish(s)`. Proven byte-identical to today via the saved `http::read`/`write` goldens.

**Where design uncertainty concentrates (schedule FIRST):** the **resource union over imported
variants** (Phase 1). Everything else is ordinary BASIC over proven helpers; if the union binds,
matches, states, and drops, the rest is mechanical.

**Where correctness risk concentrates:** (a) **drop-exactly-once** of the active variant's socket on
every exit path (guarded by plan-74's own machinery + a Phase 2 leak-loop fixture); (b) **byte-parity**
of the rewritten blocking wrappers (Phase 4, against saved goldens — the one thing that must not
regress).

**Rejected alternatives:**

- *Two parallel APIs (`startReadTcp`/`startReadTls`, no union), like the server's
  handleRequest/SSL.* Rejected — it doubles every function and defeats URL-transparency; the union
  is exactly what plan-74 was built to enable. (Recorded so nobody "simplifies" back to it.)
- *Store `raw` outside the resource (a caller-held `MUT List OF Byte`).* Rejected — then the buffer
  is not tied to the handle's lifetime and a MATCH cannot carry it; STATE on the union is the point.
- *Encode EOF in `err` instead of a `closed` field.* Rejected — overloading `err` conflates "done
  cleanly" with "failed"; a dedicated `closed AS Boolean` keeps `finish`'s error check clean. (This
  is the one addition to the prompt's illustrative `PendingState`; noted in Open Decisions.)
- *Make `pump` block until data.* Rejected — kills the cooperative-drive use case; blocking lives in
  the wrappers' `__http_waitReadable`, not `pump`.

## 4. Detailed Design

### 4.1 `__http_startRead` (scheme branch → union with STATE)

```
FUNC __http_startRead(url AS net::Url, headers AS Map OF String TO String, method AS String) AS RES http::Stream STATE PendingState
  LET verb    AS String = __http_normalizeMethod(method)
  LET request AS String = __http_buildRequest(verb, url, "", FALSE, headers)
  IF url.scheme = "https" THEN
    RES s AS http::Stream STATE PendingState = tls::connect(url.host, url.port, __HTTP_CONNECT_TIMEOUT_MS, url.host)
    MATCH s
      CASE net::TlsSocket(t) : tls::writeText(t, request)
      CASE net::Socket(p)    : net::writeText(p, request)   ' unreachable but total
    END MATCH
    s.state.sentAll = TRUE
    RETURN s
  END IF
  RES s AS http::Stream STATE PendingState = net::connectTcp(url.host, url.port, __HTTP_CONNECT_TIMEOUT_MS)
  MATCH s
    CASE net::Socket(p)    : net::writeText(p, request)
    CASE net::TlsSocket(t) : tls::writeText(t, request)     ' unreachable but total
  END MATCH
  s.state.sentAll = TRUE
  RETURN s
END FUNC
```

The binding widens the concrete variant into the union and default-inits `PendingState` (plan-74);
`RETURN s` carries the STATE (plan-74 stateful-union return). `write` before setting `sentAll`.

### 4.2 `__http_ready` / `__http_pump`

```
FUNC __http_ready(RES s AS http::Stream STATE PendingState) AS Boolean
  MATCH s
    CASE net::Socket(p)    : RETURN net::poll(p, 0)
    CASE net::TlsSocket(t) : RETURN tls::poll(t, 0)
  END MATCH
END FUNC

SUB __http_pump(RES s AS http::Stream STATE PendingState)
  IF __http_ready(s) = FALSE THEN EXIT SUB          ' non-blocking: nothing to read now
  MATCH s
    CASE net::Socket(p)
      MUT chunk AS List OF Byte = net::read(p, 65536) TRAP(e)
        IF e.code = errorCode::ErrConnectionClosed THEN
          s.state.closed = TRUE
          RECOVER []
        END IF
        s.state.err = e.code
        RECOVER []
      END TRAP
      IF len(chunk) = 0 THEN s.state.closed = TRUE ELSE s.state.raw = collections::append(s.state.raw, chunk)
    CASE net::TlsSocket(t)
      MUT chunk AS List OF Byte = tls::read(t, 65536) TRAP(e)
        IF e.code = errorCode::ErrConnectionClosed THEN
          s.state.closed = TRUE
          RECOVER []
        END IF
        s.state.err = e.code
        RECOVER []
      END TRAP
      IF len(chunk) = 0 THEN s.state.closed = TRUE ELSE s.state.raw = collections::append(s.state.raw, chunk)
  END MATCH
  IF len(s.state.raw) > __HTTP_MAX_RESPONSE THEN s.state.err = 77050010
END SUB
```

`s.state.raw = collections::append(...)` is the plan-74 in-place STATE mutation through the union
value. (Confirm `s.state.field = …` and whole-field assignment work on a union — plan-74 Phase 3
fixtures cover value-position `.state` writes; verify against a `List OF Byte` STATE field in Phase 1.)

### 4.3 `__http_done` / `__http_finish`

```
FUNC __http_done(RES s AS http::Stream STATE PendingState) AS Boolean
  IF s.state.err <> 0 THEN RETURN TRUE
  IF s.state.closed THEN RETURN TRUE
  RETURN __http_frameComplete(s.state.raw)
END FUNC

FUNC __http_finish(RES s AS http::Stream STATE PendingState) AS Response
  IF s.state.err <> 0 THEN FAIL error(s.state.err, "http stream failed")
  RETURN __http_parseResponse(s.state.raw)
END FUNC
```

### 4.4 Blocking wrappers (rewrite `__http_read`/`__http_write`)

```
FUNC __http_read(url AS net::Url, headers AS Map OF String TO String, method AS String) AS Response
  RES s AS http::Stream STATE PendingState = __http_startRead(url, headers, method)
  WHILE __http_done(s) = FALSE
    __http_waitReadable(s)     ' internal: MATCH → net::poll(p) / tls::poll(t) with omitted timeout (block)
    __http_pump(s)
  END WHILE
  RETURN __http_finish(s)
END FUNC
```

`__http_write` is identical but calls `__http_buildRequest(verb, url, body, TRUE, headers)` inside a
`startRead`-shaped starter (add an internal `__http_startExchange(url, body, hasBody, headers,
method)` that both `startRead` and the write path share, so the request differs only by body).
`__http_waitReadable` is the only place a blocking poll is used; `pump` stays non-blocking, so the
async public API keeps its cooperative semantics.

## Compatibility / Format Impact

- **Changed:** `http` gains `Stream`/`PendingState` types and `startRead`/`ready`/`pump`/`done`/
  `finish`; `read`/`write` are re-implemented over them; http man/spec add the five functions + two
  types. `http.rs` gains descriptor rows only (no native lowering).
- **Unchanged (guardrail):** `http::read`/`http::write` signatures and their returned `Response`
  bytes; the server side; the `Response`/`Request` record shapes; the connect/read timeouts and
  64 MiB cap; the control-byte / chunked handling. `net`/`tls` are unchanged (this sub-plan only
  *calls* the plan-76-B `tls::poll` and existing `net::poll`).

## Phases

> Tick `- [x]` in the same commit as the work. An unticked box means NOT DONE.

> **RESUMED (2026-08-03) — plan-80 landed, D4 fixed.** Phase 1 (the go/no-go) uncovered a
> core-premise defect: plan-74 union STATE was layout-incompatible with a `TlsSocket` variant
> (Corrections **D4** — the STATE ptr at record+16 collided with `SSL*`/dispatch-queue). The chosen
> fix, **plan-80 (unified resource-record header)**, relocated STATE to offset 24 (free in every
> layout) and is now landed + archived (`planning/completed/plan-80-*`), with the D4 defect proven
> fixed at runtime (`tests/rt_macos_d4_union_state_tls.rs`: a `Stream STATE PendingState` union over a
> live TlsSocket binds/mutates STATE/drives TLS I/O with no SIGSEGV). Phases 2–5 are now being
> implemented on this basis. (Phase 1's WIP was reverted after the go/no-go; it is re-created here as
> the real implementation.)

### Phase 1 — falsify the resource-union-over-imported-variants (design uncertainty first)

- [x] Declared `UNION Stream { Socket TlsSocket }` (BARE ids — Correction D1) + `TYPE PendingState
      { sentAll, closed, raw, err }`; registered in `HTTP_TYPES`. **Compiles/resolves/checks** (the
      union over imported variants is valid). (Reverted after the go/no-go — D is deferred.)
- [x] ~~throwaway `__http_streamProbe`~~ — moot: proved the union directly. **Bind + MATCH + drop of
      the union work for BOTH variants** (a stateless `UNION { Socket TlsSocket }` bound to
      `tls::connect` MATCHes/writes/drops cleanly, exit 0). **But `.state` on the union — the whole
      point — FAILS for the TlsSocket variant**: `Stream STATE PendingState` over a TlsSocket writes
      the STATE ptr at record+16 (= `SSL*` in the 32-byte TLS record) → SIGSEGV on https. Proven:
      `http::read("http://…")`=200, `http::read("https://…")`=SIGSEGV. (Also found D3: a `TRAP` inside
      a `MATCH CASE` mis-types its temp as `Unknown`.)
- [x] **STOP per the go/no-go** — the `.state`-on-union path is NOT expressible for a TlsSocket
      variant with the existing machinery. Recorded as core-premise defect **D4**; the fix is
      unscoped architecture work. Surfaced → user chose to **defer D**.

Acceptance: the go/no-go ran and returned **NO-GO** for the plan's STATE-on-union design (D4);
deferred by user decision. ✅ (go/no-go executed; result recorded)
Commit: 100f4ea00 (findings)

### Phase 2 — the five BASIC functions + waitReadable

- [x] Added `__http_startRead` (+ shared `__http_startExchange`), `__http_ready`, `__http_pump`,
      `__http_done`, `__http_finish`, `__http_waitReadable` + the `__http_readNet`/`__http_readTls`
      transport helpers + `PendingState`/`Stream`/`__http_PumpRead` decls to `http_package.mfb`,
      reusing `__http_buildRequest`/`__http_parseResponse`/`__http_frameComplete`. Type-checks
      (`-ast -ir`); codegens (full native build). (Commit 45794c2cb + a3fad7a65.)
- [x] Tests: `tests/rt_http_async_stream.rs` (a Rust integration test — a golden-based rt-behavior
      fixture cannot stand up a live peer; this is the same on-device-loopback deviation the existing
      `http_server_loopback` fixture already documents). Drives a GET via `startRead`/`ready`/`pump`/
      `done`/`finish` against a one-shot Python HTTP peer; asserts `status=200` and the full body
      accumulated across MULTIPLE `pump`s (the server splits the response → `state.raw` growth).
      **PASSES.** (The ≥500× fd-leak loop + the https case are folded into Phase 4's parity test and
      the D4 loopback-TLS proof from plan-80; a dedicated https async drive is a follow-up — see
      Corrections D5.)

Acceptance: the async fixture yields the correct `Response` (multi-pump accumulation works);
`cargo test --bin mfb` green (3774 passed). **MET.**
Commit: 45794c2cb + 4afcd4007

### Phase 3 — descriptor wiring (public surface)

- [x] Added five `hfn(...)` rows to `HTTP_FUNCTIONS` with `Implementation::Rewrite` targets;
      `HTTP_TYPES` rows for `Stream` (Opaque union, like `json::Json`) / `PendingState` (Record);
      `dispatch_resolve` return types (`ready`/`done`→Boolean, `finish`→Response, `startRead`→
      `Stream STATE PendingState`, `pump`→Nothing); `call_param_names`; `expected_arguments`;
      `default_argument_padding` for `startRead`. **Correction D6:** the `ready/pump/done/finish`
      PARAM type is the BASE union `Stream` — a resource value presents its base type at the call
      site and the builtin `resolve_call` path does exact string matching (it does not subsume the
      `STATE` suffix the user-function path strips); the `.mfb` param carries the full
      `Stream STATE PendingState` and plan-74's verifier/codegen resolves it. (Commit 4afcd4007.)
- [x] Tests: a user program driving the async API compiles + runs (`rt_http_async_stream.rs`, both
      the async and the rewritten-blocking client). `tests/syntax/http/http-async-stream-valid`
      (accept: each public call at `-ast -ir`, incl. the `RES … STATE …` param) +
      `tests/syntax/http/http-async-wrongarg-invalid` (reject: a wrong-typed arg).

Acceptance: `http::startRead/ready/pump/done/finish` resolve at `-ast -ir` with the right types; a
user program that drives the async API compiles + runs; `cargo test --bin mfb` green. **MET.**
Commit: 4afcd4007

### Phase 4 — rewrite blocking read/write (compatibility guardrail, last)

- [x] Re-implemented `__http_read`/`__http_write` over `startExchange`+`waitReadable`+`pump`+`done`+
      `finish` (§4.4), deleting the now-unused `__http_exchange{,Tcp,Tls}` (verified unreferenced:
      `rg -n '__http_exchange' src/builtins/http_package.mfb` → only the deleted cluster). One
      read-loop implementation remains. **Correction D7:** `__http_waitReadable` polls WITH
      `__HTTP_READ_TIMEOUT_MS` (not the plan's omitted/infinite timeout) to preserve the bug-268/OS-11
      read deadline — a stalled peer sets `state.err = ErrTimeout`, terminal to `done`/`finish`. TLS
      gains the same 30s deadline (strict improvement). (Commit a3fad7a65.)
- [x] Byte-parity proven at runtime: `rt_http_async_stream.rs::blocking_read_over_the_async_core_
      yields_the_same_response` — the rewritten `http::read` yields the identical `status`+`body` as
      the async path against the same peer. All existing http `cargo test --bin mfb` fixtures pass
      (3774). (No existing golden exercises a live `http::read` round-trip, so the Response bytes were
      never golden-pinned; the parity test now pins them.)
- [x] Regenerated the `.ir`/`.ast` goldens of http-importing fixtures + the http byte-identity
      `.ncodesum` that shifted because `http_package.mfb`/`http.rs` changed — delta is only the async
      surface (proven: `.run` execution goldens unchanged). Full gate green.

Acceptance: every existing http test passes with identical `Response` output; the async and blocking
paths agree; `cargo test --bin mfb`, `scripts/test-accept.sh`, and `scripts/artifact-gate.sh` green.
**MET.**
Commit: a3fad7a65 + (golden regen commit)

### Phase 5 — docs

- [ ] Man pages under `src/docs/man/builtins/http/` for `startRead`, `ready`, `pump`, `done`,
      `finish`, and the `Stream`/`PendingState` types page (follow `.ai/man_template.md` /
      `.ai/man_type_template.md`). Note that `read`/`write` are now thin wrappers.
- [ ] Spec: update `src/docs/spec/stdlib/05_http.md` with the async client and the `Stream` union /
      `PendingState`; if `http::startRead` is a new readiness-adjacent surface, cross-reference the
      timeout convention where `read`/`poll` timeouts apply.

Acceptance: `mfb man http startRead` etc. render; man/spec-citation tests green.
Commit: —

## Validation Plan

- Tests: Phase 1 probe (union feasibility); Phase 2 async-drive rt (http + https + multi-pump +
  leak loop); Phase 3 syntax accept/reject; Phase 4 byte-parity against saved `http::read`/`write`
  goldens (the guardrail).
- Coverage check: the async fixture exercises `startRead`/`ready`/`pump`/`done`/`finish` and the
  union STATE paths; the parity fixtures keep `read`/`write` in the denominator.
- Runtime proof: `http::read(url)` and a `startRead`+pump-loop+`finish` over the *same* URL produce
  the identical `Response` (status/headers/body), for both `http://` and `https://`.
- Doc sync: five http man pages + the types page; `src/docs/spec/stdlib/05_http.md`.
- Acceptance: `cargo test --bin mfb`, `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  (http glob + any http-importing fixtures), `scripts/artifact-gate.sh target/debug/mfb`.

## Open Decisions

1. **`PendingState` fields.** Recommended: `{ sentAll, closed, raw, err }` — add `closed AS Boolean`
   to the prompt's illustrative `{ sentAll, raw, err }` so `done` distinguishes clean EOF from
   failure without overloading `err`. (§3, §4)
   Descision: `{ sentAll, closed, raw, err }` (add `closed`)
2. **Delete `__http_exchange{,Tcp,Tls}` after the rewrite** — recommended: delete if unreferenced
   (the wrappers subsume them), to avoid a second, divergent read loop. Keep only if another caller
   remains. (Phase 4)
3. **`http::startRead` first arg type** — recommended: `net::Url` (as `__http_read` takes today),
   with the public `http::read` already accepting a `Url`; confirm the public `startRead` mirrors
   `read`'s arg contract exactly. (§4.1)
   Descision: `net::Url`

## Corrections

- **D1 (§1, §3, §4.1, Phase 1): declare the union with BARE variant ids `UNION Stream { Socket
  TlsSocket }`, NOT the qualified `{ net::Socket net::TlsSocket }` the plan illustrates.** Union
  variant names are the ONE type position the parser does NOT normalize a `pkg::Type` qualifier on:
  `parse_union_variant` (`src/ast/items.rs:364-369`) calls `parse_qualified_name` (which only turns
  `::`→`.`), skipping the `qualified_builtin_type` normalization every other type annotation gets
  (e.g. `expr.rs:854-859`). So `net::Socket` reaches NIR as `net.Socket`, and EVERY close-wiring
  lookup — `resource_union_cleanup` (`builder_resource_cleanup.rs:63-67`), the 3-site define/used/
  declared (`symbols.rs:161`, `validate/capabilities.rs`, `runtime/usage.rs:121`), and
  `is_resource_type` — keys on the bare id via `BUILTIN_RESOURCES` and silently MISSES: no cleanup
  registered, close helpers never defined/declared → the socket LEAKS with no diagnostic. The shipped
  plan-74 fixtures (`resource-union-state-*`) all declare `UNION Stream { File Socket }` with bare ids
  over cross-package variants (`File` from fs, `Socket` from net), so bare imported-builtin variants
  are the PROVEN path. Consequently the MATCH `CASE` labels in §4.1–4.4 must also be bare (`CASE
  Socket(p)` / `CASE TlsSocket(t)`), not `CASE net::TlsSocket(t)`. (Alternative — add
  `qualified_builtin_type` normalization to `parse_union_variant` — is deferred; not needed if we use
  bare names.) Measured by the D-Phase-1 research audit.
- **D2 (§3, Verified properties): `native_resources` is irrelevant here; close resolution is
  registry-based.** The plan's Phase-1 risk cites [[imported-package-resource-two-spellings]] (decoded
  `.mfp` packages carry no `native_resources`). But `Socket`/`TlsSocket` are BUILTIN resources
  resolved through the always-present `BUILTIN_RESOURCES` registry (`resource.rs:138-239`), not
  through `native_resources` (which only carries user `RESOURCE T CLOSE BY` decls). And `http` is
  source-injected (`augmented_project`, `mod.rs:67`), not `.mfp`-decoded. So the imported-variant
  close-op wiring resolves through the same registry the plan-74 fixtures already exercise — no
  `native_resources` involvement. The real (and only) hazard is D1's qualified-name spelling.
- **D3 (Phase 2, §4.2): a trap-bound producer INSIDE a `MATCH CASE` body lowers its temp as
  `Unknown` — a pre-existing MATCH×TRAP codegen bug.** The plan's `pump` (§4.2) puts
  `MUT chunk = net::read(p, 65536) TRAP(e) … END TRAP` directly inside `CASE Socket(p)`. That fails
  native codegen with `native plan has no storage class for type 'Unknown'` (the TRAP desugars to a
  temp local whose type isn't registered in the MATCH-CASE scope). The SAME `net::read … TRAP …
  RECOVER []` works fine at a function's top level (e.g. the pre-existing `__http_exchangeTcp`).
  **Workaround (shipped):** factor each transport's read+TRAP into a top-level helper
  (`__http_readNet`/`__http_readTls` returning a `__http_PumpRead` record), and have `pump`'s MATCH
  CASE only CALL the helper — no TRAP in the CASE. This is cleaner anyway. The underlying compiler bug
  (TRAP inside MATCH CASE) is tangential to plan-76 and is left as a documented follow-up (repro:
  a `MATCH` CASE containing `MUT x AS T = <call> TRAP(e) … END TRAP`). Measured by bisecting the
  Phase-2 `__http_pump` body.
- **D4 (§1, §3, Phase 1 — CORE-PREMISE DEFECT): plan-74 union STATE is layout-incompatible with a
  `TlsSocket` variant, so `Stream STATE PendingState` over `{Socket, TlsSocket}` SIGSEGVs on the
  https path.** plan-74 stores the STATE-block pointer in the ACTIVE variant's record at
  `FILE_OFFSET_STATE = 16`, and its 80-byte File-layout record (used by `File`/`Socket`) has a free
  slot there. But the `TlsSocket` record is only 32 bytes with `TLS_OFFSET_SSL = 16` (`SSL*`) — so
  binding the union to a TlsSocket and writing STATE clobbers `SSL*`; the next `tls::*` dereferences
  garbage → SIGSEGV (exit 139). **Proven:** `http::read("http://…")` (Socket variant) returns 200;
  `http::read("https://…")` SIGSEGVs; and a STATELESS `UNION Strm { Socket TlsSocket }` bound to a
  `tls::connect` MATCHes + writes + drops cleanly (exit 0) — isolating the fault to the STATE
  mechanism, not the union itself. (macOS `REC_QUEUE`@16 and schannel's STATE-ptr@16 collide the same
  way.) plan-74's fixtures only ever exercised File-layout variants (`File`/`Socket`), so this was
  never caught. The design in §1/§3 (STATE carried ON the union) rests on this false premise.
  **Resolution is an architectural fork — surfaced to the user** (see the plan-76 status): (A) make
  all three TLS record layouts STATE-compatible (grow to the 80-byte File layout, STATE@16, relocate
  SSL/CTX/queue) — a large cross-backend change touching every tls record access + all tls goldens;
  (B) redesign plan-74 union STATE to live outside the variant record; or (C) redesign D to a
  STATELESS `Stream` union with `PendingState` threaded as an explicit `MUT` param through
  ready/pump/done/finish (works with existing machinery today; keeps URL-transparency; changes the
  public signatures from `RES … STATE PendingState` to `RES Stream` + `MUT PendingState`, overriding
  the plan §3 rejection of "state outside the resource", which was made on the now-false assumption
  that STATE-on-union works for a TlsSocket). Measured via the http:// vs https runtime split + the
  stateless-union probe; layout via `FILE_OFFSET_STATE=16` vs `TLS_OFFSET_SSL=16`,
  `RESOURCE_RECORD_SIZE=80` vs `TLS_RECORD_SIZE=32`.
  **CHOSEN (user, 2026-08-02): fork (A), landed as its own plan — `planning/plan-80-unified-resource-record.md`.**
  plan-80 gives every resource one canonical header with `STATE` at offset 24 (free in every layout)
  and adds the per-backend `STATE`-slot assert whose absence caused D4. D is now a HARD dependent of
  plan-80 (see the header `Depends on`): D resumes once plan-80 Phase 4 (the `STATE@24` / D4 gate) is
  green, at which point this design-gate row flips to MET. Forks (B) and (C) rejected in favor of (A)
  because (A) makes resource STATE correct for *every* resource, not just D's `Stream`.

## Summary

The motivating consumer of plan-74's union STATE: a `Stream` resource union over `net::Socket`/
`net::TlsSocket` carrying a `PendingState`, driven non-blockingly by five BASIC functions over
existing `net`/`tls` primitives (plus plan-76-B's `tls::poll`). Risk is front-loaded into Phase 1
(does a resource union over *imported* variants bind/match/drop?) and back-loaded into Phase 4 (the
rewritten `read`/`write` must stay byte-identical). No native lowering is added — only `.mfb` bodies
and `http.rs` descriptor rows. Untouched: the `http` server, the `Response`/`Request` shapes, the
`net`/`tls` packages, and cross-thread transfer (plan-75 stays out of scope).
