# bug-516: the NIST private-key encoding `0x04‖X‖Y‖d` is bespoke, and no doc says how to interoperate with it

Last updated: 2026-09-04
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: `scripts/man-run-examples.sh crypto --run` (the new example)

For `P256`/`P384`/`P521`, `crypto::generate` returns a `privateKey` that is the
SEC1 uncompressed public point immediately followed by the secret scalar —
`0x04‖X‖Y‖d`, 97/145/199 bytes. That layout is neither of the two things a
reader will assume it is. It is not the raw scalar `d` (which is what "raw
big-endian bytes, no DER wrapper" suggests), and it is not PKCS#8 or SEC1 DER
(which is what `openssl pkey` reads and writes).

`mfb man crypto generate` documents the *layout* precisely and documents that it
is wire-compatible across this package's own platforms. It never says the thing
a reader actually needs: that this is a package-local encoding, and what to do
when the key has to leave — or arrive from — OpenSSL, a JWK, a hardware token,
or any other ecosystem.

The single correct behavior a fix produces: the `crypto::generate` page states
plainly that the NIST private-key encoding is package-local, and shows the byte
surgery that converts to and from the two formats people will actually have
(SEC1 `d`, and PKCS#8/SEC1 DER via `openssl`).

References:

- `src/codegen/builtins/crypto/func_generate.rs:1561-1582` — the "Encodings and
  sizes" paragraph and the size table
- `mfb man crypto generate`
- SEC1 §C.4 (`ECPrivateKey`), RFC 5958 (PKCS#8)

## Failing Reproduction

```
./target/release/mfb man crypto generate
```

- Observed: the page says "Every field is raw big-endian bytes — no PEM, no
  base64, no DER wrapper on the key material itself", gives the table

  | `type` | `publicKey` | `privateKey` |
  | --- | --- | --- |
  | `P256` | 65 B (`0x04‖X‖Y`) | 97 B (`0x04‖X‖Y‖d`) |

  and then says "Across platforms the encodings are wire-compatible: a key made
  on one OS is accepted by `sign`/`verify` on the others." Nothing about
  interoperating with anything *outside* this package.

- Expected: a short "Interoperating with other tools" paragraph stating that the
  private-key form is package-local, that the trailing `field` bytes are the
  SEC1 scalar `d`, and giving the `openssl` incantation both directions.

Contrast case that is documented correctly today: the *public* key needs no
note — `0x04‖X‖Y` is the standard SEC1/X9.62 uncompressed point, and the page
names it as such. Only the private form is bespoke.

## Root Cause

Not a code defect. `func_generate.rs` builds the NIST private key by
concatenating the exported public point with the scalar
(`func_generate.rs:1161` — `raw = 0x04 ‖ (blob body X‖Y‖d)`), which is a
deliberate, reasonable choice: it makes `privateKey` self-sufficient, so
`sign` never needs the public half passed alongside. The gap is that the prose
documents the *what* and the *within-package* guarantee, and stops one sentence
short of the *outside-the-package* consequence.

## Goal

- `mfb man crypto generate` states that the NIST `privateKey` layout is
  package-local and not accepted by OpenSSL/JWK/PKCS#11 as-is.
- It shows how to recover the SEC1 scalar (the last `field` bytes) and how to
  rebuild a package `privateKey` from a scalar plus its public point.
- It shows one working `openssl` command in each direction.

### Non-goals (must NOT change)

- The encoding itself. Changing `0x04‖X‖Y‖d` would break every stored key and
  every `.ncodesum` golden that covers `generate`, for a documentation problem.
- The size table, which is correct.
- Adding DER/PEM import/export members to `crypto`. That is a real feature and
  a separate decision; this bug is the note that should exist either way.
- **Tempting wrong fix, forbidden:** deleting the "no PEM, no base64, no DER
  wrapper" sentence to make the page feel less contradictory. That sentence is
  true and useful; the missing content is what to do *about* it.

## Blast Radius

- `src/codegen/builtins/crypto/func_generate.rs` — the page fixed by this bug.
- `src/codegen/builtins/crypto/func_sign.rs`, `func_verify.rs` — they consume
  this layout and describe it by size only. Add a cross-reference to the new
  note; no prose rewrite needed.
- `src/codegen/builtins/crypto/func_exchange.rs` — unaffected: X25519/X448
  keys are the standard raw RFC 7748 forms and already interoperate.
- `tls::listen`'s `certPath`/`keyPath` — unaffected: those are PEM files read
  by the platform TLS stack, not `crypto::KeyPair` material. Worth one
  sentence in the new note precisely *because* readers will conflate them.
- `src/docs/spec/**` — check for any section restating the key encodings;
  `.ai/man-content.md` notes the spec is gated by nothing, so it drifts silently.

## Fix Design

Add one subsection to `func_generate.rs`'s `DESC`, after the size table, headed
"Interoperating with other tools". It needs three facts and one example:

1. The private form is package-local. Nothing outside MFBASIC reads it.
2. The SEC1 scalar `d` is the last `field` bytes (32/48/66 for P256/P384/P521);
   the leading `1 + 2*field` bytes are exactly the `publicKey`.
3. Going the other way, a package `privateKey` is `publicKey ‖ d`.
4. An `openssl ec` / `openssl asn1parse` command showing the extraction, and the
   inverse.

Per `.ai/man-content.md` the example must actually run, so the `openssl` half
belongs in prose and the MFBASIC half (slicing `privateKey` into `publicKey`
and `d` with `collections::mid`) belongs in the runnable example block.

Rejected: putting this in `src/docs/man/**` as a narrative topic. The
built-in package pages are rendered from the registry descriptors, so a
narrative page would not be reachable from `mfb man crypto generate` — which is
exactly where the reader is standing when the question occurs to them.

## Phases

### Phase 1 — the note

- [ ] Add the "Interoperating with other tools" subsection to
      `func_generate.rs`'s `DESC`, with the offsets for all three curves.
- [ ] Add a runnable example that slices a `P256` `privateKey` into its
      `publicKey` prefix and its 32-byte scalar, and asserts the prefix equals
      the returned `publicKey`.
- [ ] Cross-reference the note from `func_sign.rs` and `func_verify.rs`.

Acceptance: `mfb man crypto generate` renders the note;
`scripts/man-run-examples.sh crypto --run` compiles and runs the new example.
Commit: —

### Phase 2 — verify the claim before shipping it

- [ ] Actually run the documented `openssl` round trip against a key from
      `crypto::generate`, on a host whose `openssl` version is recorded. Do not
      publish an incantation that has not been executed.
- [ ] `scripts/man-census.sh --memory-scope` — the new prose must introduce no
      banned memory vocabulary.
- [ ] Check `src/docs/spec/**` for a stale restatement of the encodings.

Acceptance: the `openssl` commands in the page were run and their output
recorded in the commit message; the census reports 0 unclassified hits.
Commit: —

## Validation Plan

- Regression test: the new runnable example, executed by
  `scripts/man-run-examples.sh crypto --run`.
- Runtime proof: the `openssl` round trip in Phase 2, with the version pinned in
  the commit message (local `openssl` is much newer than CI's — do not assume
  the flags are portable).
- Doc sync: `func_generate.rs`, `func_sign.rs`, `func_verify.rs`, and any
  `src/docs/spec/**` section that restates the encodings.
- Full suite: doc-only change — scope the run to the man harness and
  `cargo test --no-fail-fast -- crypto`, not the full suite.

## Open Decisions

- Whether to follow this with real `crypto::exportPkcs8`/`crypto::importPkcs8`
  members. **Recommend deferring**: the note is worth having regardless, and a
  DER codec is a much larger piece of work that should be justified by a
  concrete demand rather than by symmetry.

## Summary

Low risk: a documentation change to a page that is already accurate but
incomplete, plus one runnable example. The only real requirement is that the
`openssl` commands be executed before being published — a wrong incantation in a
man page is worse than no incantation. No encoding, no code path, and no golden
changes.
