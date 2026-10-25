# tools/oracles

Independent reference implementations that expected values are checked against, one
directory per area. An oracle lives here, with its own README, when it is not a Rust
dev-dependency (those live in `tests/`; see `tests/interop/rt_crypto_*_interop.rs`).

- **crypto/** — reference harnesses for the `crypto` builtin (Argon2id, hashes, keys,
  MAC/KDF). See `crypto/README.md` for how each is run and what it pins.
- **compress/** — the `compress` builtin against Python's and Node's zlib, one mode per
  operation. See `compress/README.md`.
- **url/** — `net::toUrl`, `toString(Url)`, `net::percentDecode`, `net::parseQuery`, the
  `encoding` percent/form codecs and `http` chunked de-framing against Node's WHATWG `URL`
  and llhttp. See `url/README.md`.
- **aarch64-decode/** — the AArch64 decoder `mfb dbg` disassembles with, against lldb's
  disassembly of dyld, the system libraries and MFBASIC programs. See
  `aarch64-decode/README.md`.
- **riscv64-decode/** — the riscv64 decoder `mfb dbg` disassembles and steps with,
  against GNU `objdump`'s disassembly of glibc, musl and MFBASIC programs. See
  `riscv64-decode/README.md`.

The rule for where an oracle goes is in `.ai/testing-gates.md` (the oracle-location
table).
