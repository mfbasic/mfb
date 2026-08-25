//! `__datetime_weekdayName` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_weekdayName(index AS Integer, full AS Boolean) AS String
  MUT names AS List OF String = []
  IF full THEN
    names = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
  ELSE
    names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
  END IF
  RETURN collections::get(names, index)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_weekdayName", BODY));
}
