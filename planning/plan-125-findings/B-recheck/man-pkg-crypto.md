### 1. Overview under-reports which members take a String
UNIT:      man-pkg:crypto
PAGE:      package-wide (overview Description)
CATEGORY:  overview-mismatch
CLAIM:     "The cryptographic operations take and return List OF Byte; crypto::hash, crypto::hmac, and crypto::seal also accept String data, which they encode as UTF-8."
VERDICT:   wrong
EVIDENCE:  `grep -ln 'ParameterType::String' src/codegen/builtins/crypto/func_*.rs` lists func_encrypt.rs and func_shake256.rs alongside hash, hmac and seal; both rendered pages show a `data AS String` overload 2. Probe /tmp/plan-125-scratch/B-recheck/man-pkg-crypto/p1 (built with `mfb build p1`, run `./p1/build/p1.out`) printed `encrypt String ok len=50` and `shake256 String ok 483366601360a877`. argon2id with a String password fails to build (`TYPE_CALL_ARGUMENT_MISMATCH`, probe p2), so the list is the only place a developer learns which five take text.
SUGGESTED: "…crypto::hash, crypto::hmac, crypto::shake256, crypto::seal, and crypto::encrypt also accept String data, which they encode as UTF-8; every other member, including the password KDFs, takes bytes (strings::toBytes)."

### 2. Overview says randomInt returns an Integer; it also has a big::Int form
UNIT:      man-pkg:crypto
PAGE:      package-wide (overview Description)
CATEGORY:  overview-mismatch
CLAIM:     "The identifier helpers (uuid4, uuid7, ulid) return a String, and randomInt returns an Integer."
VERDICT:   misleading
EVIDENCE:  `mfb man crypto randomInt` lists overload 2 `crypto::randomInt(min AS big::Int, max AS big::Int) AS big::Int` and says it "needs IMPORT big alongside IMPORT crypto". Probe p1 printed `randomInt big ok 6`. A developer skimming the overview for "random number wider than 64 bits" is told the package returns only Integer.
SUGGESTED: "…and randomInt returns an Integer (or a big::Int when given big::Int bounds; see mfb man crypto randomInt)."

### 3. Sibling pages route password work to pbkdf2 alone, contradicting the overview's argon2id rule
UNIT:      man-pkg:crypto
PAGE:      hkdf, hash, shake256, seal (versus overview, argon2id, pbkdf2)
CATEGORY:  consistency
CLAIM:     hkdf: "it is not a password hash — stretch a low-entropy password with crypto::pbkdf2 instead"; hash: "Stretch a low-entropy password into key material with the deliberately slow, salted crypto::pbkdf2"; shake256: "for a keyed MAC or password stretching use crypto::hmac/crypto::pbkdf2"; seal: "or a KDF (crypto::hkdf / crypto::pbkdf2). Never seal directly under a password".
VERDICT:   inconsistent
EVIDENCE:  Overview: "use crypto::argon2id ... to store a password, and crypto::pbkdf2 for RFC 8018 / WPA2 compatibility or to stretch a passphrase into a key"; pbkdf2 page: "For storing passwords use crypto::argon2id". `grep -c argon2id` on func_hkdf.rs, func_hash.rs, func_shake256.rs, func_seal.rs, func_hmac.rs prints 0 for every file; `git log -- func_argon2id.rs | tail -1` shows argon2id landed 2026-09-06 (bug-515), after these sentences were written. A developer who enters the package via hash or hkdf (the common entry points) never hears of argon2id, and hash's "password hash" sentence names the wrong member for the overview's own rule.
SUGGESTED: On each of the four pages, make the password pointer match the overview: "…is not a password hash — store a password with crypto::argon2id, or stretch a passphrase into a key with crypto::pbkdf2." Add crypto::argon2id to the See also of hash and hkdf (it is auto-derived once named).

### 4. The overview never names hkdf, so the "I have a shared secret, now what?" question has no route
UNIT:      man-pkg:crypto
PAGE:      package-wide (overview Description)
CATEGORY:  discoverability
CLAIM:     The overview prose names argon2id, pbkdf2, encrypt, decrypt, exchange, convert, hash, hmac, seal, open, randomBytes and the identifier helpers, but hkdf appears only as a row in the alphabetical table.
VERDICT:   missing
EVIDENCE:  `grep -n hkdf src/codegen/builtins/crypto/mod.rs` matches no line of MODULE_DESC (the only hits are the function registration and module comments). Meanwhile the exchange page says "The raw secret is ... not a key: derive symmetric keys from it with crypto::hkdf", and the seal page says a key must come from "a key-agreement result, or a KDF" — the one KDF the overview's KDF paragraph does not mention is the one those two pages depend on.
SUGGESTED: Extend the KDF paragraph: "…to stretch a passphrase into a key. For input that is already high-entropy — a crypto::exchange secret, a master key — use crypto::hkdf, which is fast and is the wrong tool for a password."

### 5. No page says every primitive is one-shot
UNIT:      man-pkg:crypto
PAGE:      package-wide (overview; hash, hmac, shake256, seal)
CATEGORY:  coverage
CLAIM:     Nothing in the unit states that hashing, MACing and AEAD are whole-message calls with no incremental (update/finalize) or streaming interface.
VERDICT:   missing
EVIDENCE:  The rendered function table has no update/finish/stream member; `grep -n 'incremental\|streaming\|one-shot\|whole message' src/codegen/builtins/crypto/mod.rs src/codegen/builtins/crypto/func_hash.rs` prints nothing; `grep -rli 'crypto::hash\|digest' src/codegen/builtins/*/mod.rs | grep -v crypto` finds no file-hashing helper in another package. "How do I hash a large file?" is one of the first questions a developer brings to a crypto package, and the answer (read it fully into a List OF Byte; every member is one call over the whole message) appears nowhere.
SUGGESTED: One sentence in the overview's "take and return List OF Byte" paragraph: "Every member is one call over the whole message — there is no incremental or streaming form — so hash a file by reading it fully into a List OF Byte first (fs::readAll)." (Confirm the fs member name on the fs pages before citing it.)

What I checked and did not flag: the overview's software-first/platform-bound split matches generate, sign, verify and randomBytes; key sizes agree across the Certificate enum, the generate table, sign, verify, exchange, convert, encrypt and decrypt; the "curve is not checked" caveat is stated the same way on convert, encrypt, decrypt, exchange and sign; Sealed's "nonce is not part of this record" agrees with seal and open; SHA1 warning scoping agrees between the Hash enum, hash, hmac, hkdf and pbkdf2; the alphabetical table plus the overview's grouped prose makes each page findable except hkdf (finding 4). No network claims are involved in this unit.
