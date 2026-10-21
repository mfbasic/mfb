//! The program entry stub refuses an entry return type it has no exit for
//! (`engine/function/entry/mod.rs`, bug-175 I).
//!
//! The stub range-checks a non-`Nothing` result and branches to the
//! `entry_exit_range_error` handler, which it emits only for an `Integer` entry; a
//! third return type would branch to a label that does not exist. The front end
//! (`validate_entry_point`) admits only `Integer` and `Nothing`, so the module
//! here is one whose entry was rewritten after it: the refusal is a named
//! plan-level error on every backend, not a broken branch.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};
use crate::types::ParameterType;

const SOURCE: &str = "\
IMPORT io

FUNC main() AS Integer
  io::print(\"x\")
  RETURN 0
END FUNC
";

#[test]
fn an_entry_returning_anything_but_integer_or_nothing_is_refused() {
    for target in CodeTarget::ALL {
        let mut module = nir_for_src(SOURCE, target, Console).expect("lowers to NIR");
        let entry = module.entry.as_mut().expect("the program has an entry");
        entry.returns = ParameterType::String;
        let Err(error) = code_for_nir(&module, target) else {
            panic!("{}: a String entry lowered", target.name());
        };
        assert!(
            error
                .contains("program entry return type must be 'Integer' or 'Nothing', got 'String'"),
            "{}: {error}",
            target.name()
        );
    }
}
