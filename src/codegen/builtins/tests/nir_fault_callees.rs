//! A call is made THROUGH a local or global only when that binding holds a
//! function value (`engine/value/builder_values/lower_call.rs`). A call whose
//! target names a binding of any other type — an `Integer` local, an `Integer`
//! global — is not an indirect call through that integer (a jump to address 5);
//! it stays a direct call to the named symbol, which defines nothing, and the
//! module is refused at its relocation check. The front end rejects such a call
//! (`SYMBOL_NOT_CALLABLE`), so only a corrupted NIR — a call target renamed to the
//! binding — reaches it. The plain call and the trapped (`CallResult`) one each check
//! the binding's type.

use super::nir_fault::walk_ops_mut;
use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::NirValue;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io

MUT total AS Integer = 1

FUNC twice(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC main() AS Integer
  LET n AS Integer = 5
  LET r AS Integer = twice(n)
  LET q AS Integer = twice(r) TRAP(e)
    RETURN 1
  END TRAP
  io::print(toString(q))
  total = total + 1
  RETURN 0
END FUNC
";

/// What `target` says of the probe with one of its calls of `twice` — the plain
/// one, or the trapped one when `trapped` — retargeted at `binding`. Lowered
/// unoptimized, so the call is not inlined away first.
fn refusal(target: CodeTarget, binding: &str, trapped: bool) -> String {
    let mut module = with_opt_level(OptLevel(0), || nir_for_src(SOURCE, target, Console))
        .expect("the probe lowers to NIR");
    let mut renamed = 0usize;
    for function in &mut module.functions {
        walk_ops_mut(&mut function.body, &mut |_| {}, &mut |value| match value {
            NirValue::Call { target, .. } if !trapped && target.ends_with("twice") => {
                *target = binding.to_string();
                renamed += 1;
            }
            NirValue::CallResult { target, .. } if trapped && target.ends_with("twice") => {
                *target = binding.to_string();
                renamed += 1;
            }
            _ => {}
        });
    }
    assert_eq!(renamed, 1, "{}: the probe has one such call", target.name());
    code_for_nir(&module, target)
        .err()
        .unwrap_or_else(|| panic!("{}: a call through `{binding}` lowered", target.name()))
}

#[test]
fn a_call_naming_a_non_function_binding_stays_a_direct_call() {
    for target in [CodeTarget::LinuxX86_64, CodeTarget::MacosAarch64] {
        for binding in ["n", "total"] {
            assert_eq!(
                refusal(target, binding, false),
                format!("native code internal relocation target '{binding}' is not defined"),
                "{}",
                target.name()
            );
        }
    }
}

/// The trapped form needs the callee's return type to build its `Result`, and a
/// non-function binding names no function that has one: refused by name.
#[test]
fn a_trapped_call_naming_a_non_function_binding_has_no_return_type() {
    for target in [CodeTarget::LinuxX86_64, CodeTarget::MacosAarch64] {
        for binding in ["n", "total"] {
            let error = refusal(target, binding, true);
            assert!(
                error.starts_with(&format!(
                    "native raw result call '{binding}' has no return type"
                )),
                "{}: {error}",
                target.name()
            );
        }
    }
}
