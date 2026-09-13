### 1. Byte-list overview excludes documented return types
UNIT:      man-pkg:crypto
PAGE:      package-wide
CATEGORY:  overview-mismatch
CLAIM:     “Inputs and outputs are List OF Byte”
VERDICT:   misleading
EVIDENCE:  `mfb man crypto uuid4` prints `crypto::uuid4() AS String`; `mfb man crypto randomInt` prints `crypto::randomInt(...) AS Integer`. The registry confirms these declarations in `src/codegen/builtins/crypto/func_uuid4.rs:register` and `src/codegen/builtins/crypto/func_random_int.rs:register`.
SUGGESTED: “The byte-oriented cryptographic functions use `List OF Byte`; hash, HMAC, and PBKDF2 also accept `String` input. Identifier helpers return `String`, and `randomInt` returns `Integer`.”

### 2. Overview omits public-key encryption and key agreement
UNIT:      man-pkg:crypto
PAGE:      package-wide
CATEGORY:  discoverability
CLAIM:     The overview’s capability list names “authenticated encryption (AEAD)” and “public-key signatures,” but never identifies the package’s public-key encryption, Diffie-Hellman key agreement, or key conversion facilities.
VERDICT:   missing
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man crypto` lists `crypto::encrypt`/`decrypt` as RFC 9180 HPKE operations, `crypto::exchange` as Diffie-Hellman, and `crypto::convert` as curve-key conversion; their registry entries are `src/codegen/builtins/crypto/func_encrypt.rs:register`, `func_decrypt.rs:register`, `func_exchange.rs:register`, and `func_convert.rs:register`. `MODULE_DESC` in `src/codegen/builtins/crypto/mod.rs:36` does not introduce any of those workflows.
SUGGESTED: Add: “For public-key encryption, use `crypto::encrypt` and `crypto::decrypt`; for X25519/X448 shared secrets, use `crypto::exchange`. `crypto::convert` converts an Ed25519 or Ed448 key pair to its matching key-agreement form.”