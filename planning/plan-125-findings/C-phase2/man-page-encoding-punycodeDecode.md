### 1. Not an inverse for all accepted strings
UNIT:      man-page:encoding/punycodeDecode
CLAIM:     "It is the inverse of `encoding::punycodeEncode`."
VERDICT:   wrong
EVIDENCE:  Probe compiled and ran with the specified release binary: `punycodeDecode(punycodeEncode("xn--mnchen-3ya.de"))` printed `münchen.de`, not `xn--mnchen-3ya.de`. `src/codegen/builtins/encoding/func_punycode_encode.rs:BODY` leaves all-ASCII labels unchanged; `func_punycode_decode.rs:BODY` then decodes any label beginning exactly `xn--`.
SUGGESTED: `punycodeDecode` reverses `punycodeEncode` for Unicode hostnames whose ASCII labels do not themselves begin with `xn--`.

### 2. Empty input behavior is omitted
UNIT:      man-page:encoding/punycodeDecode
CLAIM:     "The ASCII (Punycode) domain name to decode."
VERDICT:   incomplete
EVIDENCE:  The probe printed `empty=` for `encoding::punycodeDecode("")`. `src/codegen/builtins/encoding/func_punycode_decode.rs:__encoding_punycodeDecode` splits the input and returns the accumulated empty string; the parameter description does not state this empty-input behavior.
SUGGESTED: `The ASCII domain name to decode. The empty string decodes to the empty string.`

### 3. The documented malformed-input error is not exhaustive
UNIT:      man-page:encoding/punycodeDecode
CLAIM:     "Malformed input — a basic (pre-delimiter) byte at or above `128`, a variable-length integer that is truncated before it terminates or that would overflow, a byte that is not a valid base-36 digit, a decoded scalar value outside the Unicode range, or an encoded label longer than 1024 octets — raises `ErrInvalidFormat` rather than producing a partial result."
VERDICT:   wrong
EVIDENCE:  The probe’s RFC 3492 payload for surrogate `U+D800`, `encoding::punycodeDecode("xn--ib9b")`, printed `error: 77020004` (`ErrEncoding`), not `77050003` (`ErrInvalidFormat`). `src/codegen/builtins/encoding/helper_puny_decode_label.rs:__encoding_punyDecodeLabel` passes the surrogate to `__encoding_fromCodepoint`; its invalid UTF-8 byte sequence then fails in `toString(List OF Byte)`, whose `ErrEncoding` behavior is documented by `src/codegen/builtins/mod.rs`.
SUGGESTED: `The listed malformed Punycode forms raise ErrInvalidFormat. A payload that reconstructs a surrogate currently raises ErrEncoding while its result is converted to text.`

### 4. The stated 1024-octet bound includes the wrong portion of the label
UNIT:      man-page:encoding/punycodeDecode
CLAIM:     "Malformed input — a basic (pre-delimiter) byte at or above `128`, a variable-length integer that is truncated before it terminates or that would overflow, a byte that is not a valid base-36 digit, a decoded scalar value outside the Unicode range, or an encoded label longer than 1024 octets — raises `ErrInvalidFormat` rather than producing a partial result."
VERDICT:   wrong
EVIDENCE:  A valid full A-label of 1,028 octets (`xn--` plus a 1,024-octet payload) decoded successfully; the probe printed `payload-1024=ok-length: 1022`. A 1,029-octet full label with a 1,025-octet payload printed `payload-1025=error: 77050003`. `src/codegen/builtins/encoding/func_punycode_decode.rs:__encoding_punycodeDecode` strips `xn--` before calling `helper_puny_decode_label.rs:__encoding_punyDecodeLabel`, which checks `len(data) > 1024`.
SUGGESTED: `A Punycode payload longer than 1024 octets, excluding its xn-- prefix, raises ErrInvalidFormat.`

### 5. `punycodeEncode` can produce a label the decoder rejects
UNIT:      man-page:encoding/punycodeDecode
CLAIM:     "The length bound exists because RFC 3492's insertion is quadratic in the label's length; 1024 octets is sixteen times the 63-octet DNS label limit (RFC 1034 §3.1, RFC 5890 §2.3.1) and well past the RFC's own sample strings, so no host label or round trip through `punycodeEncode` of ordinary text can reach it."
VERDICT:   wrong
EVIDENCE:  The probe constructed a string of 1,023 ordinary `ü` characters. `punycodeEncode` produced an A-label of length `1029`, then `punycodeDecode` printed `encode-round-trip=error: 77050003`. `src/codegen/builtins/encoding/func_punycode_encode.rs:__encoding_punycodeEncode` has no corresponding label-length cap; `helper_puny_decode_label.rs:__encoding_punyDecodeLabel` rejects payloads over 1,024 octets.
SUGGESTED: `The bound limits decoder work. It is above DNS host-label limits, but punycodeEncode accepts unrestricted strings and can produce a label that this decoder rejects for length.`