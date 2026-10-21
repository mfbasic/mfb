//! A resource UNION the module drops without ever binding it still dispatches to
//! its variants' close ops, so every built-in variant's close helper must be
//! defined.
//!
//! `runtime_symbols` pulls a resource union's variant closes in when the module
//! BINDS that union (`NirOp::Bind`). Two other shapes drop one with no `Bind` of
//! the union type anywhere:
//!
//! ```text
//! streams = collections::append(streams, openStream())   // openStream() AS Stream
//! TYPE Holder / stream AS RES Stream                      // the owner's drop reaches it
//! ```
//!
//! The first appends the union a call produced and never binds it; the second only
//! ever names it as a record field. Each referenced `_mfb_rt_tcp_tcp_close` from
//! the drop while nothing defined it, and the build died with `internal relocation
//! target '_mfb_rt_tcp_tcp_close' is not defined` on valid source. A `tcp` import
//! that is never called is what makes the helper's absence visible: the program
//! has no other reason to emit it.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

const APPENDED_UNION_CALL: &str = "\
IMPORT collections
IMPORT fs
IMPORT io
IMPORT tcp

UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC openStream() AS Stream
  RES f AS fs::File = fs::createTempFile()
  RETURN f
END FUNC

FUNC main() AS Integer
  MUT streams AS List OF RES Stream = []
  streams = collections::append(streams, openStream())
  io::print(toString(len(streams)))
  RETURN 0
END FUNC
";

const UNION_ONLY_AS_A_FIELD: &str = "\
IMPORT fs
IMPORT io
IMPORT tcp

UNION Stream
  fs::File
  tcp::Socket
END UNION

TYPE Holder
  stream AS RES Stream
  n AS Integer
END TYPE

FUNC open(n AS Integer) AS Holder
  RES f AS fs::File = fs::createTempFile()
  RETURN Holder[stream := f, n := n]
END FUNC

FUNC main() AS Integer
  LET h AS Holder = open(1)
  io::print(toString(h.n))
  RETURN 0
END FUNC
";

#[test]
fn a_resource_union_dropped_without_a_bind_defines_every_variant_close() {
    for (what, source) in [
        ("an appended call result", APPENDED_UNION_CALL),
        ("a record field", UNION_ONLY_AS_A_FIELD),
    ] {
        for target in CodeTarget::ALL {
            check(what, source, target);
        }
    }
}

fn check(what: &str, source: &str, target: CodeTarget) {
    let plan = try_code_for_src(source, target, Console).unwrap_or_else(|err| {
        panic!(
            "a resource union reached only as {what} must build on {}: {err}",
            target.name()
        )
    });
    for variant in ["fs", "tcp"] {
        let close = format!("_mfb_rt_{variant}_{variant}_close");
        assert!(
            plan.functions.iter().any(|f| f.symbol.contains(&close)),
            "{} ({what}): the union's drop dispatches to `{close}`, so it must be defined",
            target.name()
        );
    }
}
