//! A call whose result type holds a resource only through a record field or a
//! union variant still needs that resource's close op defined.
//!
//! When nothing takes a call's result, codegen closes every resource the value
//! holds (`emit_for_each_resource` walks records and unions). The object plan's
//! `produced_resource_closes` only walked `RES`/`List`/`Map`/`Result` wrappers,
//! so a program that returned a union with a `RES fs::File` field and never
//! called `fs` itself failed to build with "internal relocation target
//! '_mfb_rt_fs_fs_close' is not defined".

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

const SRC: &str = "\
IMPORT io
IMPORT fs

TYPE Opened
  f AS RES fs::File
END TYPE

TYPE Missing
  why AS String
END TYPE

UNION Outcome
  Opened
  Missing
END UNION

FUNC pick(p AS String) AS Outcome
  RETURN Missing[p]
END FUNC

FUNC main AS Integer
  LET o AS Outcome = pick(\"x\")
  MATCH o
    CASE Missing(m)
      io::print(\"missing \" & m.why)
    CASE Opened(x)
      io::print(\"opened\")
  END MATCH
  RETURN 0
END FUNC
";

#[test]
fn a_resource_reached_through_a_returned_union_record_gets_its_close_defined() {
    for target in CodeTarget::ALL {
        let plan = try_code_for_src(SRC, target, Console)
            .unwrap_or_else(|err| panic!("{} refused the program: {err}", target.name()));
        assert!(
            plan.functions
                .iter()
                .any(|function| function.symbol.ends_with("mfb_rt_fs_fs_close")),
            "{}: the fs close helper the union's drop calls must be defined",
            target.name()
        );
    }
}
