# bug-477: `tls::connect` has no way to accept a self-signed certificate, so no MFBASIC program can be a TLS client to an MFBASIC TLS server

Last updated: 2026-08-31
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `tests/rt-behavior/tls/tls-connect-self-signed-rt/` (new, Phase 1)

`tls::connect` always validates the peer chain against the host trust store and
offers no argument that relaxes it. `tls::listen` in the same package terminates
TLS with any PEM cert/key pair the caller names — including a self-signed one —
so the two halves of the package cannot talk to each other. There is no
development, test, loopback, private-CA, or air-gapped configuration in which an
MFBASIC client can reach an MFBASIC TLS server; the only way to exercise
`tls::connect`'s success path is to point it at a publicly-trusted third-party
host, which is exactly what every fixture in `tests/rt-behavior/tls/` does today
(five of six connect to `8.8.8.8:443`).

This is filed as a **Footgun**, not a Correctness bug: everything the code does
today is *correct and fails closed*. What is missing is a deliberate, opt-in,
default-off escape hatch. That framing matters, because the tempting fix — make
verification lenient, or add a broad "insecure" switch — would convert a
currently-safe surface into a silent MITM hazard for every existing caller,
including `http::` (which reaches HTTPS through this very member).

**The single correct behavior a fix produces:** `tls::connect` gains one
optional, named, `Boolean` argument defaulting to `FALSE`. At `FALSE` the
handshake is byte-for-byte what it is today. At `TRUE` a chain that fails *only*
because its root is not in the host trust store (self-signed leaf, self-signed
root in chain, or issuer not locally available) is accepted — while the server
**name** check, the certificate validity dates, and the TLS ≥ 1.2 floor stay
enforced, on all three backends. Nothing else in the package changes, and no
existing call site's behavior changes.

## Sequencing: this must land AFTER bug-476 (2026-08-31, coordinator)

bug-476's root cause turned out to be general, not http-specific:
`static_type_name`'s `NirValue::Call` arm is a hand-written table, so an
*untabulated call passed as an overload-selecting argument* answers `None` and
every such selector silently takes its fallback code form. **`tls::connect` is on
the affected list** (host/port vs `Address`), alongside `tcp::connect`,
`tcp`/`tls`/`udp` `write`/`send`/`poll`, `net::ping` and `tls::localAddress`.

Why that blocks this bug specifically: `func_connect.rs:41-44` records that the
two overloads **do not share a positional layout** — `timeoutMs` and `serverName`
are parameters 2 and 3 of the host/port form but 1 and 2 of the `Address` form,
"since one endpoint value replaces two", and named arguments "bind per-overload,
against whichever overload the argument types select".

So today, `tls::connect(addressReturningCall(), …)` selects the wrong form and
its named arguments bind against the wrong positional layout. This bug adds a
**new named `Boolean` parameter that must bind correctly in both forms**.
Implementing it against the broken selection risks encoding the wrong layout, or
shipping a parameter that binds to the wrong slot in the `Address` form —
and the failure would be silent, in a security-relevant flag, which is the worst
possible place for it.

Wait for bug-476 to land, then build on the corrected selection. When it has
landed, add an `Address`-form fixture for the new argument specifically, not only
a host/port one — the two layouts are exactly what makes this member fragile.

## Implementation constraints, added 2026-08-31 (coordinator, pre-dispatch)

This document states the *behavior* correctly. What follows is the **mechanism**
trap: on all three backends the shortest implementation of "accept a self-signed
cert" also silently disables the hostname and expiry checks this document
requires to stay enforced. That converts a fail-closed surface into a silent MITM
hazard, which is the outcome the doc's own framing warns against — so it must be
gated by construction, not by intent.

**The rule for every backend: keep verification ON and *classify the failure*.
Never turn verification off.** The flag accepts a specific set of trust-anchor
errors and nothing else.

**OpenSSL** (`src/codegen/builtins/tls/gen_openssl.rs`). The client today sets
`SSL_set_verify(ssl, SSL_VERIFY_PEER, NULL)` (:538), `SSL_set1_host(ssl, sni)`
(:559) and then requires `SSL_get_verify_result(ssl) == X509_V_OK` (:661).
Note that **`SSL_set1_host` folds the hostname check into the verify result** —
so `SSL_VERIFY_NONE`, or simply skipping the `:661` check, drops the name check
too. The correct shape keeps :538 and :559 exactly as they are and relaxes only
:661, accepting `X509_V_OK` plus **only**
`X509_V_ERR_DEPTH_ZERO_SELF_SIGNED_CERT` (18),
`X509_V_ERR_SELF_SIGNED_CERT_IN_CHAIN` (19) and
`X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY` (20). A name mismatch
(`X509_V_ERR_HOSTNAME_MISMATCH`, 62) and an expiry
(`X509_V_ERR_CERT_HAS_EXPIRED`, 10 / `..._NOT_YET_VALID`, 9) must still fail.

**Schannel** (`gen_schannel.rs`). Verification runs *after* the handshake via
`CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL)` (:7, :51-52). Relax by
masking only `CERT_TRUST_IS_UNTRUSTED_ROOT` / `CERT_TRUST_IS_PARTIAL_CHAIN` in
the chain-policy status — **not** by passing
`SECURITY_FLAG_IGNORE_UNKNOWN_CA`-style blanket flags, and never by clearing
`pwszServerName` (which is what enforces the name) or ignoring
`CERT_TRUST_IS_NOT_TIME_VALID`.

**macOS** (`gen_macos/client.rs`). Do **not** reach for
`kSecTrustOptionAllowExpired*` or `SecTrustSetOptions`. Evaluate as today and
accept only a result whose sole defect is an untrusted anchor
(`kSecTrustResultRecoverableTrustFailure` **plus** a check that the recorded
reason is anchor-related); keep `SecTrustSetPolicies` with the SSL policy and
its hostname so the name check stays live.

**Plumbing constraint.** `http::` reaches HTTPS through this member. The new
argument must **not** acquire an `http::` passthrough in the same change — an
`http::get(url, insecure: TRUE)` is a much larger blast radius than this document
scopes, and it should be a separate, separately-argued decision.

**Test the negative, not just the positive.** A fixture proving a self-signed
cert is accepted at `TRUE` proves nothing about safety. Pair it with fixtures
that must still FAIL at `TRUE`: (a) a cert whose name does not match the host,
(b) an expired cert. Without those two, the implementation that disables
verification wholesale passes the suite. `examples/network-server/certs/` now
ships a self-signed pair usable for (a) via a deliberate name mismatch.

References:

- `mfb spec stdlib transports` → "TLS specifics" (`src/docs/spec/stdlib/17_transports.md:118-125`) — "**The client verifies.** `tls::connect` validates the server's chain against the host trust store … a chain it cannot verify raises rather than connecting." This is the sentence the fix amends.
- `mfb man tls connect` — rendered from `src/codegen/builtins/tls/func_connect.rs:DESC`, which states "The peer's certificate is always verified".
- `.ai/net-tls.md` — networking / TLS readiness / repository-client transport security.
- `bugs/completed/bug-177-net-tls-crypto-robustness-nits.md:38` — the prior audit that certified "no verification bypass exists" on either backend. This bug deliberately introduces one, opt-in; bug-177's finding must be re-stated, not silently invalidated.
- Found while writing `examples/network-client` (the TLS attempt against `examples/network-server --tls` cannot succeed; the example's header comment documents the limitation and points at `--server-name`).

## Phase 1 measurements (2026-09-01, this session)

Every unknown this document flagged, answered by running it. Two of the three
answers contradict the Fix Design above; both are corrected here, and the
corrections are the reason the fix is safe.

### 1. The OpenSSL design in the table above is UNSAFE — measured, not argued

The doc's own hedge ("That is a claim, not a measurement") was right to hedge.
**The claim is false.** `/tmp/b477-verify.c`, linked against OpenSSL 3.6.2 and
run against `examples/network-server --tls certs/cert.pem` on :7413:

```
SSL_set_verify(ssl, SSL_VERIFY_NONE, NULL); SSL_set1_host(ssl, name);
  name = "localhost"      (matches)   -> handshake rc=1, SSL_get_verify_result=18
  name = "wrong.example"  (MISMATCH)  -> handshake rc=1, SSL_get_verify_result=18
```

The two are **indistinguishable**. With a NULL callback the store's default
`verify_cb` returns `ok` (0) on the first error, so `X509_verify_cert` stops
inside `build_chain` and `check_id` — the hostname check — never runs. Accepting
`{0, 18, 19, 20}` under `SSL_VERIFY_NONE` therefore accepts a **name-mismatched**
certificate: exactly the silent MITM hazard this document forbids.

`openssl s_client` is NOT a valid probe for this and initially suggested the
opposite (it reports 62 for the same cert) — because `s_client` installs its own
verify callback that returns 1, which is a different configuration from the one
the emitter uses.

**Corrected OpenSSL design — the doc's own stated fallback, now measured GOOD.**
Keep `SSL_VERIFY_PEER` and pass a *callback* instead of NULL, which clears only
the three trust-anchor errors and lets verification continue:

```c
if (preverify_ok) return 1;
int err = X509_STORE_CTX_get_error(ctx);
if (err == 18 || err == 19 || err == 20) { X509_STORE_CTX_set_error(ctx, 0); return 1; }
return 0;   /* everything else still aborts the handshake */
```

`/tmp/b477-cb.c`, same library, four servers:

| case | result |
| --- | --- |
| self-signed, name matches | rc=1, verify=0 — **ACCEPTED** |
| self-signed, name mismatch | rc=-1, verify=62 — **REJECTED** |
| cert `CN=wrong.example`, expect `localhost` | rc=-1, verify=62 — **REJECTED** |
| self-signed, **expired** (notAfter 2020) | rc=-1, verify=10 — **REJECTED** |

This is precisely the required semantics. It also has a property the
`SSL_VERIFY_NONE` design lacks: because the callback resets the error to
`X509_V_OK`, `SSL_get_verify_result` still returns **0** on the accept path, so
the existing `gen_openssl.rs:661` comparison needs **no change at all**. The only
edit is which value goes into `SSL_set_verify`'s third argument.

### 2. `DefaultValue::Fill` with a Boolean: `expr` must be `"false"`, not `"FALSE"`

The Fix Design's `expr: "FALSE"` **does not encode**. `default_argument_padding`
hands the `Fill` pair to `ir/lower.rs` (~:3690) which builds
`IrValue::Const { type_: Boolean, value: expr }`; that reaches
`abi::move_immediate(dst, "Boolean", value)` and finally
`src/arch/encode_operand.rs:42 immediate()`, whose Boolean vocabulary is exactly:

```rust
"true" => Ok(1), "false" => Ok(0), _ => value.parse::<u64>()
```

so `"FALSE"` is `invalid immediate 'FALSE'`. Lowercase is the canonical IR
spelling everywhere (`HirExpression::Boolean(value) => value.to_string()` at
`ir/lower.rs:3485`); `"TRUE"`/`"FALSE"` are only the *rendered display* forms
produced by `static_primitive_text` (`builder_value_semantics.rs:1090`,
`type_utils.rs:293`). **Resolved: `expr: "false"`.** No `Fill`-lowering change is
needed; Boolean was already supported, only the doc's spelling was wrong.

### 3. The bug-476 sequencing prerequisite is satisfied in substance

bug-476 is NOT on main (checked: `git log --oneline main | grep bug-476` finds
only this document's own commit). Its fix lives on the unlanded peer branch
`worktree-B-476` (`eb64ebfce`). This bug proceeds anyway, because the specific
hazard §Sequencing names does not exist:

- The hazard was `tls::connect(addressReturningCall(), ...)` selecting the
  host/port form and binding named arguments against the wrong layout.
- bug-476's own commit message records the measurement: "`tcp::connect`'s
  host/port-vs-`Address` shape is exercised but did NOT reproduce — a
  record-returning call is spilled to a temporary before the selector runs, so it
  already sees a `Local`". `tls::connect` reads the identical selector line
  (`builder_values.rs:2285`), so the same spill protects it.

So the `Address` form already selects correctly, and the new parameter binds
against the right layout in both. This bug touches none of the files bug-476
edits (`builder_values.rs`, `builder_value_semantics.rs`), so the two merge
cleanly in either order.

### 4. macOS: the verify block works — and macOS is *stricter* than the others

Prototyped in C against Network.framework before emitting a single instruction
(`/tmp/b477-nw.c`), because this is the backend the document calls the risk
concentration. The design that works, and which the emitter implements:

```c
sec_protocol_options_set_verify_block(o, ^(metadata, trust_ref, complete) {
    SecTrustRef trust = sec_trust_copy_ref(trust_ref);
    SecTrustSetPolicies(trust, SecPolicyCreateSSL(true, cfServerName));
    /* trust exactly what the peer offered as its own root, and nothing else */
    CFArrayRef chain = SecTrustCopyCertificateChain(trust);
    root = CFArrayGetValueAtIndex(chain, CFArrayGetCount(chain) - 1);
    SecTrustSetAnchorCertificates(trust, CFArrayCreate(NULL, &root, 1, ...));
    SecTrustSetAnchorCertificatesOnly(trust, true);
    complete(SecTrustEvaluateWithError(trust, NULL));
}, queue);
```

Note what this is *not*: it never calls `complete(true)` unconditionally, and it
never touches `sec_protocol_options_set_peer_authentication_required`. It
re-runs the **full** SSL policy — hostname, `notBefore`/`notAfter`, chain
signatures — with the peer's own root as the anchor. Measured:

| case | flag | result |
| --- | --- | --- |
| compliant self-signed, name matches | ON | **ACCEPTED** |
| compliant self-signed, name mismatch | ON | **REJECTED** — "certificate name does not match input" |
| self-signed, expired 2020 | ON | **REJECTED** — "certificate is expired" |
| compliant self-signed, name matches | OFF | **REJECTED** |

**Platform difference, and it is a real one.** macOS additionally enforces
Apple's TLS certificate *shape* policy, which OpenSSL does not: a server
certificate must carry an `extendedKeyUsage` of `serverAuth` and a validity
window no longer than ~398 days. A 10-year self-signed certificate is rejected
with "certificate is not standards compliant" **even with the flag set** — the
`SecTrustSetAnchorCertificates` exemption applies to keychain-installed roots,
not to a programmatic anchor, and there is no API to opt out.

This does not violate any requirement in this document: every property the flag
must preserve (name, dates, TLS 1.2 floor) holds, and macOS merely enforces
*more*. It fails closed. But it does have a consequence the Goal section did not
anticipate: **the shipped `examples/network-server/certs/cert.pem` is a 10-year
certificate, so the Failing Reproduction cannot succeed on macOS until that pair
is regenerated** within Apple's limits. That regeneration is in scope — it keeps
the certificate self-signed and untrusted, so it is not the forbidden "ship a
publicly-trusted certificate" shortcut — and it is why `certs/` gains a
regeneration script rather than only a longer-lived key.

## Failing Reproduction

The two examples added alongside this bug are the reproduction. Terminal 1:

```
mfb build examples/network-server
cd examples/network-server
./build/network-server.out --tls certs/cert.pem certs/key.pem --port 7413
```

Terminal 2:

```
mfb build examples/network-client
./examples/network-client/build/network-client.out --port 7413
```

- Observed (client, TLS attempt): `Failed <pid> TLS handshake, certificate validation, SNI validation, or protocol operation failed.`
- Observed (server): only `Listening tls 127.0.0.1:7413` — no `Connected` line; the server never sees a completed handshake.
- Expected, once fixed, with the new argument set: `Connected <pid>`, then `Data <pid> Hello <uuid>` and five `Data <pid> Update <uuid> NN` lines, then `Disconnected <pid>`; and `Connected <uuid>` / `Disconnect <uuid>` on the server.

The shipped `examples/network-server/certs/cert.pem` is `CN=localhost`, self-signed, `notAfter=Jul 23 18:41:05 2036 GMT`, with `subjectAltName = DNS:localhost, IP Address:127.0.0.1` (`openssl x509 -noout -subject -dates -ext subjectAltName`) — i.e. it is well-formed and name-correct, and fails *only* on chain trust.

Contrast cases that work correctly today and must keep working:

- **The server side is fine.** Driving the same server with a client that can be told to trust the cert succeeds end to end: `openssl s_client -connect 127.0.0.1:7413 -CAfile examples/network-server/certs/cert.pem` prints `Hello <uuid>` and the `Update` stream, and Python `ssl.create_default_context(cafile=…)` does the same. The gap is entirely on `tls::connect`.
- **A publicly-trusted peer works.** `./examples/network-client/build/network-client.out --host dns.google --port 443` prints `Connected <pid>` on the TLS attempt — `tls::connect`'s success path, `tls::poll`, `tls::write` and `tls::close` are all healthy.
- **The repo's own fixtures route around the gap.** `grep -rn "tls::connect" tests/rt-behavior/tls/*/src/main.mfb` — five of six connect to `8.8.8.8:443` with `serverName := "dns.google"`. The sixth, `tls-timeout-convention-rt`, points `tls::connect` at a *plain-TCP* listener; it is **currently dead** and exercises nothing at all (see Coordination — its golden pins a build failure). No fixture completes a loopback TLS handshake, because none can.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS 24.6.0 (Network.framework) | aarch64, release `mfb` | fails ✗ |
| Linux (OpenSSL 1.1.1/3.x) | untested — same descriptor, same missing argument | expected ✗ |
| Windows (Schannel) | untested — same descriptor, same missing argument | expected ✗ |

The failure is *not* platform-dependent: the argument does not exist in the
registry descriptor, so every target refuses identically. The **fix**, however,
is sharply platform-dependent — see Root Cause.

## Root Cause

There is no defect in any single line; the surface simply lacks the parameter,
and each backend hard-codes strict verification at a different seam.

**Descriptor.** `src/codegen/builtins/tls/func_connect.rs:register` declares two
`Implementation`s whose parameter lists end at `timeout_param()` and
`server_name_param(..)`. `expected_arguments` is
`"String, Integer, Integer, String or Address, Integer, String"`. There is no
third optional slot, so a caller has nothing to pass.

**Linux / OpenSSL** — `src/codegen/builtins/tls/gen_openssl.rs`:

- `:538-557` emits `SSL_set_verify(ssl, SSL_VERIFY_PEER, NULL)`. In client mode
  `SSL_VERIFY_PEER` **terminates the handshake** the moment chain validation
  fails, so control never reaches the post-handshake check.
- `:661-681` then re-checks `SSL_get_verify_result(ssl)` and branches to
  `tls_fail` on anything other than `X509_V_OK` (0).
- `:559-575` calls `SSL_set1_host(ssl, sniCstr)`, whose comment already records
  that this path "fails *closed* (over-strict), never open — there is no
  verification bypass (bug-177 C)".

This is the most tractable backend: `SSL_set1_host`'s name check is folded into
`X509_verify_cert`'s result, so relaxing *only* the trust-anchor errors while
keeping the name check is a matter of which `SSL_get_verify_result` codes the
`:661` comparison accepts.

**macOS / Network.framework** — `src/codegen/builtins/tls/gen_macos/client.rs:160-270`
builds a `sec_protocol_options` configure block whose invoke calls
`sec_protocol_options_set_tls_server_name` and nothing else; verification is
whatever Network.framework does by default, which is full system-trust
evaluation. Overriding it requires `sec_protocol_options_set_verify_block`,
which is **not** in the dlsym'd import set
(`src/codegen/builtins/tls/gen_macos/mod.rs:219-259` lists
`nw_tls_copy_sec_protocol_options`, `sec_protocol_options_set_tls_server_name`,
`sec_protocol_options_set_local_identity`, `nw_release`). A verify block is a
block that receives a *completion block* and must invoke it — strictly harder
than the existing capture-and-call configure block — and to preserve the name
check it must itself run `SecTrustEvaluateWithError` against a
`SecPolicyCreateSSL`-derived policy. **This is where the engineering risk
concentrates.**

**Windows / Schannel** — `src/codegen/builtins/tls/gen_schannel.rs:37-38` sets
`SCH_CRED_FLAGS = 4194336` = `SCH_CRED_AUTO_CRED_VALIDATION (0x20) |
SCH_USE_STRONG_CRYPTO (0x400000)`, so Schannel rejects an untrusted chain during
`InitializeSecurityContext`. Post-handshake,
`src/codegen/builtins/tls/gen_schannel_io.rs:emit_verify_hostname` already does
a *manual* `CertGetCertificateChain` +
`CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL)` and requires
`dwError == 0`, with `CERT_CHAIN_POLICY_PARA.dwFlags` (at `POLICYPARA + 4`) left zero by the
blanket zeroing loop at `:151-153`.
Because that manual path already exists, this backend is tractable too: switch
the credential to `SCH_CRED_MANUAL_CRED_VALIDATION (0x8)` and set
`CERT_CHAIN_POLICY_ALLOW_UNKNOWN_CA_FLAG (0x100)` in `dwFlags`.

**Argument plumbing.** A fifth parameter changes the ABI prologue.
`src/codegen/builtins/tls/gen_shared.rs:connect_arg_prologue` currently spills
`return_register(), c_arg(1), c_arg(2), c_arg(3)` (host form) and
`return_register(), c_arg(1), c_arg(2)` (Address form). The new flag lands at
`c_arg(4)` / `c_arg(3)`. `src/target/shared/abi.rs:c_arg` accepts 0..8, but see
Blast Radius for the Win64 hazard.

## Goal

- `tls::connect` accepts one new optional named `Boolean` argument, default
  `FALSE`, in **both** overloads (host/port and `net::Address`).
- With the argument `FALSE` or omitted, generated code is **byte-identical** to
  today on all five targets (the flag's only effect is a branch that is
  statically the strict path; if a constant-fold cannot make it byte-identical,
  the `.ncodesum` delta must be shown to be nothing but the new predicate).
- With the argument `TRUE`, `tls::connect` completes a handshake against
  `examples/network-server --tls certs/cert.pem certs/key.pem` on macOS, Linux
  and Windows, and the reproduction above prints the Expected output.
- With the argument `TRUE`, a certificate whose **name does not match** is still
  rejected with `ErrTlsFailed`, on all three backends. Same for an **expired**
  certificate and for a peer that will not negotiate TLS ≥ 1.2.
- `mfb man tls connect`, `mfb spec stdlib transports` → "TLS specifics", and
  `.ai/net-tls.md` all state the new semantics and the risk.
- `mfb audit` reports use of the argument as a distinct finding, so a reviewer
  sees a relaxed-trust connection without reading the source.

### Non-goals (must NOT change)

- **No behavior change for any existing call site.** Omitting the argument must
  be exactly today's handshake. In particular
  `src/codegen/builtins/http/helper_start_exchange.rs:20` — the HTTPS path of
  the whole `http::` package — must keep full verification. `ir/lower.rs:3545`
  pads omitted trailing optionals, so this is a *padding-value* correctness
  requirement, not a documentation one: the pad must be `FALSE`.
- **Not a blanket "insecure" / "skip verification" switch.** The name check,
  the validity dates and the TLS ≥ 1.2 floor stay enforced when the flag is set.
  A backend that can only implement all-or-nothing bypass must be reported as a
  blocker, not shipped as "close enough" — that would be the
  compile-everywhere-fail-differently trap the spec cites as the reason
  `tls::wrap` exists nowhere (`src/docs/spec/stdlib/17_transports.md:140-143`).
- **`tls::listen` / `tls::accept` are untouched.** No client-certificate
  request, no mutual TLS.
- **The repository client is untouched.** Package fetch/verify transport
  security must not gain a relaxation path, by argument or by default.
- **Tempting wrong fixes, forbidden explicitly:**
  - Making the default `TRUE`, or making it configurable by environment
    variable / manifest key — the flag must be visible at the call site.
  - "Fixing" the reproduction by shipping a publicly-trusted certificate with
    `examples/network-server`, or by rewriting `examples/network-client` to stop
    attempting TLS. That masks the bug.
  - Relaxing `tests/rt-behavior/tls/*` to hide the gap.
  - Implementing macOS as `complete(true)` unconditionally inside the verify
    block. That silently drops the name check on one platform only, and is the
    single most likely way this bug ships broken.

## Blast Radius

Every consumer of `tls.connect`, found by
`grep -rn "tls\.connect\|tls::connect" src/ --include="*.rs" --include="*.mfb"`:

- `src/codegen/builtins/tls/func_connect.rs:register` — **fixed by this bug** (both `Implementation`s).
- `src/codegen/builtins/tls/gen_shared.rs:connect_arg_prologue` — **fixed by this bug**; both shapes gain a spill slot.
- `src/codegen/builtins/tls/gen_openssl.rs:538,661` — **fixed by this bug**.
- `src/codegen/builtins/tls/gen_macos/client.rs:160-270` + `gen_macos/mod.rs:219-259` (import set) — **fixed by this bug**; the hard one.
- `src/codegen/builtins/tls/gen_schannel.rs:37` + `gen_schannel_io.rs:emit_verify_hostname` — **fixed by this bug**.
- `src/codegen/builtins/http/helper_start_exchange.rs:20` — `tls::connect(url.host, url.port, __HTTP_CONNECT_TIMEOUT_MS, url.host)`. **In scope as a guard, not as a feature.** It must keep verifying; add a test that proves the padded flag is `FALSE`. Whether `http::` should ever expose the option is deferred (see Open Decisions).
- `src/ir/lower.rs:3545` ("Pad optional trailing arguments (`tls.connect` defaults)") — **fixed by this bug**: the pad table gains the `FALSE` entry. This is the seam where a mistake becomes a silent security regression rather than a compile error.
- `src/codegen/engine/value/builder_values.rs:2285` (`"tls.connect"` → `tls.connectAddr` selection) — **unaffected**: the overload is chosen by the *first* argument's static type, not by arity, and its comment already records that "every call reaches here already padded".
- `src/target/linux_common/mod.rs:210`, `src/target/win_x86_64/mod.rs:240-241`, `src/target/macos_aarch64/mod.rs:207-208` (`SUPPORTED_RUNTIME_CALLS`), `src/target.rs:583`, `src/codegen/memory/data/data_objects.rs:454` — **audit required, likely unaffected**: these gate the call *name*, which does not change. Confirm no arity is encoded.
- `src/target/linux_common/plan.rs:590-622` — special-cases `tls.connect` (it opens its own TCP socket). **Audit required**: confirm the extra argument does not shift anything it reads.
- `tests/byte-identity/tls/` (5 `.ncodesum` goldens: linux-aarch64/riscv64/x86_64, macos-aarch64, windows-x86_64) and `tests/byte-identity/http/` — **regeneration expected**, extent to be proven in Phase 3.
- `tests/rt-behavior/tls/tls-timeout-convention-rt/golden/{build.log,*.ast,*.ir}` — **in scope, but only once bug-466 lands.** It is a live `tls::connect(host, port, timeoutMs)` call site whose `.ir`/`.ast` shift when a fifth optional parameter is padded in. It cannot fail today because it does not build (see Coordination); after bug-466 it will genuinely assert the timeout convention against the new arity.
- `repository/` (the Rust package-repository client) — **unaffected**: `grep -rn "tls" repository/src/*.rs` finds no use of the MFBASIC `tls` builtin; it does not route through this member.
- `tls::listen` / `tls::accept` / `tls::poll` / `tls::read` / `tls::write` / `tls::close` — **unaffected**: none participates in peer verification.

**x86-64 argument-bank hazard (measured, not assumed).** The host-form flag
lands at `c_arg(4)`, the fifth slot. MFB's internal bank is 8 wide on every
target, so the slot does exist — but the two x86 ABIs realize index 4 to
*different* registers, and neither is a C argument register on its own ABI
(`src/arch/x86_64/select.rs:65,92`):

```
CALL_ARGS        = ["rdi", "rsi", "rdx", "rcx", "r8",  "r9",  "rax", "rbp"]   // SysV
CALL_ARGS_WIN64  = ["rcx", "rdx", "r8",  "r9",  "rdi", "rsi", "rax", "rbp"]   // Win64
```

So index 4 is `r8` on SysV and **`rdi` on Win64** — where `rdi` is *non-volatile*
in the Win64 C ABI and is simultaneously `c_arg(0)` on SysV. Any shared emitter
that stages `c_arg(0)` for a C call, or that treats `rdi` as scratch, collides
with the incoming flag on exactly one of the two ABIs. Index 6 is `rax` — the C
return register — on both, so the bank must not grow past 6 without a plan.
On AArch64/RISC-V index 4 is plainly `x4`/`a4`
(`src/target/shared/abi.rs:realize_abi_positional`) and no hazard exists, which
is precisely why a Mac-only test run cannot see this class. A mistake here is a
*wrong value*, which byte-identity cannot catch. Confirm the spill/reload of
`c_arg(4)` on both x86 targets in Phase 1, per
`.ai/arch-abi.md` ("Three x86-64 foreign-call traps").

## Fix Design

Add to both `Implementation`s:

```rust
Parameter {
    name: "allowSelfSigned",
    desc: "Optional. When TRUE, accept a certificate chain that fails validation \
           only because its root is not in the host trust store — a self-signed \
           certificate, or one issued by a private CA. The server name, the \
           validity dates and the TLS 1.2 floor are still enforced. Defaults to \
           FALSE, which requires a chain the host already trusts.",
    aliases: &[],
    ty: ParameterType::Boolean,
    default: DefaultValue::Fill {
        type_name: ParameterType::Boolean,
        expr: "FALSE",
    },
}
```

`DefaultValue::Fill` has **no Boolean precedent anywhere in the registry** —
`grep -rn "ty: ParameterType::Boolean" src/codegen/builtins/*/func_*.rs` finds
only return types and predicate function types, never an optional Boolean
parameter. `expr` is a `&'static str` parsed at
`src/codegen/registry/mod.rs:2708`; whether `"FALSE"` lowers to a Boolean
constant is unverified and is the first thing Phase 1 must establish. If it does
not, the fallback is an Integer flag with a Boolean surface, or extending the
`Fill` lowering — decide before writing backend code.

Per-backend, all gated on the flag:

| Backend | At `FALSE` (today) | At `TRUE` |
| --- | --- | --- |
| OpenSSL | `SSL_set_verify(SSL_VERIFY_PEER)`; require `SSL_get_verify_result == X509_V_OK` | `SSL_set_verify(SSL_VERIFY_NONE)` so the handshake completes, then accept `X509_V_OK (0)`, `DEPTH_ZERO_SELF_SIGNED_CERT (18)`, `SELF_SIGNED_CERT_IN_CHAIN (19)`, `UNABLE_TO_GET_ISSUER_CERT_LOCALLY (20)` — and **nothing else**, so `HOSTNAME_MISMATCH (62)` and the expiry codes still fail |
| Schannel | `SCH_CRED_AUTO_CRED_VALIDATION \| SCH_USE_STRONG_CRYPTO`; `dwFlags = 0` | `SCH_CRED_MANUAL_CRED_VALIDATION (0x8) \| SCH_USE_STRONG_CRYPTO`; `dwFlags = CERT_CHAIN_POLICY_ALLOW_UNKNOWN_CA_FLAG (0x100)` — the existing `emit_verify_hostname` keeps checking the name |
| Network.framework | default system trust evaluation | dlsym `sec_protocol_options_set_verify_block`; the block runs `SecTrustEvaluateWithError` under a `SecPolicyCreateSSL(true, serverName)` policy and completes `true` only for a trust result that is otherwise clean |

The OpenSSL move from `SSL_VERIFY_PEER` to `SSL_VERIFY_NONE` rests on the claim
that `X509_verify_cert` still runs and still records the `SSL_set1_host` name
result under `SSL_VERIFY_NONE` (which only suppresses the *abort*, not the
evaluation). **That is a claim, not a measurement** — Phase 1 must prove it with
a name-mismatched self-signed cert, or the design falls back to an
`SSL_CTX_set_cert_verify_callback` / `SSL_set_verify` callback that inspects
`X509_STORE_CTX_get_error` directly.

Rejected alternatives:

- **A `caFile` / `caPath` argument instead of a Boolean.** Strictly better
  security (pin the private CA rather than trust anything), and it is the design
  a production user wants. Rejected *for this bug* because it is a larger
  surface on all three backends (`SSL_CTX_load_verify_locations`,
  `SecTrustSetAnchorCertificates`, a `HCERTSTORE` on Windows) and does not
  satisfy the stated request. Recorded as follow-up work, not as a reason to
  delay this.
- **A separate member (`tls::connectInsecure`).** Doubles the member count, the
  goldens and the man surface for one bit, and the `net::Address` overload would
  double again to four implementations.
- **Reusing `serverName == "*"` or another in-band sentinel.** Overloads a
  string with a security meaning; invisible to `mfb audit`.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt-behavior/tls/tls-connect-self-signed-rt/` : a fixture that
      `tls::listen`s on a loopback port with a self-signed cert, `tls::connect`s
      to it with the new argument, and asserts the round trip. Confirm it fails
      today at the argument (`TYPE_CALL_ARGUMENT_MISMATCH` / unknown named
      argument), and will fail at the handshake once the argument parses.
      Per `.ai/resources-packages.md`, a collection/argument type error surfaces
      only in a **full** `mfb build`, not `-ast -ir`, so this must be
      `rt-behavior`, not `tests/syntax`.
- [ ] Add the negative fixtures alongside it: name-mismatched self-signed cert,
      and expired self-signed cert, both with the flag `TRUE`, both expected to
      raise `ErrTlsFailed`. These are the tests that stop the bug shipping as a
      blanket bypass.
- [ ] Add a guard test that `http::` HTTPS still verifies — the padded flag is
      `FALSE` at `helper_start_exchange.rs:20`.
- [ ] **Resolve `DefaultValue::Fill` with a Boolean.** Add the parameter with no
      backend change and confirm an omitting call still compiles and still
      produces the strict path. If `expr: "FALSE"` does not lower, record the
      chosen fallback here before proceeding.
- [ ] **Confirm `c_arg(4)` end to end on both x86 ABIs** for the host-form
      overload — it realizes to `r8` on SysV and `rdi` on Win64
      (`src/arch/x86_64/select.rs:65,92`), and `rdi` is non-volatile on Win64
      while being `c_arg(0)` on SysV. Prove the flag survives the argument
      prologue's spill on `linux-x86_64` and `windows-x86_64`, not just on
      aarch64. Record the answer here.
- [ ] **Confirm the OpenSSL claim**: under `SSL_VERIFY_NONE` with
      `SSL_set1_host` set, does `SSL_get_verify_result` still report
      `X509_V_ERR_HOSTNAME_MISMATCH` for a name-mismatched cert? Prove it with a
      C or `openssl` reproduction before writing the emitter.
- [ ] Complete the blast-radius audit above: give a verdict for each
      `SUPPORTED_RUNTIME_CALLS` site and for `linux_common/plan.rs:590-622`.

Acceptance: the new fixtures fail for the documented reason; every unknown in
this phase has a recorded answer in this file.
Commit: —

### Phase 2 — the fix

- [ ] Add the parameter to both `Implementation`s and update
      `expected_arguments` (`src/codegen/builtins/tls/func_connect.rs`).
- [ ] Extend `connect_arg_prologue` for both shapes
      (`src/codegen/builtins/tls/gen_shared.rs`).
- [ ] Add the `FALSE` pad entry at `src/ir/lower.rs:3545`.
- [ ] OpenSSL: gate `SSL_set_verify` and the `SSL_get_verify_result` comparison
      (`gen_openssl.rs:538,661`).
- [ ] Schannel: gate `SCH_CRED_FLAGS` (`gen_schannel.rs:37-38`) and
      `CERT_CHAIN_POLICY_PARA.dwFlags` (`gen_schannel_io.rs:151-165`).
- [ ] Network.framework: dlsym `sec_protocol_options_set_verify_block`, add it
      to the import set (`gen_macos/mod.rs`), and emit the verify block
      (`gen_macos/client.rs`).
- [ ] `mfb audit`: add `AUDIT-TLS-RELAXED-TRUST` beside the existing codes in
      `src/audit/collect/findings.rs`, reported wherever the argument is passed
      `TRUE`.

Acceptance: Phase 1 fixtures pass; the negative fixtures still raise; the
`http::` guard still verifies; nothing in Non-goals changed.
Commit: —

### Phase 3 — docs, goldens, full validation

- [ ] Update `func_connect.rs` `DESC` (the "always verified" paragraph) and add
      an example using the argument; re-render `mfb man tls connect`.
- [ ] Update `src/docs/spec/stdlib/17_transports.md` "TLS specifics" — the
      "**The client verifies.**" bullet — and `.ai/net-tls.md`.
- [ ] Add the `AUDIT-TLS-RELAXED-TRUST` row to the enumerated finding-code table
      at `src/docs/spec/tooling/04_audit-format.md:207-209`. The code is only
      half-added if it exists in `findings.rs` but not in that table.
- [ ] Update `examples/network-client/src/main.mfb`: its header comment
      currently documents the limitation and points at `--server-name`. Replace
      that with the new argument, behind an explicit opt-in flag
      (`--allow-self-signed`), so the example still defaults to verifying.
- [ ] Extend `tests/byte-identity/tls/src/main.mfb` to cover the new argument in
      both overloads (per `.ai/testing-gates.md`, a `codegen_cover` fixture that
      does not mention a member never hashes it).
- [ ] Regenerate the 5 `tls` `.ncodesum` goldens + any `http` drift via
      `scripts/regen-ncodesum.sh`; **diff and prove** the delta is only the new
      predicate and the argument spill.
- [ ] `artifact-gate.sh all` to 0 diffs; full `cargo test --no-fail-fast`;
      `cargo check --all-targets`; `test-accept.sh`.
- [ ] Re-run the reproduction on macOS, Linux and Windows.

Acceptance: full suite green; golden delta is exactly the intended change; the
reproduction passes on every row of the matrix.
Commit: —

## Validation Plan

- Regression tests: `tests/rt-behavior/tls/tls-connect-self-signed-rt/` (positive), plus the name-mismatch and expiry negatives and the `http::` guard.
- Runtime proof: the `examples/network-server --tls` ↔ `examples/network-client` round trip from Failing Reproduction, on all three platforms — the end-to-end behavior no unit test can show.
- Doc sync: `func_connect.rs:DESC`, `src/docs/spec/stdlib/17_transports.md`, `src/docs/spec/tooling/04_audit-format.md` (finding-code table), `.ai/net-tls.md`, `examples/network-client/src/main.mfb` header.
- Full suite: `cargo test --no-fail-fast`, `cargo check --all-targets`, `scripts/artifact-gate.sh all`, `scripts/test-accept.sh`, and the 5-target `scripts/build-examples.sh`.

## Coordination (in-flight work that moves these files)

Recorded 2026-08-31 from peer sessions; main was at `ab66ed781` (bug-463) when
this was written, and **none of the branches below was merged yet**. Verify
before relying on any of it.

**The rebase prerequisite is satisfied by presence on main, not by a branch
being green.** A branch can pass the full suite and still be hours from
landing, and a `.ncodesum` baseline taken against an unmerged branch is worth
nothing. Check with `git log --oneline main | grep -i 'bug-46[46]'` before
Phase 2 or Phase 3, not with a peer's test result.

**bug-464 (session `mfb-8f`, branch `worktree-B-464`)** makes `tls::Socket`,
`tls::Listener` and `tcp::Listener` thread-sendable. Reported effects on this
bug's blast radius:

- Four of Phase 2's files gain content — `gen_shared.rs`, `gen_schannel.rs`,
  `gen_schannel_server.rs`, `gen_macos/mod.rs` (two exported block-size
  constants and three `pub(crate)` record offsets), plus a `live_slots`
  descriptor on both `RegistryResource` rows in `tls/mod.rs`. No behavioural
  change to `connect`/`listen`/`accept`. **Rebase onto main before starting
  Phase 2**, or the emitter edits land on stale line numbers.
- It regenerates the five `tests/byte-identity/tls/*.ncodesum` goldens.
  **Phase 3 must rebase before baselining anything**, or the "prove the delta is
  only the new predicate" step diffs against stale sums and reads as a much
  larger change than it is.
- Its `MODULE_DESC` prose edit (`tls/mod.rs:112` today reads "Neither handle
  type is thread-sendable") does **not** conflict with this bug — verified: this
  bug edits only `func_connect.rs`'s `DESC`, a different constant in a different
  file, and never touches `MODULE_DESC`.
- It also fixes three pre-existing `binary_repr` defects that made most built-in
  resources unusable in a package's public API on clean main
  (`EXPORT FUNC f(RES s AS udp::Socket)` → `error: truncated binary
  representation`). Not a prerequisite here — every fixture this bug proposes is
  a single-project `rt-behavior` case with no package boundary — but it is the
  reason not to *add* one.

**bug-466 (session `mfb-e4`, branch `worktree-B-466`)** revives
`tests/rt-behavior/tls/tls-timeout-convention-rt`, dead since `9b62dcf23`
(plan-110-E Phase 2) — that commit migrated it from `net::` to `tcp::` and
dropped `IMPORT net` while the body still reads `bound.port` off the
`net::Address` that `tcp::localAddress` returns. Verified here: its
`golden/build.log` pins

```
$ mfb build tests/rt-behavior/tls/tls-timeout-convention-rt
error: native plan has no storage class for type 'Unknown'
[exit 1]
```

so the build failure *is* the accepted baseline and its plan-73-D timeout
assertions have not run since. Consequence for this bug: that fixture is a live
`tls::connect(host, port, timeoutMs)` call site which starts executing again the
moment bug-466 lands, so its `.ast`/`.ir` goldens join the blast radius — and,
usefully, it will then actually catch a fifth-parameter change that breaks the
timeout convention instead of staying silently broken.

**Swept for the same class** (a golden baselined onto a build failure, which is
what hid this for months), so a future phase need not redo it:
`grep -rlE '^error:' --include=build.log tests/**/golden` returns **7** hits —
**6 in `tests/syntax/`**, all intentional (those fixtures exist to assert a
diagnostic: `app_import_requires_app_mode_invalid` and five
`security/pkg-0*` package-decode refusals), and **exactly 1 in
`tests/rt-behavior/`**, the fixture above. Non-`rt-error` fixtures ending in
`[exit 255]` were checked separately and are genuine *runtime* error assertions
(`Error: 7-703-0001` and friends), not build failures. So the anomaly is a
singleton, not a pattern — but note the harness raised no alarm when an
`rt-behavior` fixture (which is meant to build *and* run) was baselined onto a
compile error. That gap is unowned.

**bug-478 (session `mfb-18`, branch `worktree-P-98`)** was briefly filed as
bug-477 and has been renumbered; **the next free bug number is 479**, and note
that a number is only safe once it is on main. It adds an `errorCode` constant
(`ErrBadFontFile`) and bumps the `ADDED_SINCE_MIGRATION` guard in
`src/codegen/builtins/errorcode/mod.rs`. **No conflict with this bug** —
verified: this bug adds no `errorCode` constant (it reuses the existing
`ErrTlsFailed`) and its only new code is an audit finding, which lives in
`src/audit/collect/findings.rs`, a different table in a different file.

## Open Decisions

- **Parameter name.** `allowSelfSigned` (recommended — says what it permits, and its scope is narrower than "insecure") vs. `insecureSkipVerify` (Go's name; accurate about the risk but overstates what this does, since the name check survives) vs. `trustAnyRoot`.
- **Should `http::` expose it too?** Recommended **no** for this bug: `http::get("https://…")` gaining a trust-relaxation argument is a much larger surface, and the guard test above pins it closed. Revisit as follow-up. (§Blast Radius)
- **macOS parity floor.** If `sec_protocol_options_set_verify_block` cannot preserve the name check without a disproportionate amount of block plumbing, is a macOS-only all-or-nothing bypass acceptable? Recommended **no** — that is precisely the compile-everywhere-behave-differently trap `tls::wrap` was refused over. Escalate rather than ship the asymmetry. (§Non-goals)
- **Follow-up: `caFile`.** Pinning a private CA is the better long-term answer and would make the loopback example verify *properly* rather than leniently. Recommended as a separate bug once this lands.

## Summary

The engineering risk is not the descriptor change — that is mechanical — but the
**macOS verify block**, which is the only backend with no existing manual
verification seam to gate, and the **`ir/lower.rs` pad value**, which is the one
place where a mistake turns a currently-safe HTTPS client into a silent MITM
target with no compile error and no golden diff to catch it. Linux and Windows
are comparatively cheap: both already re-check the result after the handshake,
so the change is which error codes are accepted, not new API surface.

Untouched: the whole server side (`listen`/`accept`), every other `tls` member,
the repository client, and — by explicit test — `http::`'s HTTPS verification.
