### 1. Not HTML form encoding
UNIT:      man-page:encoding/formUrlEncode
CLAIM:     "`encoding::formUrlEncode` encodes `text` using the `application/x-www-form-urlencoded` rules that HTML forms apply to query-string values."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/encoding/func_form_url_encode.rs:BODY` passes through only ASCII alphanumerics. The probe `formUrlEncode("*-._~")` printed `%2A%2D%2E%5F%7E`; the [WHATWG form encoding set](https://url.spec.whatwg.org/#application-x-www-form-urlencoded-percent-encode-set) leaves `*`, `-`, `.`, and `_` unescaped.
SUGGESTED: "`encoding::formUrlEncode` encodes text using MFBASIC’s form-url encoding: spaces become `+`, and every non-alphanumeric UTF-8 byte becomes `%XX`."