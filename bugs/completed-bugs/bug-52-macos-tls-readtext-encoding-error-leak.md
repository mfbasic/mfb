# bug-52: macOS `tls::readText` leaks the mapped `dispatch_data` and the retained content object on every invalid-UTF-8 read — a remote peer can drive an unbounded resource leak

Last updated: 2026-07-09
Effort: small (<1h)

In the macOS (Network.framework) TLS backend, `tls::readText` maps the received bytes
with `dispatch_data_create_map` (stored in `MAPPED`) and holds the retained nw content
object (`CTX_CONTENT`). On success both are released. But when the UTF-8 validation of
the received bytes fails, the handler branches to the `encoding_error` label, which is
emitted **after** the release block and jumps straight into `emit_fail(ERR_ENCODING)`,
skipping the two releases entirely. Each `readText` that receives invalid UTF-8
therefore leaks one `dispatch_data` map plus one retained content object.

The bytes are chosen by the peer, so a hostile client/server that keeps sending
non-UTF-8 to a program looping on `tls::readText` drives an unbounded,
remotely-triggerable resource leak — a memory-exhaustion DoS. The single correct
behavior a fix produces: the `encoding_error` exit releases `MAPPED` and `CTX_CONTENT`
just as the success exit does, so a failed decode leaks nothing.

References:

- `src/target/shared/code/tls/macos.rs:lower_tls_read_macos`. Release block (success
  path) `:1187-1210` (`dispatch_release(MAPPED)`, then `dispatch_release` of
  `[CTX].CTX_CONTENT`). `encoding_error` label `:1211-1220` — emitted after the release
  block, jumps to `emit_fail(ERR_ENCODING_CODE, …)` with no release. The UTF-8 check
  that branches there: `emit_call_validate_utf8` → `encoding_error` at `:1135`.
- Contrast: the `bytes` (non-text) read has no UTF-8 check and flows through the release
  block; the OpenSSL `readText` encoding-error path (`openssl.rs:1594`) leaks nothing
  because its buffer is arena memory, not a framework object.
- Security-core is otherwise sound (peer-cert verification via Network.framework's
  default verifying TLS; SNI/validation name via
  `sec_protocol_options_set_tls_server_name`); this is a resource-leak, not a
  verification bypass.
- Found during the goal-01 compiler source review of `src/target/shared/code/tls/`.

## Failing Reproduction

A TLS server (or client) that loops calling `tls::readText`, with a peer that sends
bytes which are not valid UTF-8 (e.g. a lone `0xFF`):

```
IMPORT tls
FUNC main AS Integer
  RES l AS TlsListener = tls::listen(8443, "cert.pem", "key.pem")
  RES c AS TlsSocket = tls::accept(l)
  WHILE TRUE
    LET s AS String = tls::readText(c) TRAP(e)
      RECOVER ""          ' invalid UTF-8 -> ErrEncoding, loop continues
    END TRAP
  WEND
  RETURN 0
END FUNC
```

Peer: repeatedly send a non-UTF-8 byte over the TLS connection.

- Observed (macOS): the process's `dispatch_data` / nw-content allocations grow without
  bound; RSS climbs one map + one content object per invalid read.
- Expected: memory stays flat; each `ErrEncoding` read releases its framework objects.

Contrast (works today): sending valid UTF-8 (the success path) releases both objects;
`tls::readBytes` never leaks (no UTF-8 check); the OpenSSL backend does not leak on
`ErrEncoding`.

## Root Cause

`lower_tls_read_macos` places the `dispatch_release(MAPPED)` + `dispatch_release(content)`
pair only on the straight-line success exit (`macos.rs:1187-1210`). The `text`-mode
UTF-8 failure branches to `encoding_error` (`:1211`), which is laid out after that block
and reaches `emit_fail` → `done` without passing through the releases. The `bytes` path
has no encoding check, so it never hits this exit.

## Goal

- The `encoding_error` exit of `tls::readText` releases `MAPPED` and the retained
  content object before failing.
- A loop of invalid-UTF-8 reads shows flat memory.

### Non-goals (must NOT change)

- The success path and `tls::readBytes`.
- The `ErrEncoding` code / trap behavior — only the missing releases are added.
- TLS verification behavior.

## Blast Radius

- `lower_tls_read_macos` `encoding_error` exit — fixed here.
- Success exit — already releases; the template.
- OpenSSL `readText` — unaffected.
- Audit the other macOS TLS error exits (connect/accept/close) for the same
  release-block-skip pattern; those are the LOW cluster (see the TLS/crypto error-path
  leak bug), but confirm none is remotely-triggerable like this one.

## Fix Design

Before branching to `emit_fail` on `encoding_error`, emit the same
`dispatch_release(MAPPED)` + `dispatch_release([CTX].CTX_CONTENT)` pair the success path
uses — or, cleaner, restructure so both the success and encoding-error exits fall
through a single shared release block before diverging to OK vs `emit_fail`. The shared
block is less error-prone against future exits.

## Phases

### Phase 1 — failing test

- [x] Add a TLS loop test that feeds invalid UTF-8 and asserts bounded framework-object
      allocation (or, minimally, a codegen assertion that the `encoding_error` path
      contains the two `dispatch_release` calls). Confirm the leak today.

### Phase 2 — the fix

- [x] Add the releases to the `encoding_error` exit (or share the release block).

### Phase 3 — validation

- [x] Regenerate macOS TLS goldens (delta = releases on the encoding-error path).
- [x] `scripts/test-accept.sh`; run the loop under `leaks`/Instruments on macOS.

## Validation Plan

- Regression test(s): the invalid-UTF-8 loop bounded-allocation test.
- Runtime proof: `leaks`/Instruments shows no growth across many `ErrEncoding` reads.
- Doc sync: none expected.
- Full suite: `scripts/test-accept.sh`.

## Summary

The encoding-error exit of macOS `tls::readText` was laid out past the release block, so
every invalid-UTF-8 read leaks a `dispatch_data` map and a content object — and the peer
controls the bytes, making it a remote DoS. The fix is to release on that path (or share
one release block); only the error exit changes.

## Resolution

Fixed in `src/target/shared/code/tls/macos.rs` (`lower_tls_read_macos`, `text` branch).
The `encoding_error` exit now performs the same two `dispatch_release` calls the
success path uses before jumping to `emit_fail(ERR_ENCODING)`:

- `dlsym("dispatch_release")` into `FNPTR`, then `dispatch_release(MAPPED)`,
  then `dispatch_release([CTX].CTX_CONTENT)`.
- `MAPPED`, `CTX`, and `CTX_CONTENT` are reloaded from stack slots for each call,
  so no live value sits in a caller-saved register across either `bl`
  (register-lifetime safe per `.ai/compiler.md`).
- Released exactly once per path: `encoding_error` is reached only via the
  `validate_utf8` branch, and the straight-line success release block ends in
  `branch(done)`, so the two exits never fall into each other. `tls::read`
  (bytes, `text == false`) has no UTF-8 check and is byte-identical — untouched.

The generated `encoding_error` release block is byte-identical to the success
release block (same slot offsets, same `blr` sequence), only ending in
`emit_fail` instead of the OK return.

### Runtime proof (macOS aarch64, `leaks` over ~200k invalid-UTF-8 reads)

A TLS server looping `tls::readText(sock, 1)` inside an inline `TRAP RECOVER`,
driven by a peer streaming lone `0xFF` bytes over a real Network.framework TLS
session:

- Before (unfixed): `412,155` total leaks, of which **`205,625` leaked
  `dispatch_data_t`** objects (~1 per invalid read), 32.9 MB.
- After (fixed): `211,088` total leaks, **`0` leaked `dispatch_data_t`**, 16.9 MB.
  Total drops by ~201k = ~2 objects/read — the `dispatch_data` map plus the
  retained content object, i.e. exactly the bug-52 pair.

### Regression test

`src/target/shared/code/tls/macos.rs` gained a `#[cfg(test)]` module
(`encoding_error_release_tests`) that lowers the read helper through a minimal
test `CodegenPlatform` and asserts the `encoding_error` exit contains two `blr`
(dispatch_release) calls and that `dispatch_release` is resolved on both exits;
a second test asserts `tls::read` has no `encoding_error` exit. The first test
fails on the pre-fix code (`0` releases, expected `2`) and passes after.

### Out of scope (separate, pre-existing leak observed while measuring)

The `leaks` residual after the fix (~211k leaks) is a **different** bug:
`emit_fresh_sem` creates a new `dispatch_semaphore` into `ctx->sem` on *every*
`tls::readText`/`write` call and overwrites the previous one without releasing
it, so each call leaks one semaphore regardless of success or error. This
affects the success path too and is outside bug-52's scope (the map/content
encoding-error leak). It belongs with the broader TLS error-/lifecycle-leak
cluster (bug-55 territory) and was left untouched here.

### Goldens

No acceptance golden shifts: no test under `tests/` that imports `tls` carries a
`.ncode` (native-code) golden, and the fix is purely in native codegen of the
macOS `readText` helper. `.ast`/`.ir`/`build.log` goldens are pre-codegen and
unaffected. The new coverage is an in-tree Rust unit test, not an acceptance
fixture.
