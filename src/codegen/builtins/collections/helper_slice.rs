//! `__collections_slice` — shared private helper for the `collections` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-306 S4: this body is DEAD -- `slice` is natively lowered as a bulk range copy,
' and the native path is what clamps `start` into [0, len] and `stop` into
' [start, len]. This source does not clamp, so it is not a usable specification of
' the behaviour; do not read it as one.
FUNC __collections_slice OF T(value AS List OF T, start AS Integer, stop AS Integer) AS List OF T
  MUT result AS List OF T = []
  MUT i AS Integer = start
  WHILE i < stop
    result = collections::append(result, collections::get(value, i))
    i = i + 1
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("collections_slice", BODY));
}
