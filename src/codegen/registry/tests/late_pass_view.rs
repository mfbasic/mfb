//! `ProjectView`: what the late passes see of a program, including its `TESTING` block.
//!
//! A late-pass package (`http`, `net`, `encoding`) injects its source companion — and a
//! gated helper only when the program calls or names what the helper serves. The view
//! that answers "does the program call X" is read off the elaborated HIR the build
//! produces (before monomorphization), and a
//! `TESTING` block is part of that HIR: `mfb test` must see a callee that only a test
//! case uses, or the helper it needs would not be injected and the test would fail to
//! build. The block's groups are scanned for the callees and the annotated types they
//! spell exactly like function bodies.

use super::*;

const TESTED: &str = "\
IMPORT strings

FUNC main() AS Integer
  RETURN 0
END FUNC

TESTING
  TGROUP \"upper\"
    TCASE \"shouts\"
      LET words AS List OF String = [\"a\"]
      expectString(strings::upper(\"x\"), \"X\")
    END TCASE
  END TGROUP
END TESTING
";

const UNTESTED: &str = "\
IMPORT strings

FUNC main() AS Integer
  RETURN 0
END FUNC
";

#[test]
fn a_callee_used_only_in_a_testing_block_is_seen() {
    let tested = ProjectView::of_hir(&crate::hir::elaborate(&crate::testutil::project_from_src(
        TESTED,
    )));
    assert!(
        tested.references_any(&["upper"]),
        "a call inside a TCASE is a call the program makes under `mfb test`"
    );
    assert!(
        tested.names_any(&["List", "String"]),
        "a type spelled in a TCASE annotation is a type the program names"
    );
    assert!(tested.imports("strings"));
    let untested = ProjectView::of_hir(&crate::hir::elaborate(&crate::testutil::project_from_src(
        UNTESTED,
    )));
    assert!(
        !untested.references_any(&["upper"]),
        "without the TESTING block the program does not call it"
    );
}
