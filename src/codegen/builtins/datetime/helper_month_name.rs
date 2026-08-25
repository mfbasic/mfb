//! `__datetime_monthName` — shared private helper for the `datetime` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_monthName(month AS Integer, full AS Boolean) AS String
  MUT names AS List OF String = []
  IF full THEN
    names = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
  ELSE
    names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
  END IF
  RETURN collections::get(names, month - 1)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("datetime_monthName", BODY));
}
