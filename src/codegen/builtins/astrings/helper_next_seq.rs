//! `__astrings_nextSeq` — shared private helper for the `astrings` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"REM The next insertion sequence: one past the maximum existing seq (0 when empty).
FUNC __astrings_nextSeq(spans AS List OF AttrSpan) AS Integer
  MUT maxSeq AS Integer = -1
  FOR EACH s IN spans
    IF s.seq > maxSeq THEN
      maxSeq = s.seq
    END IF
  NEXT
  RETURN maxSeq + 1
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("astrings_nextSeq", BODY));
}
