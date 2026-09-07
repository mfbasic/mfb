# oracles/crypto — offline oracles for the `crypto` package's primitives

`crypto` is **software-first**: every primitive is implemented in MFBASIC over
`bits` so its output is byte-identical on every target and no platform crypto
library is called. That is the right design, and it has one consequence — *the
implementation has no second opinion*. A primitive that agrees with itself on
every architecture is exactly as wrong on all five if it is wrong at all.

This directory holds the second opinions. Like `tools/math-kernels`, it is
**offline tooling only**: nothing here is linked into the compiler or the runtime,
and nothing here runs in CI.

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

| Piece | Role |
|---|---|
| `argon2id/src/main.rs` | Clean-room reference, structured to mirror the MFBASIC core it validates, so a divergence localises to a step rather than to "the tag is wrong". |
| `argon2id/Cargo.toml` | Its OWN `[workspace]` — deliberately not a member of the mfb workspace. See the comment in the file. |
| `argon2id/Cargo.lock` | Committed. The pin IS the oracle: `argon2 = "=0.5.3"` is the version whose agreement was measured. |
| `argon2id/openssl-xcheck.sh` | The third opinion, from OpenSSL's own `ARGON2ID` KDF. |

### Running it

```sh
cd tools/oracles/crypto/argon2id
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

```sh
./openssl-xcheck.sh                                       # needs OpenSSL >= 3.2
./openssl-xcheck.sh /opt/homebrew/opt/openssl@3/bin/openssl
```

macOS ships **LibreSSL** as `openssl`, which has no `ARGON2ID` KDF; the script
detects that and exits 2 (skip) rather than reporting a pass. It also refuses to
exit 0 having run zero cases — a harness that runs nothing must not read as green.

### Deriving a new vector

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
