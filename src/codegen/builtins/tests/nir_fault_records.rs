//! A record `=` whose two operands are different records is refused by name
//! (`engine/operators/builder_numeric/comparison.rs`), on every backend, rather than
//! compared field by field against the left operand's layout. The type checker
//! never lets such a comparison through, so only a corrupted NIR — the right
//! operand renamed to a local of another record type — reaches it.

use super::nir_fault::walk_ops_mut;
use crate::operators::BinaryOp;
use crate::target::shared::nir::NirValue;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io

TYPE Dot
  x AS Integer
END TYPE

TYPE Tag
  y AS Integer
END TYPE

FUNC main() AS Integer
  LET a AS Dot = Dot[1]
  LET b AS Dot = Dot[2]
  LET c AS Tag = Tag[3]
  IF a = b THEN
    io::print(\"same\")
  END IF
  io::print(toString(c.y))
  RETURN 0
END FUNC
";

#[test]
fn a_comparison_of_two_different_records_is_refused_on_every_backend() {
    for target in CodeTarget::ALL {
        let mut module = nir_for_src(SOURCE, target, Console).expect("the probe lowers to NIR");
        let mut swapped = 0usize;
        for function in &mut module.functions {
            walk_ops_mut(&mut function.body, &mut |_| {}, &mut |value| {
                if let NirValue::Binary {
                    op: BinaryOp::Equal,
                    right,
                    ..
                } = value
                {
                    if matches!(right.as_ref(), NirValue::Local(name) if name == "b") {
                        **right = NirValue::Local("c".to_string());
                        swapped += 1;
                    }
                }
            });
        }
        assert_eq!(
            swapped,
            1,
            "{}: the probe has one record `=`",
            target.name()
        );
        let error = code_for_nir(&module, target)
            .err()
            .unwrap_or_else(|| panic!("{}: a mismatched record `=` lowered", target.name()));
        assert!(
            error.contains(
                "native code comparison requires matching record operands, got Dot and Tag"
            ),
            "{}: {error}",
            target.name()
        );
    }
}
