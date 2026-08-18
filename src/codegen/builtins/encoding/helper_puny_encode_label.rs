//! `__encoding_punyEncodeLabel` — shared private helper for the `encoding` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Encode one label's code points to Punycode (without the "xn--" prefix).
FUNC __encoding_punyEncodeLabel(points AS List OF Integer) AS String
  LET total AS Integer = len(points)
  MUT out AS String = ""
  MUT handled AS Integer = 0
  FOR EACH cp IN points
    IF cp < 128 THEN
      out = out & __encoding_byteChar(cp)
      handled = handled + 1
    END IF
  NEXT
  MUT basicLength AS Integer = handled
  IF basicLength > 0 THEN
    out = out & "-"
  END IF
  MUT n AS Integer = 128
  MUT delta AS Integer = 0
  MUT bias AS Integer = 72
  MUT minCp AS Integer = 0
  MUT q AS Integer = 0
  MUT k AS Integer = 0
  MUT threshold AS Integer = 0
  MUT digit AS Integer = 0
  WHILE handled < total
    minCp = 1114112
    FOR EACH cp IN points
      IF cp >= n AND cp < minCp THEN
        minCp = cp
      END IF
    NEXT
    delta = delta + (minCp - n) * (handled + 1)
    n = minCp
    FOR EACH cp IN points
      IF cp < n THEN
        delta = delta + 1
      END IF
      IF cp = n THEN
        q = delta
        k = 36
        MUT emitting AS Boolean = TRUE
        WHILE emitting
          threshold = __encoding_punyThreshold(k, bias)
          IF q < threshold THEN
            out = out & __encoding_byteChar(__encoding_punyDigit(q))
            emitting = FALSE
          ELSE
            digit = threshold + (q - threshold) - ((q - threshold) / (36 - threshold)) * (36 - threshold)
            out = out & __encoding_byteChar(__encoding_punyDigit(digit))
            q = (q - threshold) / (36 - threshold)
            k = k + 36
          END IF
        END WHILE
        bias = __encoding_punyAdapt(delta, handled + 1, handled = basicLength)
        delta = 0
        handled = handled + 1
      END IF
    NEXT
    delta = delta + 1
    n = n + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("encoding_punyEncodeLabel", BODY));
}
