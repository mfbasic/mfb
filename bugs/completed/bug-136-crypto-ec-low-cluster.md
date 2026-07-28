# bug-136 — crypto EC LOW cluster: generate() private-key scratch not wiped; openssl verify skips result checks; SEC1 scalar splice assumes long-form DER

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9). Three independent LOW /
latent findings in the ECDSA codegen, batched per goal-02.

## 1. openssl generate() leaves private-key bytes unwiped in the RAWBUF arena scratch

`src/target/shared/code/crypto_ec/openssl.rs:736-798` (`generate`: RAWBUF filled
with point‖scalar at :747-768, only SEC1PTR wiped at :798). openssl `generate`
wipes the SEC1 DER scratch but not RAWBUF, which holds the identical
`0x04‖X‖Y‖K` private material after the output list is built — violating the
module's stated zeroization rule ("a later same-program arena allocation cannot
be handed a block still holding key bytes"). The returned list intentionally
holds the key, so this is only the stray scratch copy; an info-leak within the
same process. Fix: `zero_guarded` RAWBUF at the end of `generate` like sign
already wipes its shared buffer (crypto_ec.rs:123-171).

## 2. openssl verify skips return checks on EVP_MD_CTX_new / EVP_DigestVerifyInit

`src/target/shared/code/crypto_ec/openssl.rs:1459-1464` (ctx not null-checked),
:1493 (init result unchecked; contrast sign's checked init at :1092-1096). On
EVP_MD_CTX_new returning NULL (OOM) the subsequent init derefs NULL (crash); on
init failure the verify proceeds and EVP_DigestVerify's failure is folded into
"signature invalid" (returns FALSE) instead of an error — a silent
internal-error-to-FALSE misreport. (macos.rs's CFNumberCreate NULL at generate
:513-523 is the same unchecked-allocation class.) Fix: null-check the ctx and
propagate the init result as an error, matching sign().

## 3. generate() SEC1 scalar splice assumes the long-form DER header (breaks P-384/P-521 if OpenSSL omits the public key)

`src/target/shared/code/crypto_ec/openssl.rs:59-121` (`sec1_scalar_off` 7/8/8),
:746-768 (scalar copy from fixed offset; no `SEC1LEN` lower-bound check before
the read). The module defensively takes the *point* from the SPKI when OpenSSL
omits the SEC1 publicKey field, but the *scalar* offset stays hardcoded to the
with-publicKey encoding. If publicKey is omitted, P-384/P-521 totals drop under
128 bytes, the DER header shrinks from `30 81 xx` to `30 xx`, and the scalar
sits at offset 7 not 8 — the emitted private key has a shifted, corrupt scalar
(and the fixed-length copy can read 1 byte past the i2d output, never validated
≥ off+field_len). P-256 is immune (offset 7 in both forms). Default builds
include the point; latent. Fix: parse the DER length prefix to locate the
scalar, and bounds-check SEC1LEN before the copy.

## Prior art

bug-55 covered error-path frees/wipes (present and test-pinned); these are the
non-error-path gaps it did not cover.
