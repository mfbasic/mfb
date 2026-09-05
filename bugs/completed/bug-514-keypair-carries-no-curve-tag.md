# bug-514: `crypto::KeyPair` carries no curve tag, so `convert`/`encrypt` accept the wrong curve's key and fail silently

Last updated: 2026-09-04
Effort: large (3h–1d)
Severity: HIGH
Class: Security

Status: **Closed — fixed** (`<HASH>`). `crypto::convert` now proves the source
curve from the pair instead of inferring it from the key length, so the silent
wrong-curve path it owned is gone. The `encrypt`/`decrypt`/`exchange`/`sign`
half of the report is **not fixable as reported** — it is undecidable, not
unimplemented — and is now stated as a contract on every page and in the spec
rather than conceded on one page where nobody looking at `encrypt` would find
it. See "Verdict" and "What was NOT changed, and why" below.
Regression Test: `tests/rt-behavior/crypto/crypto-curve-attribution-valid`

`crypto::KeyPair` is a two-field record (`publicKey`, `privateKey`, both
`List OF Byte`) with nothing on it that says which curve produced it. Ed25519
and X25519 key material is 32 bytes on both halves; Ed448 is 57 and X448 56.
`crypto::convert` therefore validated only the *length* of the pair, and
`crypto::encrypt`/`crypto::decrypt` do not take a `KeyPair` at all — they take a
bare `List OF Byte` recipient key and run the Ed→Montgomery map on it
internally.

## Verdict: which side was wrong

Established by rendering `mfb man crypto convert` / `encrypt` / `types` and
reading `mfb spec stdlib` before touching anything. The two disagreed:

- **`mfb spec stdlib` §crypto, "Key conversion"** stated, unconditionally:
  "Both maps check the input lengths … and raise `ErrInvalidArgument`
  otherwise, **so a pair from the other curve is rejected rather than
  mis-mapped**." That is a categorical guarantee. It was false: an X25519 pair
  is 32 bytes on both halves, passes the length test, and was mapped.
- **`mfb man crypto convert`** contradicted the spec with a paragraph headed
  "No curve tagging" that conceded the defect as a limitation.
- **`mfb man crypto types`** said nothing at all about the curve on `KeyPair`.

So for `convert`, **the code was the wrong side**: the spec already promised the
behaviour, and the length test was a proxy standing in for it. For every other
member, **the docs were the wrong side**: nothing on the pages a developer
actually reads when passing a key (`encrypt`, `decrypt`, `exchange`, `sign`)
said the key half is unattributable, and the one page that admitted it is the
one place where it was fixable.

## The fix

`convert` is the **only** `crypto` member handed both halves of a single key
pair, and that is exactly what makes the curve decidable there. RFC 8032 defines
a signing public key *as* a derivation of the seed, so `__crypto_convert` now
re-derives it and requires equality before either map runs:

- `Ed25519ToX25519` — `A = [clamp(SHA-512(seed)[0..32])]B` must equal
  `keys.publicKey`;
- `Ed448ToX448` — `A = [prune(SHAKE256(seed, 114)[0..57])]B` must equal
  `keys.publicKey`;

compared with `__crypto_constantTimeEqual`; a mismatch raises `ErrInvalidArgument`
(77050002).

This is **not** the heuristic the report forbids ("does this decode as a valid
Edwards point?"). Nothing about the public key's own encoding is inspected. It is
a derivation check with the private half as the witness: `A = [s]B` holds for
every conformant RFC 8032 pair by construction and for an X25519 pair with
probability ≈ 2⁻²⁵², so the check accepts every legitimate key and rejects the
wrong curve. Cost: one fixed-base scalar multiplication (about one
`crypto::sign`), paid once at key setup.

The docs were then made to match: the "No curve tagging" paragraph is deleted and
replaced with the guarantee; `KeyPair`'s type-page description now states that
nothing on the record names the curve; and `encrypt`, `decrypt`, `exchange` and
`sign` each carry a paragraph naming the limit precisely where the developer is
holding the wrong key.

## What was NOT changed, and why

The report's second silent success — `encrypt(Ed25519_*, x25519PublicKey, …)`
producing an unopenable box — was **not** closed, and it cannot be closed as the
report frames it. Both the report's Fix Design shapes were weighed against the
pass's **no language-surface change** constraint:

- **Shape A (add a `curve` field to `KeyPair`)** is a source-compatibility break
  outright. `KeyPair` is constructed positionally — the compiler's own
  `__crypto_convert` writes `KeyPair[xPriv, xPub]`, and so does
  `tests/rt-behavior/crypto/crypto-x448-valid` — so a third field breaks every
  correct program that builds one. It also would not reach `encrypt`, which takes
  no `KeyPair`.
- **Shape B (distinct per-curve record types)** is the strongest answer and
  the report recommends it, but it changes the declared type of every `crypto`
  member taking key material. That is precisely a language-surface change.

The bug-521 precedent in this pass — add an overload no *correct* program could
have used — was considered and does not rescue `encrypt`. An
`encrypt(cipher, recipient AS crypto::KeyPair, …)` overload would indeed be
semantics-preserving, but the sender does not hold the recipient's private key,
so the checked path would be unreachable in the flow that matters; and
`decrypt`'s wrong-curve outcome is already *loud* (`ErrAuthenticationFailed`),
so an overload there would improve an error message, not close a silent path.
Adding four public implementations to buy neither was rejected as surface for
nothing.

The underlying fact is information-theoretic, and is now recorded as a contract:
**a wrong curve is decidable exactly when a member receives both halves of one
pair.** A lone 32-byte public key or a lone 32-byte seed carries no curve, and no
amount of checking can attribute it — a Montgomery `u` decodes as a valid Edwards
`y` about half the time, so a shape test would be a coin flip. `encrypt`,
`decrypt`, `exchange`, `sign` and `verify` each see one half, take the curve from
their `AsymmetricCipher`/`Certificate` selector, and trust the bytes. That is now
said on each of those pages and in `mfb spec stdlib` §crypto ("Curve
attribution"), and it is pinned in the regression fixture as
`encrypt-bare-key-is-unattributable=accepted 62 bytes`, so any future change to
it moves a golden and forces the docs to be updated with it.

A follow-up that wants the compile-time answer is shape B, and it is a
language-surface change that needs its own budget.

## Reproduction — before and after

`spikes/api-review/bug-514-keypair-untagged`, macOS aarch64 release:

```
                                        BEFORE (354f786c3)              AFTER
convert, real Ed25519 pair:             CONVERTED, pub starts 66 …      CONVERTED, pub starts 138 …
convert, an X25519 pair   :             CONVERTED, pub starts 228 …     RAISED code=77050002
encrypt to the Ed25519 public key:      62-byte box / OPENED 14 bytes   unchanged
encrypt to the X25519 public key :      62-byte box, no error           unchanged (undecidable — see above)
```

The regression fixture is the precise form. Its RED line, run against a release
binary built from `354f786c3` in a throwaway worktree:

```
BEFORE  convert-x25519-pair-into-ed25519-map=a8cd44eb…c94f/53efd1da…d05f
AFTER   convert-x25519-pair-into-ed25519-map=ErrInvalidArgument
```

— a genuine RFC 7748 §6.1 X25519 pair, silently mapped to garbage with no
diagnostic, now refused. `convert-mismatched-ed25519-halves` (one identity's
seed beside another's public key) was silent in exactly the same way and is
refused too.

Every other line of the fixture is **byte-identical before and after**, including
the three official-vector conversions, the RFC 8032 §7.1 signature, and the
RFC 7748 §6.1 shared secret.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fixed ✓ |
| Linux / Windows / riscv64 | — | the Ed25519/Ed448/X25519/X448 cores are pure MFBASIC software, so the result is target-independent by construction; the four cross-target `.ncodesum` goldens were regenerated and the artifact gate is clean for all five targets |

## Blast Radius — final verdicts

Confirmed by reading each member's guard, per Phase 1's acceptance:

- `func_convert.rs` — **fixed.** Length proxy replaced by a derivation check.
- `func_encrypt.rs`, `func_decrypt.rs` — **undecidable, documented.** One key
  half; no check possible without a language-surface change.
- `func_generate.rs` — **unchanged.** Nothing to attach a tag to without shape
  A or B; it already derives the public half from the private one, which is what
  makes `convert`'s check work.
- `func_sign.rs`, `func_verify.rs` — **undecidable within a size, documented.**
  Every distinct signing curve has a distinct key size, so all cross-curve
  pairings except an X25519 key under `Certificate.Ed25519` are caught on
  length; that one is accepted and yields a valid signature under an unpublished
  public key. `sign`'s page now says so.
- `func_exchange.rs` — **undecidable, documented.** Any 32 bytes are a usable
  clamped X25519 scalar, so an Ed25519 seed is accepted and yields a secret the
  peer never reproduces. The page now says so and points at `convert`.
- `gen_cert.rs`, `tls::listen` — unaffected, as reported.

## Validation

- `tests/rt-behavior/crypto/crypto-curve-attribution-valid` — RED case plus a
  positive pin on every legitimate flow, built from official offline vectors:
  RFC 8032 §7.1 TEST 1 and TEST 2 (Ed25519 seed/public/signature), RFC 8032 §7.4
  test 1 (Ed448), RFC 7748 §6.1 (X25519 Alice and Bob, and their shared secret),
  RFC 7748 §6.2 (X448 Alice).
- `scripts/artifact-gate.sh target/release/mfb all` — **0 diffs** over 1908
  goldens. Before regeneration the delta was 26 goldens, every one under a
  `crypto` path: 17 `.ir` (line-number shifts from the six added lines of helper
  body) and 9 `.ncodesum`.
- `scripts/test-accept.sh` — 1397 tests, all passing. **No `build.log` and no
  `.run` golden moved anywhere in the tree**, which is the semantics-preservation
  proof: the only crypto fixture output that changed is the new one.
- `cargo test --no-fail-fast` — green.
- `scripts/man-run-examples.sh crypto --run` — 29 examples, 29 ran, 0 failed.
- `scripts/man-census.sh --memory-scope` — 0 unclassified hits, tree-wide.
- Doc sync: `mfb man crypto convert|encrypt|decrypt|exchange|sign|types`
  rendered and read; `src/docs/spec/stdlib/10_crypto.md` updated in both the
  "Key conversion" bullet and a new "Curve attribution" bullet.
