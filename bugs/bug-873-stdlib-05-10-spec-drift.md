# bug-873: stdlib spec topics http/url/math-rng/encoding/vector drift from the code — two stale line citations and a dozen inaccurate claims

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Other (spec accuracy)

Status: Open
Regression Test: none (doc-only). Re-render with `mfb spec stdlib <topic>` and re-run the cited commands.

Each item below is a claim in `src/docs/spec/stdlib/0{5..9}_*.md` that a command contradicts.
The code is plausibly right and the spec is stale or imprecise (class SPEC) unless marked
**UNCLEAR**, where the owner must decide which side moves. Items already filed as their own
CODE findings by batch B04 are not repeated here.

References: src/docs/spec/stdlib/05_http.md, 06_url.md, 07_math-rng.md, 08_encoding.md,
09_vector.md; `.ai/specifications.md` (citation rules: symbol-preferred, grep-confirm).

## Failing Reproduction / items

Citation check: `python3 /tmp/hardening/B04/cites.py src/docs/spec/stdlib/0{5,6,7,8,9}_*.md src/docs/spec/stdlib/10_*.md` (run from the repo root).

1. **Stale line citation (06_url.md, Rendering model).** `[[src/codegen/builtins/mod.rs:73]]`
   is cited for "a universal `toString(value)` whose argument is a `Url` is routed to
   `__net_urlToString`". Line 73 of `src/codegen/builtins/mod.rs` is now `"color",` inside
   `BUILTIN_IMPORTS`. The routing is the `pkg.add_override(RegistryOverride { … })` in
   `src/codegen/builtins/net/mod.rs`, which `URL_TO_STRING` documents. Replace it with a
   symbol citation (`net/mod.rs:URL_TO_STRING`).
2. **Stale line citation (07_math-rng.md, `math::seed`).** `[[src/codegen/builtins/math/gen_math.rs:883]]`
   points at `self.emit(abi::branch_eq(&overflow));` inside `lower_math_rounding`. The seed
   lowering is `gen_math.rs:lower_math_seed` (`grep -n "fn lower_math_seed"` → line 1073).
3. **06_url.md Userinfo, step 3**: "If the authority contains `@`, the text before it is
   userinfo". The code splits at the **last** `@` (`__net_lastIndexOf`, bug-306 S3).
   `net::toUrl("http://a@b@c/")` → `u=a@b … h={c}` (`/tmp/hardening/B04/url`). The step
   should say "the last `@`".
4. **06_url.md Explicit port validation / Parse failures**: "empty text … each fail" and
   "empty … port" in the 77050003 list. `net::toUrl("http://h:/")` succeeds with
   `port=80`, rendered `http://h/`. `__net_toUrl` calls `__net_parsePort` only when
   `portText <> ""`, so its empty check is unreachable from `toUrl`. That behavior is
   RFC 3986-legal (empty port = default). Fix the spec.
5. **UNCLEAR, 06_url.md Parse failures**: "malformed URL: … empty/unterminated/garbage host".
   No host character is validated. `net::toUrl("http://exa mple.com/")` → `h={exa mple.com}`,
   and `net::toUrl("http://h\r\n/")` → host `h\r\n` (the http client then rejects it at
   `__http_buildRequest`'s control-byte check). Either validate reg-name/IPv4 characters
   or drop "garbage host".
6. **05_http.md Response Size Cap**: "exceeding it fails with `77050010` ("response too
   large")". The code is right (77050010), but the message is `http stream transport failed`
   (`__http_pump` sets `state.err`, `__http_finish` re-raises with that text). Observed: `big FAIL 77050010 http stream transport failed`
   (`/tmp/hardening/B04/async`, 64 MiB + 10 body from a 127.0.0.1 peer).
7. **05_http.md intro / Request parsing**: "reads until end-of-stream" and "the client
   always reads to EOF". `http::done` stops on a satisfied `Content-Length` or final chunk
   (`__http_frameComplete`). See the CODE finding
   `B04-http-client-close-delimited-body-dropped`. The intro should describe the actual
   framing rule once that is fixed.
8. **05_http.md Request Construction**: "empty or whitespace-containing methods fail
   (`77050002`)". Only U+0020 and bytes < 0x20 are rejected (`__http_normalizeMethod`).
   `http::read(u, {}, "GE\u{A0}T")` is sent as the request line `GE\xa0T /m4 HTTP/1.1`
   (logged by the 127.0.0.1 peer). `mfb man http read` already says "no space or control
   byte". Align the spec, or reject non-token characters (RFC 9110 `token`).
9. **UNCLEAR, 05_http.md Chunked transfer decoding**: grammar `chunk = HEX [";" ext] CRLF
   data CRLF` and "Malformed framing … fails with `77050003`". `__http_dechunkBytes` skips
   the 2 bytes after each chunk's data without checking that they are CRLF. A response body
   `4\r\nWikiXX5\r\npedia\r\n0\r\n\r\n` decodes to `Wikipedia` (`/tmp/hardening/B04/httpc`,
   `/chunknocrlf`). The server path uses the same de-chunker (`__http_parseRequest`), so by code inspection a request is accepted the same
   way (not exercised). Either check the CRLF or state that the trailing CRLF is not verified.
10. **05_http.md Lifecycle**: "… a slow or idle client a `408`". A client that connects
    and sends **nothing** is closed after 10 s with no response
    (`rawc2.py idle` → `idle-no-bytes (10.0s) b''`). `mfb man http handleRequest` documents
    "silently closed if not one byte arrived". Say so in the spec.
11. **09_vector.md Dispatch model**: "The compiler resolves the public return type …
    (`vector::resolve_call`), and IR lowering rewrites the public call onto the
    type-specific internal name … (`vector::implementation_name`)". Neither function exists.
    `src/codegen/builtins/vector/mod.rs`'s header says the registry's
    `select`/`rewrite_target` over each overload's `Body::Rewrite` replaced them, and
    `resolve_call` survives only as a comment anchor. Describe the registry mechanism.
12. **09_vector.md Integer rounding rule**: "the `project`/`reject` quotient … rounds half
    away from zero". The quotient is a Float, and each *component* `ratio * b.i` is
    rounded (`__vector_project_integer2`). `vector::project(Integer2[3, 5], Integer2[2, 0])`
    → `(3, 0)`. Rounding the quotient 1.5 first would give `(4, 0)` (`/tmp/hardening/B04/vec`).
13. **09_vector.md slerp**: "Interpolates *direction*, not magnitude". This is the
    un-normalized sin-weighted formula. `vector::slerp(Float2[1.0, 0.0], Float2[0.0, 3.0], 0.5)`
    → `(0.71, 2.12)`: 71.6° instead of the 45° midpoint, and |r| = 2.24. The claim holds
    only for equal-length inputs. Say so.
14. **08_encoding.md Base-N**: Base32 alphabet "uppercase `A–Z 2–7`" with "Decoding
    validates the alphabet". The decoder is case-insensitive:
    `encoding::base32Decode("abcdefgh")` → bytes `00443214c7` (`/tmp/hardening/B04/encx`).
    `mfb man encoding base32Decode` documents "Decoding is case-insensitive". Add it to the spec.
15. **08_encoding.md intro**: "decoders fail closed with `ErrInvalidFormat` (`77050003`)
    on malformed input". `encoding::htmlUnescape("&#xD800;")` raises `ErrEncoding`
    (77020004), which `mfb man encoding htmlUnescape` lists. Either map the surrogate
    reference to 77050003 or name the exception in the spec.

   **Owner direction (2026-10-05):** measure against Node. The `tools/oracles/url/`
   harness also runs chunked bodies through Node's HTTP parser (llhttp) so the CRLF decision
   is made from measured behavior. **Measured: Node rejects it in both directions (`HPE_STRICT`).
   Owner decision: match Node. Code fix in bug-980**, which also covers whitespace, extensions,
   final CRLF, trailers and trailing bytes.

   **Owner decision (2026-10-05): validate hosts, and check all URL handling against
   Node.** `net::toUrl` rejects a host that is not a valid host, and the whole URL surface is
   checked against a differential oracle of Node's `URL` / `URLSearchParams`
   (`tools/oracles/url/`). The oracle's measured divergences decide the remaining details. **Measured and decided: bug-979**
   (345 host-validation cases, plus IPv4 canonicalization and backslash, all decided to match WHATWG).

Verified-correct claims (no action): PCG64 constants/step/XSL-RR/seeding/Lemire bounding
match a Python oracle bit-for-bit for 9 sequences and spawn-seeding. All crypto vectors
match. Codepage tables match the WHATWG index files.

## Root Cause

Spec text not updated when the code changed: bug-306 S3 (last-`@`), the registry migration
(vector), the `__http_finish` re-raise path, and line drift for the two numeric citations.

## Goal

- Each item above reworded (or, for UNCLEAR items, the owner's chosen side implemented) so its command matches the spec text.
- No `[[file:NNN]]` citation in these topics points at an unrelated line.

### Non-goals (must NOT change)

- Code behavior for the SPEC-class items.

## Blast Radius

- Docs only: the five spec files above.
