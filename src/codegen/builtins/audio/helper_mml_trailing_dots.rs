//! `__audio_mmlTrailingDots` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Count trailing '.' characters starting at fromIdx; -1 if any non-dot appears.
FUNC __audio_mmlTrailingDots(token AS String, fromIdx AS Integer) AS Integer
  MUT count AS Integer = 0
  MUT i AS Integer = fromIdx
  WHILE i < len(token)
    IF strings::mid(token, i, 1) <> "." THEN
      RETURN -1
    END IF
    count = count + 1
    i = i + 1
  END WHILE
  RETURN count
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlTrailingDots", BODY));
}
