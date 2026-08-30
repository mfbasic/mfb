Cleaned up codegen

- = Not reviewed
+ = Started
@ = Reviewed

[@] app
[@] astrings
[@] audio
[@] bits
[@] collections
[@] crypto
[@] csv
[@] datetime
[@] encoding
[@] errorcode
[@] fs
[@] http
[@] io
[@] json
[@] math
[@] money
[@] net
[@] os
[@] process
[@] regex
[@] strings
[@] term
[@] thread
[@] tls
[-] vector

---

# Cleanup investigation (2026-08-23, read-only survey)

## Q1 — After tls migration, can the deprecated registry items be removed cleanly?

**No. tls is irrelevant to them — it's already fully migrated.**

- tls is already on the clean-room registry: every member (`func_connect/listen/accept/
  read/write/poll/close`) uses `Body::native_os_seam` + the generic `native::lower_tls_helper`
  dispatcher (`src/codegen/builtins/tls/native/mod.rs:365`). It calls none of the deprecated
  shims (its only reference to one is inside a code comment).
- The 7 deprecated items live in `src/codegen/registry/mod.rs` and are doc-comment markers
  (`/// #[deprecated(note="migrate registry()...")]`), NOT real `#[deprecated]` attributes —
  so the build does not warn on them. The real migration is an API-shape change: route callers
  through the `registry()` accessor instead of these `Box::leak`-ing free-function shims.
- Each still has a live NON-tls production caller (removal is blocked on repointing these):
  - `call_return_type` (:1849)            → `src/builtins/mod.rs:351`
  - `rewrite_target` (:2243)              → `src/ir/lower.rs:2986`
  - `argument_types` (:2505)              → `src/builtins/mod.rs:423`
  - `call_param_names` (:2629)            → `src/builtins/mod.rs:653`
  - `call_param_name_overloads` (:2667)   → `src/builtins/mod.rs:625`
  - `default_argument_padding` (:2694)    → `src/ir/lower.rs:2805`
  - `resource_close_function` (:2227)     → NO production caller; only `#[cfg(test)]` refs in
    audio/os/process. Closest to dead. (Naming trap: distinct from the still-live
    `builtins::resource_close_function` wrapper → `resource::builtin_resource_close_function`.)
- Separate, also-not-tls deprecation markers gated on crypto/strings/collections/vector
  SOURCE-GENERICS work: `codegen/builtins/encoding/mod.rs:235`, `collections/mod.rs:215`,
  `vector/mod.rs:80`.

## Q2 — Any remaining hardcoded registers, or all moved to vreg?

**The compiler's own codegen path is fully vreg. Leftovers are only in hand-written platform
emitters (tracked as bug-387).**

- Clean: neutral instruction stream, all per-arch code plans (`linux_aarch64/linux_riscv64/
  linux_x86_64/win_x86_64` `{code,plan}.rs`), shared lowering. Flows as vregs (`Operand::vreg`),
  typed ABI tokens (`Operand::abi`), or `%`-sentinels; realized to physicals ONLY at two
  legitimate seams: `src/target/shared/abi.rs` `realize_abi_token` (:381-443) and
  `src/arch/*/select.rs` + `regmodel.rs`. (x86/riscv backends carry no x86/riscv literals —
  they remap the AArch64-spelled neutral stream.)
- Leftovers run BELOW the register allocator, so their target is neutral ABI tokens
  (`LOCAL`/`SCRATCH`/`FP_SCRATCH`/`c_arg`), NOT vregs. `Asm` helpers already accept
  `impl Into<Operand>`, so raw `"xNN"` strings are pure leftover:
  - `src/target/linux_gtk/term_draw.rs` — LARGEST gap, ~110+ literals (callee-saved x19-x28,
    scratch x9-x17, FP d0-d3) intermixed with tokens; clearly mid-conversion.
  - `src/target/linux_gtk/bootstrap.rs` — ~27 lines (x9/x10/x11/x13/x19, two raw sp).
  - `src/target/macos_aarch64/app/{bootstrap.rs,term_view.rs,mod.rs}` — only C-arg staging
    x0-x3; callee-saved bank already migrated. Low risk.
  - `src/target/macos_aarch64/tls.rs` — x1/x2 in the arg-reg→context-offset table.

## Q3 — Move ParameterType to an integer enum from IR downward?

**Yes, startable — but the enum already exists; the task is pushing its boundary UP, and the
right model is string-interning inside the existing structural enum, NOT a flat integer enum.**

- `ParameterType` (`src/types.rs:22`) is already a structural enum and the internal currency of
  the codegen registry. String-based today: IR (`src/ir/*`), monomorph (`src/monomorph/*`), and
  the registry's own boundary, which round-trips string → `parse` → unify/substitute → `name()`
  → string per call (`ParameterType::parse` @ `src/types.rs:146`, 70 call sites).
- Measured hotspots this eliminates (non-test greps): 111 scalar-name string `==`; 218
  structural prefix matches (`strip_prefix("List OF ")` etc.); 698 `type_:/returns: …to_string()`
  allocation sites; 49 `format!("List OF …")`; 17 `IrValue` variants each carrying
  `type_: String` (52 alloc sites in `src/ir/lower.rs` alone); 675 `.type_` accesses.
- NOT a flat enum: records/unions/user types (`Named`) and generics (`Var`) are open sets and
  types nest (`List OF Foo`). Correct model = structural enum with an interned handle at the
  nominal/var leaves. Precedent: `binary_repr` interns type names into a `type_id` table
  (`src/binary_repr/builder.rs:82`).
- BLOCKER to fix first: interning is currently `Box::leak` (`src/types.rs:216`) — fine at the
  low-frequency registry boundary, but leaks per-IR-node if pushed down. Replace with a real
  interner returning `Copy Symbol(u32)`/`TypeId` (also makes Named/Var compares integer compares
  and the enum cheap to clone).
- Recommended start order:
  1. Interner → `Copy Symbol`; Named/Var hold Symbol; keep parse/name. (Localized to
     `src/types.rs` + call sites.)
  2. Convert registry boundary to pass/return `ParameterType`, shrinking the 1137-line
     `resolve_call` string matcher (`src/codegen/builtins/general/mod.rs:287`).
  3. Flip IR `type_`/`returns`/`kind` (`src/ir/types.rs`, `src/ir/value.rs`) String→ParameterType,
     converting ONCE at cut point `ir::lower_augmented_project` (`src/ir/lower.rs`); update
     `ir::verify`, `binary_repr`, codegen; keep `name()` only at serialize seams
     (`src/ir/binary.rs`, `src/ir/json.rs` — wire format stays string for ABI stability).
  4. (Later, separate phase) Give monomorph a typed representation.
- CAVEAT: monomorph runs on the AST BEFORE IR (`src/cli/build/mod.rs:332` precedes `:416`) with a
  parallel string type system (`src/monomorph/helpers.rs:41,171`). An IR-only cut leaves monomorph
  string-based → does NOT speed up monomorphization. If ever unified onto `ParameterType`,
  reconcile `MapEntry OF`/`Result OF` first (monomorph models them structurally; `parse` doesn't).

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
- `EdDSA` with Ed25519 using `crypto::sign`/`verify`.
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
- `EdDSA`
- optionally `ES256/384/512` after implementing and heavily testing DER↔raw conversion

The verifier should require an explicit algorithm allowlist and reject `alg: "none"`, algorithm/key-type mismatches, duplicate security-sensitive claims, malformed token segment counts, and noncanonical Base64url. Do not choose the verification algorithm solely from the untrusted header.

For full JWT ecosystem compatibility, the most valuable additions to `crypto` would be standard key import/export plus RSA-PSS/RSA-PKCS#1 verification.

---

# websockets

Not quite. You can build a standalone RFC 6455 WebSocket implementation over `net`/`tls`, but the existing `http` package cannot perform or hand off an upgraded connection.

What you already have:

- Raw TCP and TLS byte streams with full-buffer writes.
- Partial-read semantics suitable for framed protocols.
- Polling and configurable timeouts.
- Client and server TLS with certificate verification.
- Secure randomness for client masking keys and `Sec-WebSocket-Key`.
- Base64 encoding, UTF-8 validation, bitwise operations, and 64-bit integers.
- Enough collection support to maintain a receive buffer and parse fragmented frames.

What is missing or awkward:

- SHA-1. The WebSocket handshake requires:

  `Base64(SHA1(Sec-WebSocket-Key + GUID))`

  `crypto` exposes only SHA-2. You could implement SHA-1 inside the WebSocket package using `bits`, but a narrowly documented `crypto::hash(Hash.SHA1, ...)` would be much cleaner. SHA-1 is safe here because RFC 6455 uses it as a handshake transform, not for collision resistance.

- HTTP connection upgrade/hijacking. `http::handleRequest` owns and closes the accepted socket, forces `Connection: close`, and only gives handlers a parsed `Request`. It cannot return the socket or unread buffered bytes.

- HTTP client upgrade support. `http::startRead` also sends `Connection: close` and drives the connection toward a complete HTTP response. It does not expose a “101 received; take this stream” operation.

- `ws://` and `wss://` URL parsing. `net::toUrl` accepts only `http` and `https`. A package can translate the schemes before parsing, but native support would be preferable.

- TLS concurrency. `TlsSocket` is documented as not thread-sendable and cannot be stored in collections. That significantly limits a concurrent `wss://` server, even though a single-connection or polling design is possible. Plain `net::Socket` has better multiplexing support.

Important implementation requirements:

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

- A self-contained, blocking WebSocket client: yes, after implementing SHA-1 and the HTTP handshake yourself over `net`/`tls`.
- A standalone WebSocket server: yes, with the same work.
- WebSockets integrated into the existing HTTP router: no.
- A scalable threaded secure WebSocket server: currently constrained by `TlsSocket` ownership/thread limitations.

The two highest-value platform additions would be an HTTP upgrade API that returns the live transport plus buffered surplus bytes, and SHA-1 specifically documented for protocol compatibility. Native `ws`/`wss` URL support would be a smaller convenience improvement.

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

## window test fail on ci

   Compiling pin-project v1.1.13
   Compiling tower v0.4.13error[E0433]: cannot find `unix` in `os`
  --> tests\cli_fmt_indent_bound.rs:11:14
   |
11 | use std::os::unix::process::ExitStatusExt;
   |              ^^^^ could not find `unix` in `os`
   |
note: found an item that was configured out
  --> /rustc/ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96/library\std\src\os\mod.rs:29:4
   |
   = note: the item is gated here
note: found an item that was configured out
  --> /rustc/ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96/library\std\src\os\mod.rs:84:40
   |
   = note: the item is gated here

error[E0599]: no method named `signal` found for struct `ExitStatus` in the current scope
  --> tests\cli_fmt_indent_bound.rs:34:23
   |
34 |         output.status.signal()
   |                       ^^^^^^ method not found in `ExitStatus`

Some errors have detailed explanations: E0433, E0599.
For more information about an error, try `rustc --explain E0433`.
error: could not compile `mfb` (test "cli_fmt_indent_bound") due to 2 previous errors
warning: build failed, waiting for other jobs to finish...
Error: Process completed with exit code 1.

---

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

## Networking Updates

Rework the current `net` and `tls` packages into the following.

### net package

- `net::lookup(host AS String, [port AS Integer]) AS List OF Address`
- `net::parseQuery(s AS String) AS Map OF String TO String`
- `net::percentDecode(s AS String) AS String`
- `net::toUrl(href AS String) AS Url`
- `net::ping(host AS String, [timeoutMs AS Integer], [ttl AS Integer], [size AS Integer]) AS net::PingResult`  *should Error on permissions*
- `net::ping(address AS net::Address, [timeoutMs AS Integer], [ttl AS Integer], [size AS Integer]) AS net::PingResult` *should Error on permissions*

- enum: `net::PingStatus` - Ok | Timeout | Unreachable | TtlExceeded
- Records: `net::Url`, `net::Address`
  - `net::PingResult`
    status  net::PingStatus
    address net::Address     — the responder
    rttMs   Integer          — round-trip, milliseconds (0 when not Ok)
    ttl     Integer          — TTL of the reply (0 when not Ok)
    size    Integer          — payload bytes echoed back (0 when not Ok)

### tcp package

- `tcp::localAddress(sock AS tcp::Socket) AS net::Address`
- `tcp::localAddress(listener AS tcp::Listener) AS net::Address`
- `tcp::remoteAddress(sock AS tcp::Socket) AS net::Address`
- `tcp::listen(host AS String, port AS Integer, [backlog AS Integer]) AS tcp::Listener`
- `tcp::accept(listener AS tcp::Listener, [timeoutMs AS Integer]) AS tcp::Socket`
- `tcp::connect(host AS String, port AS Integer, [timeoutMs AS Integer]) AS tcp::Socket`
- `tcp::connect(address AS Address, [timeoutMs AS Integer]) AS tcp::Socket`
- `tcp::read(sock AS tcp::Socket, maxBytes AS Integer) AS List OF Byte`
- `tcp::write(sock AS tcp::Socket, bytes AS List OF Byte) AS Nothing`
- `tcp::write(sock AS tcp::Socket, value AS String) AS Nothing`
- `tcp::close(resource AS tcp::Socket) AS Nothing`
- `tcp::close(resource AS tcp::Listener) AS Nothing`
- `tcp::poll(sock AS tcp::Listener, [timeoutMs AS Integer]) AS Boolean`
- `tcp::poll(sock AS tcp::Socket, [timeoutMs AS Integer]) AS Boolean`
- `tcp::poll(socks AS List OF tcp::Socket, [timeoutMs AS Integer]) AS tcp::Socket`
- `tcp::setReadTimeout(sock AS tcp::Socket, timeoutMs AS Integer) AS Nothing`
- `tcp::setWriteTimeout(sock AS tcp::Socket, timeoutMs AS Integer) AS Nothing`

- Resources: `tcp::Socket`, `tcp::Listener`

### udp package

- `udp::localAddress(sock AS udp::Socket) AS net::Address`
- `udp::bind(host AS String, port AS Integer) AS udp::Socket`
- `udp::close(resource AS udp::Socket) AS Nothing`
- `udp::send(sock AS udp::Socket, address AS net::Address, bytes AS List OF Byte) AS Nothing`
- `udp::send(sock AS udp::Socket, address AS net::Address, value AS String) AS Nothing`
- `udp::receive(sock AS udp::Socket, maxBytes AS Integer) AS udp::Datagram`
- `udp::setReadTimeout(sock AS udp::Socket, timeoutMs AS Integer) AS Nothing`
- `udp::setWriteTimeout(sock AS udp::Socket, timeoutMs AS Integer) AS Nothing`
- `udp::poll(sock AS udp::Socket, [timeoutMs AS Integer]) AS Boolean`
- `udp::poll(socks AS List OF udp::Socket, [timeoutMs AS Integer]) AS udp::Socket`

- Resources: `udp::Socket`
- Records `udp::Datagram`

### tls package

- `tls::wrap(sock AS tcp::Socket, mode AS tls::WrapMode, [serverName AS String], [certPath AS String], [keyPath AS String], [caPath AS String]) AS tls::Socket`
- `tls::localAddress(sock AS tls::Socket) AS net::Address`
- `tls::localAddress(listener AS tls::Listener) AS net::Address`
- `tls::remoteAddress(sock AS tls::Socket) AS net::Address`
- `tls::listen(host AS String, port AS Integer, certPath AS String, keyPath AS String, [backlog AS Integer]) AS tls::Listener`
- `tls::accept(listener AS tls::Listener, [timeoutMs AS Integer]) AS tls::Socket`
- `tls::connect(host AS String, port AS Integer, [timeoutMs AS Integer], [serverName AS String]) AS tls::Socket`
- `tls::connect(address AS Address, [timeoutMs AS Integer], [serverName AS String]) AS tls::Socket`
- `tls::read(sock AS tls::Socket, maxBytes AS Integer) AS List OF Byte`
- `tls::write(sock AS tls::Socket, bytes AS List OF Byte) AS Nothing`
- `tls::write(sock AS tls::Socket, value AS String) AS Nothing`
- `tls::close(resource AS tls::Socket) AS Nothing`
- `tls::close(resource AS tls::Listener) AS Nothing`
- `tls::poll(sock AS tls::Listener, [timeoutMs AS Integer]) AS Boolean`
- `tls::poll(sock AS tls::Socket, [timeoutMs AS Integer]) AS Boolean`
- `tls::poll(socks AS List OF tls::Socket, [timeoutMs AS Integer]) AS tls::Socket`
- `tls::setReadTimeout(sock AS tls::Socket, timeoutMs AS Integer) AS Nothing`
- `tls::setWriteTimeout(sock AS tls::Socket, timeoutMs AS Integer) AS Nothing`

- enum: `tls::WrapMode` Server, Client
- Resources: `tls::Socket`, `tls::Listener`

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

- **Operators** — `HirExpression::Binary/Unary { operator: String }` (`src/hir/mod.rs:420,426`), `IrValue::Binary/Unary { op: String }` (`src/ir/value.rs:123,132`). The source token itself is carried and re-decided: `operator == "&"` (`src/ir/lower.rs:2368`), `== "NOT"` (`:2392`), `== "-"` (`:3596`), `== "SIZEOF"` (`src/ir/shape.rs:383`). 22 sites (`grep -rn 'operator ==\|operator\.as_str()\|match operator' src/ir src/monomorph src/resolver src/codegen | grep -v _tests.rs`).

- **Local/global variable identity** — `IrValue::Local(String)`, `Global(String)` (`src/ir/value.rs:25,26`), `IrOp::Bind/Assign/AssignGlobal { name: String }` (`src/ir/op.rs:8,21,26`), `NirValue::Local(String)` (`src/target/shared/nir/mod.rs:264`). Bindings are matched by name string from HIR to register allocation — no index, no `Symbol`.

- **Call targets / function identity** — `Call { callee: String }` (`src/hir/mod.rs:432`), `IrValue::Call/CallResult { target: String }` (`src/ir/value.rs:57,65`), `NirValue::Call/CallResult/RuntimeCall { target: String }`. Dispatch is string compare plus `split_once('.')` on `"pkg.member"` (27 sites; `src/ir/shape.rs:1769,1834,1854`, `src/ir/lower.rs:2462`) against literals `"thread.start"`, `"net.poll"`, `"process.spawnEnv"`, `"tls.listen"`, `"crypto.sign"`.

- **Member / field names** — `MemberAccess { member: String }` (`src/hir/mod.rs:466`), `HirRecordUpdate.field: String` (`:291`), `IrValue::MemberAccess { member: String }` (`src/ir/value.rs:118`), decided by `member.as_str()` match (`src/ir/lower.rs:2164,2172,2179`).

- **Declaration keywords, as text, inside the IR** — `IrType { kind: String, visibility: String }` (`src/ir/types.rs:6,7`), `IrFunction.kind: String` (`:209`), `IrField.visibility: Option<String>`. `"record"`/`"union"`/`"enum"`, `"public"`/`"private"`, `"function"`/`"sub"` are compared as spellings: 53 sites (`grep -rn 'visibility *== *"\|kind *== *"' src/ | grep -v _tests.rs`).

- **Literal payloads** — `HirExpression::Number(String)` (`src/hir/mod.rs:415`), `IrValue::Const { value: String }` (`src/ir/value.rs:23`), `NirValue::Const { value: String }`. Numeric literals stay source text and are re-parsed downstream: 24 `parse::<f64>/<i64>/<u64>` in `src/ir`+`src/codegen`.

- **The entire C FFI type vocabulary — a second type domain plan-111 never touched** — `ctype: String` (`src/ir/link.rs:97,432,872`; `src/ast/types.rs:299,364,366,378,388`). `"CString"`/`"CPtr"`/`"CInt32"`/`"CBuffer"`/`"CVoid"` decided by spelling in 19 places. Invisible to the gate by construction: its needle vocabulary is the MFBASIC scalars only. `src/hir/mod.rs:18-21` says why — the native-binding nodes are "reused verbatim from `crate::ast`", so these strings flow straight to codegen.

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

