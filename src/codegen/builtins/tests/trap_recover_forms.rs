//! bug-1026 at the `RECOVER` store (`engine/control/builder_control/assign_op.rs`):
//! an inline `TRAP` binding a resource union, whose handler recovers with ANOTHER
//! binding's union, lends the trap temp that binding's record and hands it a copy
//! of the box — the box it names belongs to `other`, and the temp's drop frees
//! only its own copy. A recover with a fresh union (a call's result) owns its
//! box already and copies nothing. Observed through the `inline_value_*` scratch
//! of the box copy.

use super::lowered::slots_on;
use crate::testutil::CodeTarget;

fn program(recover: &str) -> String {
    format!(
        "IMPORT fs
IMPORT tcp
IMPORT io

UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC openStream(p AS String) AS RES Stream
  RES f AS fs::File = fs::openFile(p)
  RETURN f
END FUNC

FUNC main() AS Integer
  RES other AS Stream = fs::createTempFile()
  RES s AS Stream = openStream(\"/nonexistent\") TRAP(e)
    RECOVER {recover}
  END TRAP
  io::print(\"x\")
  RETURN 0
END FUNC
"
    )
}

fn copies_box(recover: &str, target: CodeTarget) -> bool {
    slots_on(&program(recover), "main", target)
        .iter()
        .any(|slot| slot == "inline_value_source")
}

#[test]
fn recovering_with_another_bindings_union_copies_its_box() {
    for target in CodeTarget::ALL {
        assert!(copies_box("other", target), "{}", target.name());
        assert!(
            !copies_box("openStream(\"/tmp/x\")", target),
            "{}",
            target.name()
        );
    }
}
