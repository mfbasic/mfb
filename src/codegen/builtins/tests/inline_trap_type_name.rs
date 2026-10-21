//! An inline `TRAP` on `typeName` builds, and owns no runtime helper.
//!
//! `typeName` folds to a string constant at compile time, so
//! `_mfb_rt_general_typeName` is never emitted. The native plan knew that for the
//! plain `RuntimeCall` form (`function_builder.rs:lower_value`,
//! `symbols.rs:collect_runtime_symbols_from_value`) but registered the helper for
//! the `CallResult` an inline `TRAP` makes of the same call, so
//! `LET t = typeName(x) TRAP(e) ... END TRAP` failed the build with "native
//! object plan relocation target '_mfb_rt_general_typeName' is neither defined
//! nor imported" although the front end had accepted it (with the advisory
//! `TYPE_INLINE_TRAP_DEAD_HANDLER`).

use super::lowered::calls_on;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{native_plan_for_src, try_code_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io
IMPORT collections

FUNC main() AS Integer
  MUT xs AS List OF Integer = [1, 2]
  LET t AS String = typeName(xs) TRAP(e)
    RECOVER \"?\"
  END TRAP
  io::print(t)
  RETURN 0
END FUNC
";

#[test]
fn an_inline_trap_on_type_name_plans_and_lowers_without_a_helper() {
    for target in CodeTarget::ALL {
        let plan = native_plan_for_src(SOURCE, target, Console)
            .unwrap_or_else(|error| panic!("{}: {error}", target.name()));
        assert!(
            !plan
                .runtime_symbols
                .iter()
                .any(|symbol| symbol.contains("typeName")),
            "{}: the plan defines a typeName helper: {:?}",
            target.name(),
            plan.runtime_symbols
        );
        for function in &plan.functions {
            assert!(
                !function
                    .calls
                    .iter()
                    .any(|call| call.symbol.contains("typeName")),
                "{}: `{}` plans a call to a typeName symbol, which nothing defines",
                target.name(),
                function.name
            );
        }
        if let Err(error) = try_code_for_src(SOURCE, target, Console) {
            panic!("{}: {error}", target.name());
        }
        let calls = calls_on(SOURCE, "main", target);
        assert!(
            !calls.iter().any(|call| call.contains("typeName")),
            "{}: typeName is a compile-time constant, not a call: {calls:?}",
            target.name()
        );
    }
}
