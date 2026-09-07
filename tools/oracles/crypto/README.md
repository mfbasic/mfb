# oracles/crypto — offline oracles for the `crypto` package's primitives

`crypto` is **software-first**: every primitive is implemented in MFBASIC over
`bits` so its output is byte-identical on every target and no platform crypto
library is called. That is the right design, and it has one consequence — *the
implementation has no second opinion*. A primitive that agrees with itself on
every architecture is exactly as wrong on all five if it is wrong at all.

This directory holds the second opinions. Like `tools/math-kernels`, it is
**offline tooling only**: nothing here is linked into the compiler or the runtime,
and nothing here runs in CI.

| Oracle | Covers | Run |
|---|---|---|
| [`argon2id/`](#argon2id--argon2id-v19-rfc-9106--blake2b-512-rfc-7693) | `crypto::argon2id`, and the BLAKE2b-512 under it | `argon2id/run.sh` |
| [`hash/`](#hash--every-hash-the-package-computes) | `crypto::hash` (all nine `Hash` variants) and `crypto::shake256` | `hash/run.sh` |
| [`mac-kdf/`](#mac-kdf--hmac-hkdf-and-pbkdf2-over-all-nine-hashes) | `crypto::hmac`, `crypto::hkdf`, `crypto::pbkdf2` — full `Hash` matrix | `mac-kdf/run.sh` |
| [`keys/`](#keys--the-public-key-matrix-both-directions) | `generate`, `sign`, `verify`, `exchange`, `convert` — every `Certificate` | `keys/run.sh` |

They share a shape: a `mfb/` **subject**, a `rust/` **judge**, and a `run.sh`
that builds both and plays them against each other, over the common plumbing in
`_lib/harness.sh`. (`keys/` inverts the last part — see its section.)

## Some of this package is checked in CI instead — read this first

`.ai/testing-gates.md` names three homes for an oracle and says the best one is
**the test file itself**, because it is the only one CI executes. That rule
decides what lives here:

| Member | Checked by | Why there |
|---|---|---|
| `hash`, `shake256`, `argon2id` | `tools/` (here) | needs a pinned third-party crate |
| `hmac`, `hkdf`, `pbkdf2` | **both** | `tests/interop/rt_crypto_mac_kdf_interop.rs` for every selector `ring` can compute; here for the SHA-3 family and SHA-224, which it cannot |
| `seal`, `open` | `tests/interop/rt_crypto_aead_interop.rs` | `ring` was already a dev-dependency |
| `generate`, `sign`, `verify`, `exchange`, `convert` | **both** | `tests/interop/rt_crypto_key_interop.rs` for Ed25519/X25519/P-256/P-384; here for Ed448, X448, P-521 |
| `encrypt`, `decrypt` | `tests/interop/rt_crypto_hpke_interop.rs` | already a bidirectional RFC 9180 interop proof, pinned to Appendix A.1/A.6 |
| `randomBytes`, `randomInt`, `uuid4`, `uuid7`, `ulid` | nothing, deliberately | no oracle is possible for a random value; only distributional or format properties, which are not what an oracle is for |
| `constantTimeEqual` | nothing, deliberately | it computes equality; a second opinion on `==` adds no information |

The overlaps are deliberate. Where a member is checked in both places the
coverage is arranged to intersect rather than abut, so that a disagreement
*between the two references* would surface rather than hide in a seam.

The test in `tests/` is preferred whenever the crate it needs is **already in
the lockfile** — that is the repo's own bar, recorded in the dev-dependency
comments in `Cargo.toml`. A member ends up here when checking it would mean new
compiled code in every CI job on five platforms for a matrix only this tool
needs.

## Why an oracle and not a test

A test built from values the implementation produced ratifies the implementation.
Two rules follow, and both were live problems while `crypto::argon2id` was
written (bug-515):

1. **Fetch the published vectors; never recite them.** A hallucinated test vector
   is worse than no vector, because it looks like evidence. The RFC texts are
   retrievable — `curl -O https://www.rfc-editor.org/rfc/rfc9106.txt` — so
   retrieve them.
2. **Cross-check against a codebase that did not read yours.** Agreement between
   your reference and your implementation is weak when you wrote the reference by
   reading the implementation. Three independent codebases currently agree on
   Argon2id here: this reference, RustCrypto `argon2`, and OpenSSL.

See also `.ai/testing-gates.md`, and the standing note that a **wrong oracle is
ratified, not caught** — GPU-vs-oracle agreement cannot see an oracle bug.

## `argon2id/` — Argon2id v19 (RFC 9106) + BLAKE2b-512 (RFC 7693)

The oracle for `crypto::argon2id` and the BLAKE2b-512 it is built on.

The directory has two halves — `mfb/` is the **subject**, `rust/` is the
**judge** — and `run.sh` plays them against each other.

| Piece | Role |
|---|---|
| `argon2id/run.sh` | The differential check. Builds `mfb/`, runs it, re-derives every tag with `rust/`, compares. Start here. |
| `argon2id/mfb/` | An MFBASIC project calling `crypto::argon2id` over a spread of costs and edges. It prints the parameters it used next to each digest. |
| `argon2id/rust/src/main.rs` | Clean-room reference, structured to mirror the MFBASIC core it validates, so a divergence localises to a step rather than to "the tag is wrong". |
| `argon2id/rust/Cargo.toml` | Its OWN `[workspace]` — deliberately not a member of the mfb workspace. See the comment in the file. |
| `argon2id/rust/Cargo.lock` | Committed. The pin IS the oracle: `argon2 = "=0.5.3"` is the version whose agreement was measured. |
| `argon2id/rust/openssl-xcheck.sh` | The third opinion, from OpenSSL's own `ARGON2ID` KDF. |

### Running the differential check

```sh
tools/oracles/crypto/argon2id/run.sh          # builds mfb + rust, compares
tools/oracles/crypto/argon2id/run.sh /path/to/mfb   # or point it at an mfb binary
```

It builds the `mfb` compiler if `target/release/mfb` is missing. Expected tail:

```
OK   m=19456 t=2 p=1 l=32 pw=70617373776f7264 salt=736f6d…  0c4c0b6db2…
argon2id mfb-vs-rust: 12 case(s), 0 failure(s)
```

Exit codes match `openssl-xcheck.sh`: `0` all agreed, `1` a case disagreed,
`2` the harness could not run (build failure, no cases, or a case COUNT other
than the `EXPECTED_CASES` pinned in `run.sh`).

Two things make the comparison honest rather than circular:

- **`mfb/` prints its own inputs, and `run.sh` feeds those to `rust/`.** There is
  no case table duplicated between the two, so the sides cannot silently drift
  onto different parameters and still agree.
- **The case count is pinned in `run.sh`, not counted from `mfb/`'s output.** A
  count derived from the producer is true by construction — if the program died
  after case 3, "3 of 3 agreed" would read as a pass. Keep `EXPECTED_CASES` in
  step with the `emit(...)` calls in `mfb/src/main.mfb`.

### Running the Rust reference alone

```sh
cd tools/oracles/crypto/argon2id/rust
cargo run --release            # self-check + RustCrypto cross-check
```

It checks itself against the published vectors first and only then cross-checks,
and it exits non-zero on any mismatch. Expected tail:

```
blake2b-512(abc) OK ba80a53f981c4d0d…
argon2id rfc9106 OK 0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659
xcheck m=8 t=1 p=1 l=32 OK d838041400…
…
failures=0
```

The `rfc9106` line is RFC 9106 §5.3's own tag. Note that vector carries a
**secret and associated data**, which `crypto::argon2id` deliberately does not
expose — so the reference reaches it and the shipped member cannot. That is why
the committed fixture pins the no-secret case at the RFC's cost parameters
instead of surface being invented to reach one vector.

The third opinion, also from `rust/` (its `target/release/argon2ref` path is
relative, so run it from there):

```sh
./openssl-xcheck.sh                                       # needs OpenSSL >= 3.2
./openssl-xcheck.sh /opt/homebrew/opt/openssl@3/bin/openssl
```

macOS ships **LibreSSL** as `openssl`, which has no `ARGON2ID` KDF; the script
detects that and exits 2 (skip) rather than reporting a pass. It also refuses to
exit 0 having run zero cases — a harness that runs nothing must not read as green.

### Deriving a new vector

Also from `rust/`. This is the mode `run.sh` drives, one case per invocation:

```sh
cargo run --release -- run <password-hex> <salt-hex> <m> <t> <p> <len>
```

### What is committed as the pin

`tests/rt-behavior/crypto/crypto-argon2id-valid` is the fixture. It is the thing
CI runs; this directory is how its expected values were *obtained*, and how to
re-obtain them if the core changes.

Two properties it records that are easy to get wrong:

- **`memoryKiB` is bound into `H_0`**, so `m=32` and `m=33` at `p=4` round to the
  same 32 blocks but derive **different** keys. Rounding is not equivalence.
- The profile overload is a **wrapper**: `Argon2Profile.Minimum` is byte-identical
  to the explicit `(19456, 2, 1)` call. That equality is what proves it is not a
  second implementation.

`(19456, 2, 1)` is also `Argon2::default()` in `repository/src/crypto.rs`, so an
MFBASIC program can reproduce the repository's pairing-key derivation exactly.

## `hash/` — every hash the package computes

The oracle for `crypto::hash` and `crypto::shake256`. Same shape as `argon2id/`
— `mfb/` is the subject, `rust/` is the judge, `run.sh` plays them against each
other — but the comparison is the simple one: input bytes in, digest hex out.

Ten algorithm/width combinations, which is the whole public hash surface: the
nine `crypto::Hash` variants (`SHA1`, `SHA2_224/256/384/512`,
`SHA3_224/256/384/512`) plus `crypto::shake256` at two widths, because an XOF
whose length parameter were ignored would still match at one of them.

| Piece | Role |
|---|---|
| `hash/run.sh` | The differential check. Builds `mfb/`, runs it, re-derives every digest with `rust/`, compares. Start here. |
| `hash/mfb/` | An MFBASIC project hashing 37 inputs with all ten. It prints the input it hashed beside each digest. |
| `hash/rust/src/main.rs` | The reference. **Not** hand-written — see below. |
| `hash/rust/Cargo.lock` | Committed. The pin IS the oracle: `sha1 =0.10.6`, `sha2 =0.10.8`, `sha3 =0.10.8`. |
| `hash/rust/openssl-xcheck.sh` | The third opinion, from OpenSSL's `dgst`. |

```sh
tools/oracles/crypto/hash/run.sh
```

```
sha1       37/37  agreed
…
shake256   74/74  agreed
crypto::hash mfb-vs-rust: 407 case(s), 0 failure(s)
```

Exit codes and the count-pinning discipline are the same as `argon2id/`'s;
`EXPECTED_PER_INPUT` and `EXPECTED_INPUTS` in `run.sh` must stay in step with
the `emitOne(...)` calls and the input list in `mfb/src/main.mfb`.

### Why this reference is not hand-written

`argon2id/rust` is a clean-room transliteration because the MFBASIC Argon2id is
a large custom construction and a divergence needs to localise to a *step*. A
hash is not like that: the answer is one value, so what an oracle has to supply
is **independent authorship**, and hand-transliterating SHA-3 here would produce
a second implementation by the same author as the first. RustCrypto is a
different codebase maintained by people who never read the MFBASIC core, so it
is the stronger oracle precisely because none of it was written here.

That leaves the reference itself unchecked by `run.sh`, which is what
`openssl-xcheck.sh` is for — it compares the *reference* against OpenSSL rather
than MFBASIC against anything. Algorithms an `openssl` cannot compute (LibreSSL
has no SHA-3 or SHAKE) are skipped and counted; a run that skipped everything
exits 2 rather than reporting a pass.

### The spread

Inputs are chosen by LENGTH, because that is where hash bugs live — the padding
and the block/rate boundary. 34 generated lengths straddle every boundary the
two families have (64-byte blocks for SHA-1/224/256, 128 for SHA-384/512, and
rates 144/136/104/72 for the SHA-3 widths), each probed at rate−1, rate and
rate+1 so the one-byte padding case gets its own case. Three text inputs follow,
one carrying a non-ASCII scalar.

`SHA1` is included deliberately and its `CRYPTO_SHA1_INSECURE` build warning is
expected: it is still a hash this package computes, and skipping it would leave
the algorithm most likely to be quietly broken unchecked. The warning is about
choosing SHA-1, not about computing it.

## `mac-kdf/` — HMAC, HKDF and PBKDF2 over all nine hashes

The oracle for `crypto::hmac`, `crypto::hkdf` and `crypto::pbkdf2`. Same shape
as `hash/`: `mfb/` emits `case` lines carrying the inputs it used, `run.sh`
feeds those to `rust/`, and the digests are compared byte for byte.

189 cases — 21 input sets across the full nine-selector `crypto::Hash` matrix.
`tests/interop/rt_crypto_mac_kdf_interop.rs` already covers SHA-1 and the SHA-2 widths
on every `cargo test`; what only exists here is the **SHA-3 family** (no `ring`
equivalent) and **SHA-224 for the two KDFs**. Reaching those needs `sha3`,
`hkdf` and `pbkdf2`, which would be new compiled code in every CI job.

```sh
tools/oracles/crypto/mac-kdf/run.sh
```

HMAC key lengths straddle every block size in the matrix (64 for SHA-1/224/256,
128 for SHA-384/512, and the SHA-3 rates 144/136/104/72), because a key longer
than the block is **hashed first** and that reduction is the step most likely to
be wrong. HKDF probes an empty salt and a non-multiple output length; PBKDF2
uses 1 and 2 iterations to separate "did the loop run" from "did it run the
right number of times".

## `keys/` — the public-key matrix, both directions

The oracle for `crypto::generate`, `crypto::sign`, `crypto::verify`,
`crypto::exchange` and `crypto::convert`, over every `crypto::Certificate` and
both `KeyConvert` directions.

**This one inverts the usual shape.** The other oracles are one-shot: the MFB
program emits every case and the shell compares. Key interop cannot work that
way, because the questions depend on the answers — you have to *see* a key MFB
generated before you can ask it to sign with that key, and see its public half
before you can compute the matching shared secret. So `rust/` is the **driver**,
`mfb/` is a tiny RPC server reading a job from `MFB_KEY_JOB`, and `run.sh` is
thin.

```sh
tools/oracles/crypto/keys/run.sh
```

What each curve is asked, and why each half is needed:

| Direction | Claim |
|---|---|
| MFB generates | we re-derive its public key from its private key. A pair that fails its own definition is broken however well it round-trips. |
| MFB signs | we verify — and for the deterministic schemes (Ed25519, Ed448) compare the signature **byte for byte**, which a round trip cannot do: a signer that chose its nonce differently would still verify. |
| we sign | MFB verifies, **and rejects a corrupted signature**. Without the second half a `verify` that always returned true would pass. |
| ECDH | each side uses its own private key and the other's public key; the secrets must match. |

For the NIST curves the subject is the **encoding contract** rather than an
MFBASIC core — those bind the platform key API (SecKey / EVP_PKEY / CNG). The
external representation is `04‖X‖Y` for a public key and `04‖X‖Y‖d` for a
private one, with ASN.1 DER signatures; the oracle splits `d` off and checks
that `d·G` really is the reported public key. ECDSA signing is randomized, so
there is nothing to compare byte for byte and only the round trip is available.

`ed448-rust` is pinned rather than `ed448-goldilocks`: the latter's released
versions expose only curve arithmetic, and its signing API exists only in a
0.14 prerelease.
