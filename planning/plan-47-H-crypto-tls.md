# plan-47-H: crypto over CNG and TLS over Schannel

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-47-G2 (sockets — TLS is a transport over them), plan-47-C.
Feature-wide precondition: master §Prerequisites.
Produces: the Windows `crypto::*` implementation and a third `tls::` backend;
`crypto.*`/`tls.*` in `runtime_calls`. Nothing consumes it — this is the last surface.

Implements `crypto::*` over CNG/BCrypt and `tls::*` over Schannel.

The single behavioral outcome: a program that hashes, signs and verifies produces
byte-identical output on Windows and linux-x86_64; and a program that makes an HTTPS
request to a real host succeeds on Windows with the same response bytes.

**The master calls this "the third sibling to `code/tls/{openssl,macos}.rs`". That is
wrong in a way that matters:** `tls/openssl.rs` is not the *Linux* backend, it is the
**default** backend, reached by an `else` — so Windows currently falls into OpenSSL, and
`crypto_ec.rs:113` sends Windows to the OpenSSL EC path too. Adding Schannel is not
adding a file; it is editing a dispatch that today has no third arm.

References (read first):

- `src/target/shared/code/crypto_ec.rs:113` — `if platform.target().contains("macos") {
  macos::lower(…) } else { openssl::lower(…) }`. **Windows takes the `else`.**
- `src/target/shared/code/mod.rs:680`, `:688`, `:703` — data-object emission for TLS
  C-strings, ALSA sonames and EC dlsym names, all `contains("macos")` binary tests.
  `:688` is **negated** (`!contains("macos")`), so Windows gets the ALSA arm.
- `src/target/shared/code/tls/openssl.rs` — 7 internal `contains("macos")` branches
  (`rg -c 'contains("macos")'` → 7). This is the default backend, not the Linux one.
- `src/target/shared/code/tls/{mod,macos}.rs` — the dispatch and the one genuinely
  OS-specific sibling.
- `planning/plan-47-P-platform-family-match.md` — converts all of the above to exhaustive
  matches; H fills the Windows arms.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-47-P has landed (dispatches are exhaustive) | `rg -n 'enum PlatformFamily' src/` | **NOT MET** |
| plan-47-G2 has landed (sockets work) | `rg -n 'closesocket' src/target/win_x86_64/` | **NOT MET** |
| The Win11 box answers, with outbound HTTPS | `ssh -p 2230 test@127.0.0.1 true` | **UNVERIFIED — run it** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before continuing and again before deciding to stop. If you stop, report all
> three statuses.

## 1. Goal

- `crypto::*` over CNG/BCrypt: hashing, HMAC, random, and the EC sign/verify surface.
- `tls::*` over Schannel: client connect with certificate validation, read/write,
  shutdown; and the server/accept path if `tls::listen` is advertised.
- The three dispatch sites (`crypto_ec.rs:113`, `mod.rs:680`/`:703`) gain real Windows
  arms; `mod.rs:688`'s **negated** ALSA test stops giving Windows ALSA sonames.
- `crypto.*`/`tls.*` advertised only after this lands.

### Non-goals (explicit constraints)

- **No new crypto primitives.** Whatever `crypto::` exposes today, implemented on CNG.
  Nothing new, nothing removed.
- **No bundled OpenSSL for Windows.** Schannel is the platform TLS stack and needs no
  vendored library — the same reasoning that makes macOS use Network.framework.
- **No certificate-store policy invention.** Schannel uses the Windows certificate store;
  do not add a custom trust root mechanism.
- **Do not "fix" `tls/openssl.rs`'s 7 macOS branches.** They are that backend's business;
  H adds a sibling, it does not refactor the default one.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Backend dispatch sites Windows currently falls through | **6** | `rg -n 'contains("macos")' src/target/shared/code/crypto_ec.rs src/target/shared/code/mod.rs` |
| — of those, **negated** (the dangerous one) | 1 | `mod.rs:688`, `!contains("macos")` → Windows gets ALSA |
| `contains("macos")` branches inside `tls/openssl.rs` | **7** | `rg -c 'contains("macos")' src/target/shared/code/tls/openssl.rs` |
| Existing TLS backends | 2 (`openssl.rs` = default, `macos.rs` = specific) | `ls src/target/shared/code/tls/` |
| Existing `CodegenPlatform` methods for crypto/TLS | 2 (`emit_random_bytes` — done in 47-C — and `emit_tls_block_trampolines`, defaulted) | master §2.1 |

### 2.2 What Windows silently gets today

Every one of the 6 dispatch sites is binary. Registering `windows-x86_64` without this
sub-plan means:

| Site | Windows silently gets |
|---|---|
| `crypto_ec.rs:113` | the **OpenSSL** EC backend |
| `mod.rs:680` | OpenSSL TLS C-strings in `.rdata` |
| `mod.rs:688` (negated) | **ALSA sonames in `.rdata`** — a Linux audio library, on Windows |
| `mod.rs:703` | OpenSSL EC dlsym names |
| `mod.rs:1036`, `:1052` | no audio callback (correct by accident) |

47-P converts these to exhaustive matches so they become compile errors; H supplies the
answers.

### 2.3 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `tls/openssl.rs` is the **default** backend, not the Linux one | **CONFIRMED** | `crypto_ec.rs:113` and `tls/mod.rs` dispatch is `if macos { macos } else { openssl }` |
| Windows therefore falls into OpenSSL today | **CONFIRMED** | it is the `else` arm |
| `mod.rs:688` is negated and gives Windows the ALSA arm | **CONFIRMED** | `!platform.target().contains("macos")` |
| Adding a backend requires editing dispatch, not just adding a file | **CONFIRMED** | there is no registry — the dispatch is an `if/else` |
| Schannel needs no vendored library | **CONFIRMED** | it is a system component (`secur32.dll`/`schannel.dll`) |
| CNG covers the `crypto::` surface | **UNVERIFIED — Phase 1 establishes it** | BCrypt covers hashing/HMAC/random; the **EC sign/verify** surface must be checked primitive by primitive against what `crypto::` exposes |

That last row is this sub-plan's real unknown and the reason Phase 1 is an inventory, not
an implementation.

## 3. Design Overview

Four pieces:

1. **A third dispatch arm.** `crypto_ec.rs:113` and `tls/mod.rs` gain a `Windows` arm
   (47-P made them exhaustive). This is the structural change the master missed.
2. **`crypto::` over CNG/BCrypt** — `BCryptOpenAlgorithmProvider` /
   `BCryptCreateHash` / `BCryptHashData` / `BCryptFinishHash` for digests;
   `BCryptGenerateKeyPair` / `BCryptSignHash` / `BCryptVerifySignature` for EC.
3. **`tls::` over Schannel** — `AcquireCredentialsHandle` /
   `InitializeSecurityContext` (loop until complete) / `EncryptMessage` /
   `DecryptMessage` / `DeleteSecurityContext`, over the 47-G socket.
4. **Data objects** — `mod.rs:680`/`:688`/`:703` emit Windows-appropriate strings, or
   none. Windows needs **no** dlsym names: CNG and Schannel are linked through the IAT
   like every other Win32 call, not `dlopen`ed.

**Where design uncertainty concentrates: the Schannel handshake loop.** OpenSSL and
Network.framework both expose "do the handshake" as roughly one call.
`InitializeSecurityContext` is a **state machine the caller drives** — it returns
`SEC_I_CONTINUE_NEEDED` and hands back a token to write to the socket, repeatedly, and
the caller must manage a receive buffer that may hold a partial record. That is a
genuinely different shape from both existing backends, and it is emitted as machine code,
not written in Rust.

**Phase 2 is a spike on exactly the handshake**, against a real host, before read/write
is attempted. If driving that loop from generated code proves impractical, the shape of
this sub-plan changes and it is better to know at Phase 2 than Phase 4.

**Where correctness risk concentrates:** certificate validation. The failure mode is
silent and severe — a handshake that *succeeds* while validating nothing looks identical
to a correct one from the program's perspective. Schannel validates via
`AcquireCredentialsHandle` flags plus an explicit `CertGetCertificateChain` /
`CertVerifyCertificateChainPolicy` step that is **easy to omit and never noticed**.
Phase 3's acceptance therefore includes connecting to a host with a *bad* certificate and
requiring failure — a negative test is the only proof that validation is on.

**Rejected alternative:** *bundle OpenSSL for Windows.* Rejected: it contradicts the
no-vendored-library posture, adds a supply-chain surface, and duplicates a platform stack
that already exists. macOS made the same call.

**Rejected alternative:** *implement `tls::` on Windows via the OpenSSL backend and
ship OpenSSL DLLs.* Same rejection, plus it would make Windows the only target needing a
runtime redistributable.

**Rejected alternative:** *refactor `tls/openssl.rs`'s 7 macOS branches into the
dispatch while here.* Tempting cleanup, rejected as scope: it would put a refactor of the
default backend inside the diff that adds a third one, making any TLS regression
unattributable.

## 4. Detailed Design

| Surface | Windows |
|---|---|
| digest / HMAC | `BCryptOpenAlgorithmProvider`, `BCryptCreateHash`, `BCryptHashData`, `BCryptFinishHash` |
| random | `BCryptGenRandom` (already in 47-C's floor) |
| EC sign / verify | `BCryptGenerateKeyPair`, `BCryptSignHash`, `BCryptVerifySignature` |
| TLS handshake | `AcquireCredentialsHandle` + `InitializeSecurityContext` loop |
| TLS read / write | `DecryptMessage` / `EncryptMessage` |
| TLS shutdown | `ApplyControlToken(SCHANNEL_SHUTDOWN)` + a final `InitializeSecurityContext` |
| cert validation | `CertGetCertificateChain` + `CertVerifyCertificateChainPolicy` |

Imports: `bcrypt.dll`, `secur32.dll`, `crypt32.dll`.

## Compatibility / Format Impact

- **New:** `crypto.*`/`tls.*` in the Windows `runtime_calls`; bcrypt/secur32/crypt32
  imports; a `tls/schannel.rs`.
- **Changed (shared):** `crypto_ec.rs:113` and `tls/mod.rs` dispatch gain a third arm;
  `mod.rs:680`/`:688`/`:703` gain Windows data-object answers. All four existing targets
  byte-identical.
- **Unchanged:** the `crypto::`/`tls::` language surface; `tls/openssl.rs`'s and
  `tls/macos.rs`'s behavior; every other backend.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — inventory: does CNG cover the surface? (settles the unknown)

No implementation. This phase exists because §2.3's last row is unverified, and
discovering a gap at Phase 4 would be expensive.

- [ ] Enumerate every primitive `crypto::` exposes today
      (`rg -n 'crypto\.' src/target/shared/runtime/`).
- [ ] Map each to its CNG/BCrypt equivalent, or record it as a gap.
- [ ] For each gap, decide: emulate, or reduce the Windows `runtime_calls` set and reject
      that call at compile time.

Acceptance: a written primitive-by-primitive mapping with every gap named and decided.
If a gap has no acceptable answer, stop and report — that is a scope question, not an
implementation detail.
Commit: —

### Phase 2 — spike: the Schannel handshake (falsifies the design premise)

- [ ] Third dispatch arm in `crypto_ec.rs:113` and `tls/mod.rs`; `tls/schannel.rs`
      skeleton.
- [ ] Drive `AcquireCredentialsHandle` + the `InitializeSecurityContext` loop to a
      completed handshake against a real HTTPS host, over a 47-G socket. Handshake only —
      no application data.
- [ ] Handle the partial-record case explicitly (`SEC_E_INCOMPLETE_MESSAGE`).

Acceptance: a completed handshake against a real host, from generated code. If driving
the state machine from emitted code is impractical, stop and redesign §3 — that is what
this phase is for.
Commit: —

### Phase 3 — certificate validation (highest-consequence, and silent if wrong)

- [ ] `CertGetCertificateChain` + `CertVerifyCertificateChainPolicy` on the handshake
      result.
- [ ] **Negative test:** connect to a host with an expired/self-signed/wrong-name
      certificate and require the connection to **fail**.

Acceptance: the good host succeeds **and** all three bad-certificate cases fail. A
passing positive test alone proves nothing here — a handshake that validates nothing also
passes it.
Commit: —

### Phase 4 — crypto primitives and data objects

- [ ] The Phase 1 mapping, implemented over BCrypt.
- [ ] `mod.rs:680`/`:688`/`:703` Windows arms — **no dlsym names** (CNG/Schannel are IAT
      imports), and **no ALSA sonames** (`:688` is negated; this is the site that would
      otherwise put a Linux audio library string in a Windows binary).
- [ ] Tests: hash/HMAC/sign/verify outputs byte-identical to linux-x86_64.

Acceptance: every crypto primitive produces byte-identical output across the two targets;
`strings` on a Windows binary shows no OpenSSL or ALSA artifacts.
Commit: —

### Phase 5 — read/write and advertise (largest blast radius last)

- [ ] `EncryptMessage`/`DecryptMessage`, shutdown, and the `tls::listen`/accept path if
      advertised.
- [ ] Advertise `crypto.*`/`tls.*` in `runtime_calls`.
- [ ] Runtime: an HTTPS GET returning byte-identical response bytes to linux-x86_64.

Acceptance: the HTTPS round-trip matches byte-for-byte, including a response large enough
to span multiple TLS records.
Commit: —

## Validation Plan

- Tests: crypto primitives are byte-comparable and belong in the acceptance suite. TLS is
  proven by runtime round-trips.
- Coverage check: the dispatch and data-object edits are shared-code changes gated by
  `scripts/artifact-gate.sh`; `linux-riscv64` has zero goldens (master §Prerequisites
  row 3). Seed them first.
- Runtime proof: the Win11 box with outbound HTTPS. **The negative certificate tests are
  the load-bearing ones** — they are the only evidence validation is enabled.
- Doc sync: if the Phase 1 inventory reduces the Windows crypto surface, that is a
  documented per-target capability difference and belongs in the spec.
- Acceptance: full suite plus `scripts/artifact-gate.sh` 0 diffs, plus `strings` on a
  Windows TLS binary showing no OpenSSL/ALSA artifacts.

## Open Decisions

1. **What to do about a CNG gap found in Phase 1.** Recommended: reduce the advertised
   `runtime_calls` and reject that call at compile time on Windows, rather than emulating
   a primitive by hand. A hand-rolled crypto primitive is worse than an unsupported one.
2. **Whether `tls::listen`/accept is in scope.** Recommended: client-only first, and
   advertise only `tls.connect`-side calls. Schannel's server path needs a server
   certificate, which is a configuration surface this plan does not have.
3. **Certificate store selection.** Recommended: the default Windows machine/user store,
   with no custom trust root mechanism — matching the macOS backend's use of the system
   trust store.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **"the third sibling to `code/tls/{openssl,macos}.rs`" is misleading.**
  `tls/openssl.rs` is the **default** backend reached by an `else`, not the Linux one, so
  **Windows currently falls into OpenSSL** at `crypto_ec.rs:113`. Adding Schannel means
  editing a dispatch that has no third arm — not adding a file.
- 2026-07-20 — **`mod.rs:688` is a negated test** (`!contains("macos")`), so Windows
  would get the **ALSA** data objects: Linux audio library sonames baked into a Windows
  binary's `.rdata`. Called out as a specific Phase 4 task and a `strings` acceptance
  check.
- 2026-07-20 — **Windows needs no dlsym names at all.** The `mod.rs:703` EC dlsym data
  objects exist because OpenSSL is `dlopen`ed; CNG and Schannel are ordinary IAT imports.

## Summary

The engineering risk is certificate validation, because its failure mode is silent: a
Schannel handshake that validates nothing succeeds exactly like one that validates
correctly, and only a negative test distinguishes them. Phase 3's acceptance is therefore
three *failing* connections, not one succeeding one.

The design risk is the handshake shape. Both existing TLS backends expose the handshake
as roughly one call; `InitializeSecurityContext` is a caller-driven state machine with
partial-record handling, emitted as machine code. Phase 2 spikes it before anything is
built on top.

The structural correction is that this is not "add a third file" — it is "add a third arm
to a dispatch that has two", and until that arm exists Windows silently runs the OpenSSL
path and carries ALSA sonames in its binary.

What is left untouched: the `crypto::`/`tls::` language surface, the OpenSSL and macOS
backends, and the certificate-store policy, which is the platform's to decide.
