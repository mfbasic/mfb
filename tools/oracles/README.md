# tools/oracles

Independent reference implementations that expected values are checked against, one
directory per area. An oracle lives here, with its own README, when it is not a Rust
dev-dependency (those live in `tests/`; see `tests/interop/rt_crypto_*_interop.rs`).

- **crypto/** — reference harnesses for the `crypto` builtin (Argon2id, hashes, keys,
  MAC/KDF). See `crypto/README.md` for how each is run and what it pins.

The rule for where an oracle goes is in `.ai/testing-gates.md` (the oracle-location
table).
