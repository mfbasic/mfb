# bug-514: `crypto::KeyPair` carries no curve tag, so `convert`/`encrypt` accept the wrong curve's key and fail silently

Last updated: 2026-09-04
Effort: large (3h–1d)
Severity: HIGH
Class: Security

Status: Open
Regression Test: `tests/` — new `rt_crypto_curve_mismatch` fixture (Phase 1)

`crypto::KeyPair` is a two-field record (`publicKey`, `privateKey`, both
`List OF Byte`) with nothing on it that says which curve produced it. Ed25519
and X25519 key material is 32 bytes on both halves; Ed448 is 57 and X448 is 56.
`crypto::convert` therefore validates only the *length* of the pair, and
`crypto::encrypt`/`crypto::decrypt` do not take a `KeyPair` at all — they take a
bare `List OF Byte` recipient key and run the Ed→Montgomery map on it
internally. Handing an X25519 public key to an `Ed25519_*` HPKE suite passes
every check the package makes, produces a well-formed box, and yields
ciphertext **no private key in existence can open**.

The single correct behavior a fix produces: key material generated for one
curve cannot be consumed by an operation for a different curve without a
diagnostic. Either the type system distinguishes them (a `Certificate` tag on
`KeyPair`, or distinct per-curve record types), or the runtime rejects the
mismatch with `ErrInvalidArgument`. What must stop is the current outcome —
success, with wrong bytes.

This is the dangerous class: no error, no warning, and the failure surfaces at
the *recipient*, arbitrarily later, as an undecryptable message. `mfb man crypto
convert` already concedes it in a paragraph headed "No curve tagging"; the type
system does not.

References:

- `mfb man crypto convert` — "No curve tagging" paragraph
- `mfb man crypto encrypt` — `recipientPublicKey` is `List OF Byte`
- `src/codegen/builtins/crypto/func_convert.rs`, `func_encrypt.rs`, `func_generate.rs`
- Spike: `spikes/api-review/bug-514-keypair-untagged/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-514-keypair-untagged
./spikes/api-review/bug-514-keypair-untagged/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
Ed25519 public key is 32 bytes; X25519 public key is 32 bytes -- the same length.

convert, real Ed25519 pair: CONVERTED, pub starts 126 12 86 48 215 160 8 74
convert, an X25519 pair   : CONVERTED, pub starts 24 228 198 241 175 136 183 158

encrypt to the Ed25519 public key: 62-byte box
  with the matching Ed25519 private key: OPENED 14 bytes

encrypt to the X25519 public key : 62-byte box -- accepted, no error
  with the X25519 private key : RAISED code=77050016
  with the Ed25519 private key: RAISED code=77050016
```

- Expected: the second `convert` and the second `encrypt` are refused —
  at compile time if the tag is carried in the type, otherwise with
  `ErrInvalidArgument` (77050002).

Contrast cases that behave correctly today, and must keep doing so:

- Ed448 material handed to `Ed25519ToX25519` **is** caught — but only because
  57 ≠ 32. The length check is the whole guard, and it is right by accident.
- The matching Ed25519 pair round-trips (`OPENED 14 bytes`), so a fix must not
  break the supported single-identity-for-both-primitives convenience that
  `mfb man crypto convert` documents.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | not yet run; the Ed25519/X25519 cores are pure MFBASIC software (`func_generate.rs`, "Implementation") so the same result is expected — confirm in Phase 3 |

## Root Cause

`crypto::KeyPair` is declared as a plain `RegistryRecord` of two `List OF Byte`
fields (`src/codegen/builtins/crypto/mod.rs`, the `KeyPair` row). Nothing on the
value records which `crypto::Certificate` variant `crypto::generate` was called
with, so:

- `crypto::convert` (`func_convert.rs`) can only branch on
  `len(publicKey)`/`len(privateKey)`. Its documented guard — "a pair whose
  halves are not both 57 bytes (Ed448ToX448) or both 32 bytes
  (Ed25519ToX25519) raises `ErrInvalidArgument`" — is exactly a length test,
  and Ed25519 and X25519 are indistinguishable under it.
- `crypto::encrypt`/`crypto::decrypt` (`func_encrypt.rs`, `func_decrypt.rs`)
  do not accept a `KeyPair`; the recipient key is a bare `List OF Byte`, so
  even a tagged `KeyPair` would not help these two without a signature change.
  They apply the same Ed→Montgomery map internally, so a raw Montgomery `u`
  coordinate is decoded as an Edwards `y` and re-mapped — arithmetic that is
  perfectly well-defined and completely wrong.

The AEAD tag is what eventually notices, at `decrypt`, on a different machine.

## Goal

- Handing curve-A key material to a curve-B operation is rejected, with the
  rejection visible to the *sender*, not deferred to the recipient's failed
  `decrypt`.
- `crypto::generate` output can be traced to the curve that produced it, by the
  compiler or by the runtime.
- The "No curve tagging" paragraph in `mfb man crypto convert` becomes
  unnecessary and is deleted.

### Non-goals (must NOT change)

- The wire format. `encrypt` must keep emitting RFC 9180 `enc ‖ ct`, byte-identical
  and interoperable with conformant HPKE implementations; `generate` must keep
  its documented raw big-endian encodings and sizes.
- The deliberate single-identity convenience: an Ed25519 pair must still be
  usable for both `sign` and `encrypt`, via the documented internal conversion.
- Cross-platform byte-identity of the software curves.
- **Tempting wrong fix, forbidden:** widening the length check into a heuristic
  ("does this decode as a valid Edwards point?"). A valid X25519 public key can
  decode as a valid Edwards `y`; the test would pass and the bug would remain.
  The curve must be *carried*, not *guessed*.

## Blast Radius

Found by `grep -rn "KeyPair" src/codegen/builtins/crypto/`:

- `func_convert.rs` — fixed by this bug (the length-only guard).
- `func_encrypt.rs`, `func_decrypt.rs` — fixed by this bug; they take raw
  `List OF Byte` and need a curve-carrying parameter or an explicit check.
- `func_generate.rs` — fixed by this bug; it is where the tag must be attached.
- `func_sign.rs`, `func_verify.rs` — **latent, same hazard.** They take an
  explicit `crypto::Certificate` argument *and* raw key bytes, so a caller can
  still pass P256 bytes under `Certificate.P384`. Length differs between all
  the NIST curves and between Ed25519/Ed448, so the current length checks catch
  every cross-curve pairing except Ed25519↔X25519 — and `sign`/`verify` already
  reject `X25519`/`X448` outright. In scope only for the tag design; no
  behavior change required.
- `func_exchange.rs` — **latent.** Takes X25519/X448 material. The inverse
  mistake (an Ed25519 pair into `exchange`) is the same length confusion, and
  should be covered by whatever tag `generate` attaches.
- `gen_cert.rs` — unaffected; it operates on platform key handles, not on
  `KeyPair` values.
- `tls::listen`'s `certPath`/`keyPath` — unaffected; PEM files carry their own
  algorithm identifiers.

## Fix Design

Two shapes, and the choice is a public-surface decision (see Open Decisions).

**A — tag the record.** Add a `curve AS crypto::Certificate` field to
`crypto::KeyPair`, set by `crypto::generate` and by `crypto::convert`'s output.
`convert` then checks the tag instead of the length; `encrypt`/`decrypt` gain
`KeyPair`-taking overloads that check it. Cheapest to implement, keeps one type,
and the tag is a runtime value — so the diagnostic is a runtime
`ErrInvalidArgument`, not a compile error. Adds a field to a public record,
which is a source-compatibility break for positional construction.

**B — distinct record types per curve family.** `crypto::SigningKeyPair` and
`crypto::ExchangeKeyPair` (or one per curve). Mismatches become
`TYPE_CALL_ARGUMENT_MISMATCH` at compile time — the strongest possible answer,
and the one that makes the docs' caveat go away entirely. Much larger surface
change: every `crypto::` member taking key material, plus the man pages.

Rejected: keeping `KeyPair` untagged and adding a `expectedCurve` argument to
`convert`. It moves the burden to the caller who already believes they have the
right key, so it catches nothing the current length check does not.

Whichever shape is chosen, `encrypt`/`decrypt` must stop taking bare
`List OF Byte` recipient keys, or the tag is unreachable at the call that
matters most.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-514-keypair-untagged/` (done) and add a
      runtime fixture asserting the *desired* rejection, marked failing.
- [ ] Re-run the spike on Linux and Windows; fill in the environment matrix.
- [ ] Confirm the `sign`/`verify`/`exchange` verdicts above by reading each
      member's length guard; write the result into the Blast Radius list.

Acceptance: the fixture fails for the documented reason; the matrix has a row
per platform; every `crypto::` member taking key material has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Resolve the Open Decision, then attach the curve at `crypto::generate`.
- [ ] Check the tag in `convert`, `encrypt`, `decrypt` (and `exchange` if shape B).
- [ ] Update the affected `RegistryFunction` prose; delete the "No curve
      tagging" paragraph from `func_convert.rs`.

Acceptance: the Phase 1 fixture passes; the matching-curve paths still succeed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate `.ncode`/`.ncodesum` goldens the signature change shifts;
      confirm the delta is only crypto's.
- [ ] `cargo test --no-fail-fast`, then `scripts/test-accept.sh`.
- [ ] Re-run the spike on every platform in the matrix; confirm the second
      `convert` and second `encrypt` are now refused.
- [ ] `scripts/man-run-examples.sh crypto --run`.

Acceptance: full suite green; golden deltas are only the intended ones; the
reproduction is refused everywhere it previously succeeded.
Commit: —

## Validation Plan

- Regression test: the Phase 1 fixture — X25519 material into every
  `Ed25519_*` entry point, each asserted to raise.
- Runtime proof: `spikes/api-review/bug-514-keypair-untagged/` reruns and the
  two silent successes become diagnostics.
- Doc sync: `func_convert.rs` (drop the caveat), `func_encrypt.rs`,
  `func_decrypt.rs`, `func_generate.rs` prose; `src/docs/spec/**` if it
  describes `KeyPair`.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Shape A (tag field, runtime check) vs. shape B (distinct types, compile-time
  check). **Recommend B** for `encrypt`/`decrypt` and `exchange`, where the
  mistake is silent and remote; B is the only option that makes the failure
  impossible rather than merely reported. If B is too large for one change,
  land A first — it removes the silence, which is the dangerous part — and
  record B as the follow-up.

## Summary

The engineering risk is entirely in the public-surface change: adding a field
to `crypto::KeyPair` or splitting it breaks source compatibility for anyone
constructing one positionally, and `encrypt`/`decrypt` need a signature change
to see the tag at all. The cryptographic cores, the wire format, and the
platform key-generation seams are untouched.
