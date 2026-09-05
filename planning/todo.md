
# Cleanup investigation (2026-08-23, read-only survey)

## Q4 — What other areas are a mess? (ranked next cleanup targets)

1. **syntaxcheck vs ir::verify — two overlapping semantic-check passes.** Documented,
   half-finished migration (`src/rules/mod.rs:5-11`, plan-20-Z): "not-yet-relocated" vs
   "relocated" rules. Mirrored filenames (resources/types/link ↔). Rule codes: 58 in
   syntaxcheck vs 118 in ir/verify — actively moving, neither empty. ~19k lines, duplicated
   traversal, goldens pinned to transitional ordering. Finish relocation + delete syntaxcheck
   half = single biggest structural simplification.
2. **Three hand-written app/terminal runtimes, no shared layer** (overlaps Q2/bug-387). Codegen
   targets already unified via `target/linux_common/` (bug-321), but app runtimes were not:
   `macos_aarch64/app/` (8,002 LOC), `win_x86_64/app/` (3,318), `linux_gtk/` (4,813). Terminal
   render + app_io + bootstrap triplicated; plan-13/94/98 keep adding each feature 3×. ~16k LOC.
3. **CLI monoliths + stringly-typed errors.** `cli/build/mod.rs` (3,581), `cli/pkg.rs` (3,296).
   `Result<_, String>` at 484 sites (cli/manifest/resolver/os). Three error mechanisms coexist:
   `rules`+`PendingDiagnostic`, `ast::DocError`, raw `Result<_, String>`. Consolidate tooling side.
4. **os/ per-OS object writers/linkers + dead prototype.** Three stacks (linux 4,233 / windows
   3,500 / macos 3,013) over a thin shared seam; partly inherent (ELF/Mach-O/PE differ). Quick
   win: delete `src/os/windows/link/spike.rs` (426 LOC proof-of-concept PE that writes
   `mfb_spike_proof.txt`, sitting in the linker path).
5. **Hand-rolled JSON serializers.** serde avoided (2 files); ~27 files hand-emit JSON
   (`ir/json.rs` 908, `nir/json.rs` 1,096, `audit/json.rs` 639, …). `src/json.rs` is only a
   shared escaper/parser — no shared value→JSON writer. Mechanical consolidation.
6. **(Diagnose first — likely intentional) src/ir vs target/shared/nir.** Two IR layers; NIR used
   by all backends + ~40 codegen files. Probably deliberate layering (IR → NIR → arch encoders);
   confirm it earns its keep before growing either.

Cross-cutting symptoms (evidence, not targets): `#[allow(clippy::too_many_arguments)]` ×117
(missing context structs, in target/os/syntaxcheck); `#[allow(dead_code)]` ×34 (mostly still in
codegen). TODO/FIXME grep understates debt — this team encodes it in `planning/`/`bug-NN` docs.

Suggested order: #1 and #2 lead (largest, actively worsening). #3 and #5 contained mechanical
wins. #4 spike.rs is a quick delete. #6 diagnose-first.

---

# JWT

Yes—the primitives are sufficient for a useful JWT package, but not a broadly interoperable one.

You can implement now:

- `HS256`, `HS384`, `HS512` using `crypto::hmac`.
- `EdDSA` with both RFC 8037 curves using `crypto::sign`/`verify`: Ed25519 (`crv: "Ed25519"`, 64-byte raw `R || S`) and Ed448 (`crv: "Ed448"`, 114-byte raw `R || S`). Both already return the fixed-width raw form JWS wants, so no signature re-encoding is needed.
- Compact JWT serialization using unpadded `encoding::base64UrlEncode`.
- Claim parsing/serialization using `json`.
- `exp`, `nbf`, and `iat` checks using `datetime::now().seconds`.
- Secure HMAC comparison using `crypto::constantTimeEqual`.

The main missing capabilities are:

- RSA signing and verification, required for `RS256/384/512` and `PS256/384/512`.
- Standard key import/export:
  - PEM
  - PKCS#8
  - SPKI
  - JWK/JWKS
- JWS-format ECDSA signatures. `crypto::sign(P256, ...)` returns ASN.1 DER, while `ES256` requires fixed-width raw `R || S`. A JWT package could write the DER↔raw conversion itself, but it is security-sensitive and cumbersome.
- A clean way to construct NIST-curve keys from JWK `x`, `y`, and `d` values. The current API expects the package-specific `0x04 || X || Y || d` representation.
- Strict/canonical Base64url decoding. The current decoder accepts padded input and discards leftover bits. JWT verification should reject noncanonical encodings to avoid multiple textual tokens representing equivalent bytes.
- Lossless JSON integers. `json::JsonNum` stores a `Float`. Normal epoch timestamps are safe, but arbitrary JWT numeric claims above `2^53` lose precision.

My recommendation is to ship a first JWT package supporting only:

- `HS256`, `HS384`, `HS512`
- `EdDSA` (Ed25519 and Ed448)
- optionally `ES256/384/512` after implementing and heavily testing DER↔raw conversion

The verifier should require an explicit algorithm allowlist and reject `alg: "none"`, algorithm/key-type mismatches, duplicate security-sensitive claims, malformed token segment counts, and noncanonical Base64url. Do not choose the verification algorithm solely from the untrusted header.

For full JWT ecosystem compatibility, the most valuable additions to `crypto` would be standard key import/export plus RSA-PSS/RSA-PKCS#1 verification.

---

# websockets

Still not quite — but the gap narrowed. You can build a standalone RFC 6455 WebSocket implementation over `tcp`/`tls`, and SHA-1 now ships, so the handshake no longer has to be hand-rolled. What remains missing is the same thing as before: the `http` package cannot perform or hand off an upgraded connection.

Updated 2026-08-30 for the `net` split (plan-110): the old monolithic `net` transport surface is gone. `net` now owns only DNS (`net::lookup`), ICMP echo (`net::ping`), URL parsing (`net::toUrl`, `net::percentDecode`, `net::parseQuery`), and the shared `net::Address` record. Byte streams moved to `tcp`, datagrams to `udp`, encrypted streams stayed in `tls`. A WebSocket package would `IMPORT tcp`, `IMPORT tls`, and `IMPORT net` (imports are not transitive, and naming an `Address` requires importing `net` itself).

What you already have:

- Raw TCP (`tcp::read`/`tcp::write`) and TLS (`tls::read`/`tls::write`) byte streams, both with full-buffer writes and a `String` overload on write.
- Partial-read semantics suitable for framed protocols: `tcp::read` is a short read, `tls::read` returns as soon as any plaintext is decrypted.
- Per-socket deadlines on both transports — `tcp::setReadTimeout`/`setWriteTimeout` and `tls::setReadTimeout`/`setWriteTimeout` (the TLS deadlines landed in plan-110-D).
- A readiness **multiplex** on both transports: `tcp::poll(List OF RES tcp::Socket, timeoutMs)` and `tls::poll(List OF RES tls::Socket, timeoutMs)` each return the first ready socket. That is enough to write a single-threaded, many-connection WebSocket server without a thread per client. `tls::poll` also accounts for bytes already buffered inside the TLS layer, which a raw transport poll would miss.
- **SHA-1**: `crypto::hash(Hash.SHA1, ...)` exists and is the standard FIPS 180-4 digest, computed by the portable software core. See the caveat below.
- Client and server TLS with certificate verification (`tls::connect`, `tls::listen`, `tls::accept`).
- Secure randomness for client masking keys and `Sec-WebSocket-Key` (`crypto::randomBytes`).
- Base64 including the URL alphabet (`encoding::base64Encode`/`base64UrlEncode`), UTF-8 validation, bitwise operations, and 64-bit integers.
- Enough collection support to maintain a receive buffer and parse fragmented frames — including `List OF RES tcp::Socket` / `List OF RES tls::Socket` for a connection table. (Resources may be collection *elements* when spelled `RES`; they may never be record *fields*, so a per-connection state record cannot embed its own socket — keep the socket and its state in parallel structures, or in a `RES ... STATE` binding.)

What is missing or awkward:

- **HTTP connection upgrade/hijacking.** Unchanged and still the blocker. `http::handleRequest` accepts the connection, owns the accepted socket, always emits `Connection: close`, drops any handler-set `Connection`/`Content-Length`, and closes the socket by lexical drop on return. A handler only ever sees a parsed `Request` and returns a `Response`; there is no way to get the live socket or the unread buffered bytes back out.
- **HTTP client upgrade support.** Also unchanged. `http::startRead` returns a `RES http::Stream STATE PendingState` — a resource union over `tcp::Socket` and `tls::Socket` — but it always sends `Connection: close` and drives toward a complete HTTP response. There is no "101 received; take this stream" operation, and no way to unwrap the union back into the underlying socket.
- **`ws://` and `wss://` URL parsing.** `net::toUrl` still lowercases the scheme and accepts only `http`/`https`; anything else raises `ErrUnsupported`. A package can rewrite the scheme before parsing (and must then re-apply the 80/443 port default itself, since `toUrl` derives the default from the scheme it saw).
- **The SHA-1 warning.** `crypto::hash(Hash.SHA1, ...)` works, but every source occurrence of `Hash.SHA1` emits the non-fatal `CRYPTO_SHA1_INSECURE` warning (2-203-0136). A WebSocket package would carry one unavoidable warning at its single handshake call site. Nothing suppresses it today; a narrow protocol-compatibility exemption (or a documented `crypto` entry point for handshake transforms) would keep a WebSocket package's build clean.
- **Every socket and listener is thread-sendable (bug-464, resolved 2026-08-31).** Resources are not sendable by default — it is a per-resource opt-in (`THREAD_SENDABLE` on a user declaration, spec §17; the registry `sendable` bit for a builtin), enforced by `require_thread_sendable` on the thread's resource plane (`src/ir/verify/resources.rs:544`). `tls::Socket`, `tls::Listener` and `tcp::Listener` used to be `sendable: false`, so `Thread OF RES tls::Socket TO …` was rejected outright with `TYPE_THREAD_NOT_SENDABLE` (2-203-0063) at the *type declaration*. All five now transfer. The blocker was never the flag: `copy_resource_to_current_arena` carried only the canonical header and **zeroed** the type-specific record tail, so a TLS handle arrived at its receiver with a null session. The registry now declares each resource's live tail slots (`RegistryResource::live_slots`) and the transfer copy carries them, per backend — `SSL_CTX*`/`SSL*` on OpenSSL, an arena SSPI block (deep-copied) on Schannel, the connection ctx / dispatch queue on Network.framework.

End of stream: **both transports raise `ErrConnectionClosed`** — they agree, and a framing loop needs a `TRAP`, not an empty-list check. `mfb man tcp read` used to claim `tcp::read` returned an empty list at EOF and shipped a drain example looping on `len(chunk) = 0`; that was the documentation being wrong about its own emitter, corrected in bug-465, and both contracts are now pinned side by side (`tests/rt-behavior/{tcp/tcp-read-eof-raises-rt,tls/tls-read-eof-raises-rt}`). Do not write a normalizing transport shim for this; there is nothing to normalize.

Writing, though, is **not** symmetric with reading and is the thing to design around: a write to a peer that has gone away is not reported — the second one kills the process with `SIGPIPE` (bug-467). A WebSocket server cannot rely on a `TRAP` around its send path to survive a client that disconnects mid-frame; detect the disconnect on the read side, where the raise is prompt and correct.

Important implementation requirements (unchanged):

- Preserve bytes following `\r\n\r\n`; the first WebSocket frame may arrive in the same read as the handshake.
- Accumulate partial frame headers and payloads across reads.
- Handle 7-bit, 16-bit, and 64-bit payload lengths with overflow and allocation limits.
- Require client-to-server masking and reject masked server frames.
- Generate a fresh unpredictable 32-bit mask per client frame.
- Validate control-frame constraints: FIN set and payload ≤125 bytes.
- Implement continuation frames and fragmented messages.
- Validate text as UTF-8 across the complete fragmented message, not independently per frame.
- Echo ping payloads in pong frames.
- Implement the close handshake and validate close codes/reason text.
- Reject unsupported RSV bits and extensions.
- Apply explicit message/frame size limits.
- Do not negotiate `permessage-deflate` unless compression and its security limits are deliberately implemented.

So the practical verdict is:

- A self-contained, blocking WebSocket client: **yes**, and cheaper than before — the HTTP handshake is still hand-written over `tcp`/`tls`, but SHA-1 is no longer part of the work.
- A standalone WebSocket server: **yes**, same work.
- A *many-connection* server without a thread per client: **yes**, using the `tcp::poll` / `tls::poll` list multiplex over a connection table. This is new; the previous entry predates the multiplex.
- A thread-per-connection plaintext (`ws://`) server: **yes** — `tcp::Socket` is thread-sendable, so an accepted socket can be `thread::transfer`red to a worker. Since bug-464 the accept loop need not stay on one thread either: `tcp::Listener` transfers too, so a program can bind on one thread and accept on another.
- A thread-per-connection secure (`wss://`) server: **yes** since bug-464 — `tls::Socket` and `tls::Listener` are both sendable, so an accepted TLS socket transfers to a worker and the accept loop can itself live on a transferred listener. The single-threaded `tls::poll` multiplex remains the cheaper choice for many idle connections; the two are now a real design choice rather than one option.
- WebSockets integrated into the existing HTTP router: **no**. Unchanged.

The single highest-value platform addition is now an **HTTP upgrade API** that returns the live transport plus buffered surplus bytes — on the server side out of `handleRequest`, and on the client side out of the `http::Stream` union. (Making `tls::Socket` thread-sendable was second on this list; bug-464 did it, so a threaded `wss://` server is no longer blocked.) Native `ws`/`wss` `toUrl` support and a warning-free SHA-1 spelling for protocol handshakes are both smaller conveniences.

---

# Templates (Mustache / Handlebars)

Yes. You have everything needed for a solid Mustache package and most of a Handlebars-style package without builtin changes.

For Mustache, the existing APIs cover:

- Template loading with `fs::readText`.
- Tokenization using `strings::find`, `mid`, and related functions.
- Context representation with `json::Json`.
- Objects, arrays, strings, numbers, booleans, and null.
- Dotted-name lookup through nested `JsonObj` values.
- Sections, inverted sections, and array iteration.
- HTML escaping with `encoding::htmlEscape`.
- Partials supplied through a `Map OF String TO String`.
- Comments, delimiter changes, and standalone-line whitespace handling.
- Deterministic rendering across platforms.

You should implement a real tokenizer/parser rather than using `regex`; the current regex API does not return captures or match lengths, and templates require nesting-aware parsing anyway.

Potential gaps and design decisions:

- Mustache lambdas: JSON cannot contain callable values. You could omit lambdas, document them as unsupported, or define a richer package-owned value union that includes a function. Everything else in the core Mustache specification is feasible.
- Exact escaping: verify `encoding::htmlEscape` against your chosen specification. Handlebars escapes additional characters such as backticks and equals signs; implement those remaining replacements explicitly.
- Partials: accept an in-memory partial map by default. If you offer filesystem loading, use `fs::openWithin` or `fs::isWithin` to prevent `../` traversal.
- Recursive AST representation: if recursive package types become awkward, keep nodes in a flat list with matching section indices. That also avoids repeatedly searching for closing tags during rendering.
- Rendering limits: impose maximum template size, nesting depth, partial depth, and output size. Recursive partials otherwise permit infinite expansion.
- Error locations: track scalar offsets and line/column positions during tokenization so malformed or unmatched sections produce useful diagnostics.

Full Handlebars is more ambitious, but still broadly feasible. You would need package-owned abstractions for:

- A helper registry.
- Positional and named helper arguments.
- Block helpers and inverse blocks.
- Subexpressions.
- Partial parameters and partial blocks.
- `@index`, `@key`, `@first`, `@last`, and parent-context traversal.
- `SafeString` or an equivalent “already escaped” value.
- Strict versus permissive missing-property behavior.

A practical helper signature could resemble:

```text
FUNC helper(
  args AS List OF json::Json,
  named AS Map OF String TO json::Json
) AS TemplateValue
```

where `TemplateValue` distinguishes ordinary text from explicitly safe text. Block helpers need an additional rendering mechanism or a package-defined options/context record.

My recommendation:

- Start with a spec-oriented Mustache package.
- Use `json::Json` as the public context.
- Support variables, unescaped variables, dotted names, sections, inverted sections, comments, partials, delimiter changes, and standalone whitespace.
- Explicitly defer lambdas.
- Later build a separate Handlebars-compatible layer with a richer value type and helper registry.

---

# YAML (JSON-compatible YAML 1.2 subset)

The API should parse text into the existing `json::Json` model:

```mfb
IMPORT yaml
IMPORT json

LET value AS json::Json = yaml::parse(text)
```

Keep file I/O separate:

```mfb
LET text AS String = fs::readText(path)
LET value AS json::Json = yaml::parse(text)
```

That matches the current design: both `csv::parse` and `json::parse` consume `String`; neither owns filesystem access. I confirmed this with `./target/debug/mfb man csv parse` and `./target/debug/mfb man json parse`.

Why external first:

- JSON has a crisp six-variant data model and is already consumed by HTTP support; `rg -n "IMPORT json|func_json" src` finds the HTTP integration.
- CSV has a deliberately narrow, documented representation: `List OF List OF String`.
- Full YAML is substantially less crisp: tags, anchors, aliases, cyclic graphs, merge keys, multiple documents, non-string mapping keys, duplicate keys, and schema-dependent scalar typing cannot all map faithfully to `json::Json`.
- An external package can evolve its compatibility policy without tying that policy to every MFB release.


```mfb
yaml::parse(text AS String) AS json::Json
yaml::parseAll(text AS String) AS List OF json::Json
```

Document these decisions explicitly:

- Mapping keys must be strings.
- Duplicate keys are rejected.
- Multiple documents require `parseAll`; `parse` requires exactly one.
- Custom tags and merge keys are rejected initially.
- Aliases are expanded with depth/node limits; cyclic aliases are rejected.
- Scalars use an explicitly named YAML 1.2 schema.
- Values without a JSON equivalent are rejected rather than silently converted.
- Parser depth, alias expansion, and total-node limits protect against YAML expansion attacks.

---

# JSON Schema Validation

Yes, you can build a useful JSON Schema validator package now. Full Draft 2020-12 conformance would hit two important platform limitations: JSON number precision and regex dialect compatibility.

Straightforward with the current APIs:

- Boolean schemas (`true`/`false`)
- `type`
- `const` and `enum`
- `required`, `properties`, `patternProperties`, `additionalProperties`
- `items`, `prefixItems`, `contains`
- `minItems`, `maxItems`, `uniqueItems`
- String and object size constraints
- `allOf`, `anyOf`, `oneOf`, `not`
- `if`/`then`/`else`
- Local `$defs` and JSON Pointer `$ref`
- `dependentRequired` and `dependentSchemas`
- Structured validation errors containing instance and schema paths

Harder but implementable:

- `$id`-based reference resolution
- Cyclic-reference detection
- `unevaluatedProperties` and `unevaluatedItems`
- `$dynamicAnchor` and `$dynamicRef`
- Remote schema loading and caching
- `format` validation
- Exact deep equality for `enum`, `const`, and `uniqueItems`

The main blockers to strict conformance are:

- Numbers are stored as `Float`. Consequently, JSON integers beyond `2^53`, high-precision decimals, and exact `multipleOf` checks can produce incorrect results. For example, distinct source numbers may become the same `JsonNum`.
- JSON Schema `pattern` uses the ECMA-262 regular-expression model. MFBASIC’s regex package intentionally has its own portable dialect. Many common expressions will work, but claiming standards conformance requires a compatibility audit or a schema-specific ECMA-262 engine.
- Generic URI-reference resolution is not provided. `net::toUrl` accepts only absolute HTTP(S) URLs, whereas `$id` and `$ref` use general relative and absolute URI references. This can be implemented with string processing, but should not be delegated directly to `net::toUrl`.
- Remote `$ref` fetching needs an explicit security policy. Automatically fetching arbitrary URLs creates SSRF, recursion, response-size, and cache-poisoning risks.

A sensible first release would support an explicitly named subset such as “JSON Schema Draft 2020-12 Core” with:

- In-memory schemas
- Local `$ref`/`$defs`
- Validation keywords and applicators
- No remote retrieval
- No dynamic references
- Documented floating-point number semantics
- A documented portable regex dialect

I would design the API around compiled schemas:

```text
LET result = schema::validate(compiled, instance)
```

with errors resembling:

```text
ValidationError {
  keyword,
  instancePath,
  schemaPath,
  message
}
```

Compilation should resolve local references, detect malformed schemas, precompile patterns where possible, and build the bookkeeping needed for `unevaluated*`. Validation failures should be returned as data; malformed schemas and resolver failures should remain distinct errors.

For genuine official conformance, the most valuable underlying improvement would be a lossless JSON numeric representation retaining the original decimal spelling. ECMA-262-compatible regex behavior would be the second major requirement.

---

# test fix

run acceptance under each system in the matrix.

## riscv fail


test cli::build::tests::builtin_codegen_corpora_lower_in_process ... ok

failures:

---- cli::build::tests::mfb_test_host_run_leaves_project_build_dir_untouched stdout ----

thread 'cli::build::tests::mfb_test_host_run_leaves_project_build_dir_untouched' (7543) panicked at src/cli/build/mod.rs:1691:33:
mfb test should pass: ()

---- cli::build::tests::build_project_coverage_test_writes_a_report stdout ----
Wrote coverage report to /tmp/.tmprfwNj0/coverage.html

thread 'cli::build::tests::build_project_coverage_test_writes_a_report' (7485) panicked at src/cli/build/mod.rs:3029:33:
coverage test passes: ()


failures:
    cli::build::tests::build_project_coverage_test_writes_a_report
    cli::build::tests::mfb_test_host_run_leaves_project_build_dir_untouched

test result: FAILED. 3803 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7728.90s

error: test failed, to rerun pass `-p mfb --bin mfb`

---

## Benchmark-equivalence concerns

Some results strongly suggest that not every implementation performs equivalent observable work:

C list operations such as transform, window, zip, and replace
report approximately 0 ms.
C chunks, flatten, drop, and take are also nearly free.
MFBasic copy is about 0.005 ms while C copy is 2.813–18.512 ms.
Python crypto is dramatically faster because modules such as hashlib
call optimized native implementations.
Python regex similarly uses a mature native regex engine.
Even at -O0, a C compiler can eliminate unused results or reduce work when it can prove that it has no observable effect. Every benchmark should therefore:

Consume the complete result.
Compute and validate a checksum.
Keep workload sizes and data identical.
Exclude setup consistently.
Confirm equivalent allocation and copying semantics.
Prevent constant folding and dead-result elimination.
Verify output after timing, rather than printing inside the timed region.
The near-zero C collection timings deserve particular scrutiny before calculating an overall geometric mean.

---

## Benchmark Main performance cliffs

The biggest issue appears to be dynamic collection reduction:

Dynamic reduce: roughly 825–892 ms
Dynamic reduceRight: roughly 872–935 ms
Python equivalents: roughly 49–52 ms
C equivalents: roughly 1.1 ms
That is likely more than normal dynamic-dispatch overhead. It suggests something structural, such as:

copying or materializing the accumulator on every iteration;
repeated boxing/unboxing;
generic callback dispatch through a costly runtime path;
reference-count or state synchronization overhead;
repeated dynamic type checks;
accidental quadratic behavior.
State-backed mutations are another major hotspot:

list (State-Dynamic).set: 1236 ms median
removeAt: 245 ms
prepend: 103 ms
map (State-Dynamic).removeKey: 66 ms
set (State-Dynamic).remove: 70 ms
The progression from plain → record → state strongly suggests mutation bookkeeping, copy-on-write, observer propagation, or state snapshots are dominating.

Base64 is also anomalous:

MFBasic: 154.844 ms
C: 6.910 ms
Python: 4.874 ms
This likely indicates MFBasic is implementing Base64 using high-level per-byte operations while Python and C call optimized native routines.

---

# FS

- add `fs::pathSeparator() as String`
- add `fs::pathListSeparator() as String`
- add `fs::lineEnding() AS String`

---

# MORE FUCKING STRINGS

Straight answer first: plan-111's charter, in its own words (`planning/completed/plan-111-A…md:8`), was *"delete every **type** string after the AST."* It did that and only that. Nothing in it addressed names, operators, keywords, literals, symbols, or machine operands. The gate it left behind (`tests/no_type_strings.rs`) encodes that same narrow scope, which is why it reads green while 899 `== "` / `!= "` sites remain in `src/` (`grep -rn --include="*.rs" '[!=]= *"' src/ | wc -l`) — `ir` 204, `codegen` 212, `arch` 86, `target` 82, `monomorph` 32, `optimizer` 26, `hir` 5, `resolver` 2.

Here is the enumerated census of what is still a string after the AST.

## Still strings

- **Local/global variable identity** — `IrValue::Local(String)`, `Global(String)` (`src/ir/value.rs:25,26`), `IrOp::Bind/Assign/AssignGlobal { name: String }` (`src/ir/op.rs:8,21,26`), `NirValue::Local(String)` (`src/target/shared/nir/mod.rs:264`). Bindings are matched by name string from HIR to register allocation — no index, no `Symbol`.

- **Call targets / function identity** — `Call { callee: String }` (`src/hir/mod.rs:432`), `IrValue::Call/CallResult { target: String }` (`src/ir/value.rs:57,65`), `NirValue::Call/CallResult/RuntimeCall { target: String }`. Dispatch is string compare plus `split_once('.')` on `"pkg.member"` (27 sites; `src/ir/shape.rs:1769,1834,1854`, `src/ir/lower.rs:2462`) against literals `"thread.start"`, `"net.poll"`, `"process.spawnEnv"`, `"tls.listen"`, `"crypto.sign"`.

- **Member / field names** — `MemberAccess { member: String }` (`src/hir/mod.rs:466`), `HirRecordUpdate.field: String` (`:291`), `IrValue::MemberAccess { member: String }` (`src/ir/value.rs:118`), decided by `member.as_str()` match (`src/ir/lower.rs:2164,2172,2179`).

- **Declaration keywords, as text, inside the IR** — `IrType { kind: String, visibility: String }` (`src/ir/types.rs:6,7`), `IrFunction.kind: String` (`:209`), `IrField.visibility: Option<String>`. `"record"`/`"union"`/`"enum"`, `"public"`/`"private"`, `"function"`/`"sub"` are compared as spellings: 53 sites (`grep -rn 'visibility *== *"\|kind *== *"' src/ | grep -v _tests.rs`).

- **Literal payloads** — `HirExpression::Number(String)` (`src/hir/mod.rs:415`), `IrValue::Const { value: String }` (`src/ir/value.rs:23`), `NirValue::Const { value: String }`. Numeric literals stay source text and are re-parsed downstream: 24 `parse::<f64>/<i64>/<u64>` in `src/ir`+`src/codegen`.

- **Machine operands and registers** — `Operand::Raw(Box<str>)` plus `impl From<&str> for Operand` (`src/codegen/engine/operand/operand.rs:163,362`), so any `"x0".into()` mints a string operand. The file's own doc: *"in the pre-allocation stream every register operand is `Raw`"*. Regalloc then re-parses it: `starts_with('%')` (`src/codegen/engine/regalloc/analysis.rs:298,352`), `value.starts_with('%') || value == "sp"` (`src/codegen/engine/regalloc/mod.rs:107`). 465 non-test `.render()`/`.rendered()` calls.

- **Type spellings inside the codegen code plan** — `CodeParam.type_: String`, `CodeFunction.returns: String`, `CodeStackSlot.type_: String` (`src/codegen/engine/types/types.rs:37,23,1332`), filled by rendering a `ParameterType` back out — `param.type_.clone().name().into_owned()` (`src/codegen/engine/function/function_lowering.rs:829`, `src/codegen/link/thunk/link_thunk.rs:1755`) — and by bare literals `"Nothing".to_string()`, `"Integer".to_string()` (`src/codegen/engine/builder/mod.rs:2185,2267,2397`). These contradict plan-111's headline ("`ParameterType` is the compiler's only type currency … to the emitted byte") and pass the gate because the gate has a class for `&str` *parameters* and none for a `String` *struct field*. In fairness: I grepped for readers and found none that decide on them — they terminate in `json_string(&self.returns)` (`src/codegen/engine/builder/code_impl.rs:252`, `src/codegen/engine/mir/mir.rs:901`). Dead carriers, not live decisions — but they are type strings after the AST.

- **Generic instantiation identity** — monomorph names instantiations with a mangled string (`emit$Integer`, `show$List$OF$String`) built by `sanitize_type_name` (`src/monomorph/helpers.rs:546`), documented lossy at `src/monomorph/lower.rs:2352`: "`(`/`)` and `{`/`}` both sanitize to `$`."

- **Link symbol / library identity** — `library: String`, `symbol: String`, `alias: String` (`src/hir/mod.rs:102,104,123`; `src/codegen/engine/types/types.rs:120,121,125`), compared against `"libc.so.6"`, `"libpthread.so.0"`, `"GetStdHandle"`, `"getentropy"`.

- **Target/platform identity** — `NativeCodePlan { target: String, arch: String }` (`src/codegen/engine/types/types.rs:7,11`), compared `== "macos-aarch64"`.

- **Data-object and relocation tags** — `CodeDataObject { kind: String, layout: String, value: String }` (`:126,127,130`); operand-class tags `== "label"`, `== "external"`, `== "symbol"`, `== "data"`, `== "str_u64"` in `src/arch`; and `emit_symbol_ref(kind: &str)` matching `"adrp"`/`"add_pageoff"` (`src/arch/aarch64/encode/emitter.rs:1190-1195`).

- **Ad-hoc positional tags** — e.g. `side(instructions, models, overlay, i, "lhs")` / `"rhs"` (`src/optimizer/opt2/plans/ranges.rs:229-230`).

## Two things that are actually clean

- **Lexer tokens do not survive the AST.** `grep -rn "TokenKind\|lexer::" --include="*.rs" src/ | grep -v "^src/lexer.rs" | grep -v "^src/ast/"` → one file, `src/fmt.rs`, the source formatter working on raw text.
- **Instruction opcodes are typed.** `CodeOp` is a `Copy` enum; the `== "adrp"`/`== "fadd_d"`/`== "lr"` hits in `src/arch` are almost all `#[cfg(test)]` inspection via `op.mnemonic()`, not selection logic. Same for `RuntimeHelper`, `RegClass`, `AbiConvention`/`AbiRole`, `LoopKind`.

So the typed machinery exists and works — it was applied to exactly one vocabulary. No changes made.

---

KeyPair is untagged. A 32-byte pair might be Ed25519 or X25519; a 57-byte pair is Ed448, a 56-byte pair is X448. convert only checks length. encrypt / decrypt take signing keys and convert internally, so a raw X25519 public key of the same length would be run through the Ed→Montgomery map and silently produce garbage (or a box nobody can open). The docs admit this. The type system does not. A Certificate tag on KeyPair, or distinct record types, would make the “no curve tagging” paragraph unnecessary.

Password hashing is the weak spot they already named. The only password KDF is PBKDF2. The docs tell you to prefer Argon2id/scrypt/bcrypt and then do not provide them. For a “software-first” package that already has SHAKE and a bits layer, Argon2id would be the one addition that changes real-world advice from “use this for compatibility” to “use this to store passwords.”

NIST private-key encoding is bespoke. 0x04‖X‖Y‖d is neither raw d nor PKCS#8. Wire-compatible across this package’s platforms, yes; drop-in with OpenSSL PEM/DER, no. That belongs in a migration note next to the size table.

SHA-1 as a hard warning (build still works) is the right severity. HMAC-SHA1 is still fine; hashing with SHA-1 is not. The advisory does not distinguish those, which will annoy people doing HMAC-SHA1 for TLS-era interop.

---

If this is heading toward a v2, I would (1) fix the withZone contradiction, (2) make parse validate or return a distinct unchecked type, (3) add named zones or at least a way to serialize “local at this offset on this host” portably, and (4) give toIso an overload that keeps nanoseconds.

If you want a deeper pass, the highest-value follow-ups are a DST worked example for civil/addDays, or a critique of the format mini-language versus strftime / Temporal.

---

review all man pages - RES can be a a record, RES can be in a collection, RES can transfer threads.
review all RES types - in the thread::transfer | thread::accept man list any that **can't** be transfered and why.

The package intro says every built-in socket and listener may cross:

fs::File, tcp::Socket, udp::Socket, tcp::Listener, tls::Socket and tls::Listener

thread::transfer says the opposite:

fs::File, tcp::Socket and udp::Socket may; listeners and tls::Socket may not

Those cannot both be true. The intro is also the one that motivates “accept on one thread, hand each connection to a worker,” which is exactly the listener/TLS case. Either the intro is aspirational and transfer is current, or transfer is stale. Pick one and make See also match (transfer currently links tls::Socket after forbidding it).

---

process::close is named wrong. It closes the child’s stdin. The docs have to say “does not close the handle” in three places, and the parameter tables still say “you still close it.” In a language where drop is close, this will be the first support question. closeInput / endInput would have been cheaper than the entire “despite the name” paragraph.

---

The documented inconsistencies are more interesting than the APIs. tcp::close errors on double-close; tls::close does not. tcp::listen backlog defaults to 128; tls::listen defaults to 0 (host default). Those are called out as not going to be papered over because each side already has callers. That is rare. Most libraries would “fix” one and break the other quietly.

List-form tls::poll is documented as List OF tls::Socket in one place and List OF RES tls::Socket in another. The RES rule is the real one; the signature table looks like a slip.

---

Inclusive [start, endIndex] ranges plus end being reserved is a BASIC-language scar... Maybe we should do something lanuage wide for all builtins? start/stop, startIdx/endIdx, startIndex/endIndex...

---

displayWidth vs pad width. Padding is in scalars, display is in columns. padLeft("x", 3, "😀") is "😀😀x" — three scalars, five columns. Anyone building a table will hit this. Either that is intentional (pad is a string operation, alignment is the caller’s problem) or there is a missing padToDisplayWidth.
Byte search vs scalar index. contains/count/replace scan bytes; find returns a scalar index. Fine because UTF-8 is self-synchronizing, but count advancing “one byte on mismatch” is an implementation detail leaking into the man page. For well-formed UTF-8 it cannot land mid-scalar, so it is safe — just a bit of C showing through the BASIC.
utf8Encode as a return-type overload. Needing contextual type to pick List OF Byte vs List OF Integer is the sharpest language-level edge in the whole set. strings::toBytes exists as the unambiguous byte path; I would default to that in new code.

---

Absence is the one place it breaks the strings package’s religion. strings::find raises ErrNotFound. regex::find returns -1. The man page even points at the difference. Both are defensible:

literal search: absence is exceptional if you already contains-guarded
regex search: absence is the common case, and -1 is unambiguous because every real index is >= 0

---

The API reports where, almost never what. find/findAll return start indices only. No end index, no match text, no capture list, no Match record. Extraction is replace with $N, or you take the start and re-slice yourself — except you cannot slice without the end. That is the hole. With only starts, findAll("a1b2c3", "\\d+") tells you where digits begin, not how long they are. For a fixed pattern you can reconstruct; for a general one you cannot without matching again or using replace as a poor man’s extractor ("$0" into a side channel you do not have).

---

Empty-pattern replace is the opposite of strings::replace. Documented clearly:

strings::replace("hi", "", "x") → "hi" (empty needle never matches)
regex::replace("abc", "", "-") → "-a-b-c-" (zero-width match at every position, plus the end)

---

Match span (start and end), or the matched substring. Without this, find is half a locator.
Capture access that is not replace. Named groups in the replacement template imply the engine already has them.
split. strings::split is literal-only; regex split is how you tokenize. You can fake it with findAll if you have ends.
AttributedString overloads. Every strings::* query has one. Regex does not. Visible-text search would be the obvious analog; rewrite-with-span-remap is harder and maybe correctly omitted.
count. Trivial given findAll, but strings::count exists and people will look for it.

---


