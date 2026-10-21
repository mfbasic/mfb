//! A `CONTINUE` or `EXIT` whose target names no enclosing loop is refused by name
//! (`engine/control/builder_control/statement_dispatch.rs`), on every backend, rather
//! than branching to a label that does not exist. The front end resolves the target
//! first, so only a corrupted NIR (a loop label renamed after resolution) reaches it.

use super::nir_fault::walk_ops_mut;
use crate::target::shared::nir::NirOp;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io

FUNC main() AS Integer
  FOR i = 1 TO 3
    IF i = 2 THEN
      CONTINUE FOR
    END IF
    IF i = 3 THEN
      EXIT FOR
    END IF
    io::print(toString(i))
  NEXT
  RETURN 0
END FUNC
";

/// Retarget every `CONTINUE` (or every `EXIT`) of the probe to a loop that is not
/// there, and return what each backend says.
fn refusals(continues: bool) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for target in CodeTarget::ALL {
        let mut module = nir_for_src(SOURCE, target, Console).expect("the probe lowers to NIR");
        let mut retargeted = 0usize;
        for function in &mut module.functions {
            walk_ops_mut(
                &mut function.body,
                &mut |op| match op {
                    NirOp::ContinueLoop { target, .. } if continues => {
                        *target = Some("__no_such_loop__".to_string());
                        retargeted += 1;
                    }
                    NirOp::ExitLoop { target, .. } if !continues => {
                        *target = Some("__no_such_loop__".to_string());
                        retargeted += 1;
                    }
                    _ => {}
                },
                &mut |_| {},
            );
        }
        assert_eq!(
            retargeted,
            1,
            "{}: the probe has one such statement",
            target.name()
        );
        match code_for_nir(&module, target) {
            Ok(_) => panic!("{}: a retargeted loop exit was lowered", target.name()),
            Err(error) => out.push((target.name(), error)),
        }
    }
    out
}

#[test]
fn a_continue_naming_no_loop_is_refused_on_every_backend() {
    for (target, error) in refusals(true) {
        assert!(
            error.contains("native code CONTINUE has no matching loop"),
            "{target}: {error}"
        );
    }
}

#[test]
fn an_exit_naming_no_loop_is_refused_on_every_backend() {
    for (target, error) in refusals(false) {
        assert!(
            error.contains("native code EXIT has no matching loop"),
            "{target}: {error}"
        );
    }
}
