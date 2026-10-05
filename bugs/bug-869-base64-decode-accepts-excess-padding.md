# bug-869: `encoding::base64Decode` accepts excess `=` padding (`"AAAA===="`, `"AA======"`)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-error/encoding/func_encoding_base64Decode_* (add excess-padding cases)

`base64Decode` checks that the input length is a multiple of 4, that `=` appears only as
a trailing run, and that the symbol count mod 4 is not 1. It never checks that the
padding *length* matches the final group. A whole group of `====`, or six `=` after two
symbols, is accepted and silently decoded, so two different strings decode to the same
bytes and a non-canonical encoding passes validation. Python's strict decoder rejects both
("Excess padding not allowed").

Correct behavior: padding is 0, 1 or 2 `=` that complete the last 4-symbol group
(symbols mod 4 = 0 → 0 pad, 3 → 1, 2 → 2). Anything else raises `ErrInvalidFormat`
(77050003).

References:

- src/docs/spec/stdlib/08_encoding.md §Base-N: "Decoding validates the alphabet, that `=`
  appears only as a trailing run, the group length, and (Base64) that the total input
  length is a multiple of 4." and "decoders fail closed with `ErrInvalidFormat`
  (`77050003`) on malformed input"
- RFC 4648 §3.2/§4 (padding completes the final quantum)

## Failing Reproduction

`/tmp/hardening/B04/repro/small/src/main.mfb` (`b64` helper prints the decode or FAIL code):

```
  b64("AAAA====")
  b64("AA======")
  b64("AAA=====")
  b64("AAAA")
  b64("AA==")
```

- Observed:
  ```
  base64Decode(AAAA====) = [000000]
  base64Decode(AA======) = [00]
  base64Decode(AAA=====) = [0000]
  base64Decode(AAAA) = [000000]
  base64Decode(AA==) = [00]
  ```
- Expected: the first three `FAIL 77050003`; the last two as shown.

Contrast: all 20 round-trip vectors and 14 other malformed inputs match python `base64`
(`/tmp/hardening/B04/encx`, 458 encoding cases, the only base64 mismatch is this one).
`base32Decode("A=======")` etc. are rejected by its `tail = 1|3|6` check, but base32 has the
same gap for whole extra `========` groups (not separately probed).

## Root Cause

`src/codegen/builtins/encoding/func_base64_decode.rs:__encoding_base64Decode` validates
`total mod 4 = 0` and `symbols mod 4 <> 1`. `__encoding_base64Symbols` validates
trailing-run placement. Nothing compares `total - symbols` (the pad count) with
`(4 - symbols mod 4) mod 4`. Add that check. `base64UrlDecode` is deliberately
padding-tolerant per the spec and is out of scope.

## Goal

- `"AAAA===="`, `"AA======"`, `"AAA====="` → 77050003; canonical inputs unchanged.

### Non-goals (must NOT change)

- The documented non-canonical *final-bits* acceptance (`"AB=="` → one 0 byte, per `mfb man encoding`).

## Blast Radius

- `__encoding_base64Decode`: fixed. `__encoding_base32Decode`: same pad-count gap likely (whole extra pad group); fix together.
