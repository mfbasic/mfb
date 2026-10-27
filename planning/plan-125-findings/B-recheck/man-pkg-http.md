### 1. Routing never looks at the request method
UNIT:      man-pkg:http
PAGE:      package-wide (handleRequest, route, types)
CATEGORY:  coverage
CLAIM:     handleRequest "Matching. Routes are tested in list order and the first match wins. Path matching is segment-based on the decoded path…" — no page says what happens to the method.
VERDICT:   missing
EVIDENCE:  `src/codegen/builtins/http/helper_dispatch.rs` `__http_dispatch`: the only test is `__http_matchPath(r.pattern, req.path)`; `req.method` is never consulted. `grep -n -i method helper_dispatch.rs helper_match_path.rs` prints nothing. A developer arriving with "how do I serve GET and POST on one path differently" gets no answer from the overview, handleRequest, or route.
SUGGESTED: On handleRequest "Matching" and route "Ordering": "A route matches on path only; the method is not part of the pattern. A handler that must distinguish GET from POST reads `req.method` (always uppercased) and answers, for example, `http::status(405, "Method Not Allowed")` itself."

### 2. The default User-Agent and Accept values are never stated
UNIT:      man-pkg:http
PAGE:      read, write, startRead (headers parameter)
CATEGORY:  coverage
CLAIM:     "Names matching Host/User-Agent/Accept override the defaults case-insensitively"
VERDICT:   missing
EVIDENCE:  `src/codegen/builtins/http/helper_build_request.rs:26-27`: `"User-Agent: " & __http_headerValue(headers, "User-Agent", "mfb-http/1")` and `"Accept: " & __http_headerValue(headers, "Accept", "*/*")`. `grep -rn "mfb-http" src/codegen/builtins/http/func_*.rs` prints nothing, so no rendered page tells the developer what their program identifies itself as on the wire.
SUGGESTED: In the headers parameter description on all three pages: "The automatic headers are Host (from the URL), User-Agent: mfb-http/1, and Accept: */*; a caller entry with one of those names (case-insensitive) replaces it."

### 3. Four pages say "you still close" the stream, but no page says how
UNIT:      man-pkg:http
PAGE:      ready, done, pump, finish, startRead
CATEGORY:  consistency
CLAIM:     ready/done: "leaves it to you to close"; pump: "leaves it open — you still close it"; versus startRead: "the socket is closed exactly once when the handle goes out of scope" and finish: "its socket is closed exactly once when its binding goes out of scope".
VERDICT:   inconsistent
EVIDENCE:  There is no `http::close` (function table, `mfb man http --all`). The parameter rows promise an explicit close the developer must perform; the descriptions say scope exit does it. Nothing shows the early-close form. Probe: `/tmp/plan-125-scratch/B-recheck/man-pkg-http/p2/main.mfb` (MATCH on the union, `tcp::close(p)` / `tls::close(t)` per CASE) compiles with `mfb build p2`; so does `p1/main.mfb` calling `tcp::close(s)` directly on the union. Runtime behaviour of either UNVERIFIED — needs a socket, NEEDS-NETWORK-PROBE.
SUGGESTED: Make the four parameter rows say "The stream stays open; it is closed when its binding leaves scope." and add one sentence on startRead: "To close it earlier, MATCH the stream and call tcp::close or tls::close on the variant" (or whichever form the network probe confirms).

### 4. startRead's method rule is weaker than read/write's, but all three share one validator
UNIT:      man-pkg:http
PAGE:      startRead (method parameter and Description)
CATEGORY:  consistency
CLAIM:     "Must be non-empty and contain no space. Defaults to GET." — read and write say "no space or control byte", and startRead's Description lists control-byte rejection for headers, target and Host but not the method.
VERDICT:   inconsistent
EVIDENCE:  `src/codegen/builtins/http/helper_normalize_method.rs` `__http_normalizeMethod`: `IF strings::contains(method, " ") OR __http_hasControlBytes(method) THEN FAIL error(77050002, ...)`; it is the single helper behind all three entry points.
SUGGESTED: Copy read's wording: "Must be non-empty and contain no space or control byte."

### 5. read/write say the per-read deadline is plaintext-only; the code applies it to HTTPS too
UNIT:      man-pkg:http
PAGE:      read, write (also startRead's "the blocking wrappers' internal readiness wait bounds a stalled peer")
CATEGORY:  consistency
CLAIM:     "The client applies a 30-second connect deadline and, for plaintext, a 30-second per-read deadline"
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/http/helper_wait_readable.rs` `__http_waitReadable`: `CASE tcp::Socket(p) rdy = tcp::poll(p, __HTTP_READ_TIMEOUT_MS)` and `CASE tls::Socket(t) rdy = tls::poll(t, __HTTP_READ_TIMEOUT_MS)`, then `s.state.err = errorCode::ErrTimeout` when FALSE; `helper_limits.rs:21` sets `__HTTP_READ_TIMEOUT_MS = 30000`; callers are `func_read.rs`, `func_write.rs`, `func_finish.rs` (`grep -ln __http_waitReadable`). `mfb man tls poll` confirms a positive timeoutMs "waits up to that long". So a stalled HTTPS peer also fails with ErrTimeout after 30 s. Runtime confirmation NEEDS-NETWORK-PROBE. The "for plaintext" qualifier dates from before the shared drive loop (`git log -S"for plaintext, a 30-second"` → 17409be9b, the package split).
SUGGESTED: "…and a 30-second per-read deadline on both transports, so a stalled or black-holed peer fails with ErrTimeout rather than wedging the calling thread".

### 6. status page names the 400 response "400 Bad http::Request"
UNIT:      man-pkg:http
PAGE:      status
CATEGORY:  consistency
CLAIM:     "404 Not Found for an unmatched route, 400 Bad http::Request and 413 Payload Too Large for framing failures"
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/http/func_status.rs:48` literally reads `` `400 Bad http::Request` ``; handleRequest and respondPath say "400 Bad Request". The reason phrase on the wire is "Bad Request" (handleRequest "Emission").
SUGGESTED: "400 Bad Request".

### 7. The types page leaves PendingState undescribed and contradicts startRead on sentAll
UNIT:      man-pkg:http
PAGE:      types
CATEGORY:  discoverability
CLAIM:     http::PendingState has no record-level description (every other record has one); its sentAll row says "Request fully written (reserved for a future write-pump)."; http::Stream's union entry says nothing about RES or STATE.
VERDICT:   inconsistent
EVIDENCE:  `src/codegen/builtins/http/mod.rs:267-269` `name: "PendingState", description: ""`. startRead says "The whole request is written before startRead returns; state.sentAll is TRUE" — so the field is already meaningful, and "reserved for a future write-pump" is roadmap text aimed at a contributor (§3). A developer who lands on `mfb man http types` cannot tell from the Stream or PendingState entries that the declaration is `RES s AS http::Stream STATE http::PendingState`, which every client example uses.
SUGGESTED: PendingState description: "The STATE a non-blocking client stream carries: read it through the handle (s.state.raw, s.state.err) while driving it with http::ready/pump/done; declared as `RES s AS http::Stream STATE http::PendingState`." sentAll: "TRUE once the request has been written; startRead always returns with it TRUE."

### 8. The overview's constructor list omits http::bytes
UNIT:      man-pkg:http
PAGE:      overview
CATEGORY:  overview-mismatch
CLAIM:     "the response constructors (responseDefault/ok/status/json/withHeader/respondFile/respondPath) build a http::Response"
VERDICT:   missing
EVIDENCE:  `mfb man http --all` function table lists http::bytes; its own page says it exists "for the case where you are editing a body directly — typically with WITH on an existing response". The overview narrative never mentions it, so the one helper a WITH-based edit needs is the only function with no path from the overview prose.
SUGGESTED: Append "…and http::bytes encodes a String into the byte list a body field holds, for WITH edits."
