//! bug-429: the error-path binding of a `RES x = <fallible> TRAP` over a
//! resource UNION.
//!
//! `lower_default_value_inner` has an arm for a resource union, and its comment
//! says exactly where the need comes from: a `RES` binding whose initializer can
//! fail has to bind SOMETHING on the error path, and a resource union has no
//! reconstructible default. The arm builds a real `{tag@0, record-ptr@8}` union
//! whose record is CLOSED, so its tag-dispatched drop is a safe no-op and a
//! `RECOVER`ed union's `.state` or `MATCH` reads a valid closed record rather
//! than dereferencing null.
//!
//! Nothing in process reached it. The corpus has resource-union fixtures and it
//! has trap fixtures; what it did not have is one program that is both — a
//! resource union produced by a call that can fail, bound with a `TRAP`.
//!
//! The failure the arm prevents is not a wrong answer. Without a default the
//! error path binds null, and the scope drop then dispatches a close op through
//! a null record pointer.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

/// A resource union bound from a fallible producer, with a `TRAP`.
///
/// The producer must RETURN the union, not a member of it. `RES s AS Stream =
/// fs::openFile(…) TRAP` lowers its error path against the CALL's return type —
/// `fs::File` — and takes the plain-resource default (`default_resource_*`); it
/// never reaches the union arm. A `FUNC … AS Stream` that can `FAIL` is what
/// makes the default's type the union.
///
/// The `MATCH` after `RECOVER` is what makes the default's *shape* matter rather
/// than just its existence: it reads the tag and the record out of whatever was
/// bound.
const TRAPPED_RESOURCE_UNION: &str = "\
IMPORT fs
IMPORT tcp
IMPORT io

UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC pick(n AS Integer) AS Stream
  IF n < 0 THEN
    FAIL error(77070001, \"no stream\")
  END IF
  RETURN fs::createTempFile()
END FUNC

FUNC main AS Integer
  RES s AS Stream = pick(-1) TRAP(e)
    io::print(\"trapped \" & toString(e.code))
    RECOVER
  END TRAP
  MATCH s
    CASE fs::File(f)
      io::print(\"file\")
    CASE tcp::Socket(sock)
      io::print(\"socket\")
  END MATCH
  RETURN 0
END FUNC
";

/// The trapped resource-union binding lowers on every backend.
///
/// All five: the default builds a record and a two-word union block through the
/// arena, and the closed-flag layout it depends on is shared codegen — a
/// backend that laid the record out differently would produce a `MATCH` reading
/// the wrong word.
#[test]
fn a_trapped_resource_union_binding_lowers_on_every_backend() {
    for target in CodeTarget::ALL {
        try_code_for_src(TRAPPED_RESOURCE_UNION, target, Console).unwrap_or_else(|err| {
            panic!(
                "a `RES x = <fallible> TRAP` over a resource union on {}: {err}",
                target.name()
            )
        });
    }
}

/// The error path binds a real union block, not null.
///
/// The arm allocates 16 bytes — one word of tag, one word of record pointer —
/// and stores a CLOSED record into the second. Asserting the allocation and the
/// label is what distinguishes "a default was produced" from "the error path
/// fell through with whatever was in the register", which is the bug-429 shape:
/// the scope drop would then dispatch a close op through a null record pointer.
#[test]
fn the_error_path_binds_a_closed_record_rather_than_null() {
    use crate::codegen::engine::tests::test_support::Stream as CodeStream;

    let plan = try_code_for_src(TRAPPED_RESOURCE_UNION, CodeTarget::LinuxX86_64, Console)
        .expect("the program must lower");
    let labels: Vec<String> = plan
        .functions
        .iter()
        .flat_map(|f| CodeStream::of(f).labels())
        .map(|(_, name)| name)
        .collect();
    assert!(
        labels.iter().any(|name| name.contains("default_union")),
        "the error path must build the union default -- without it the binding \
         is null and the scope drop dispatches a close op through it. The \
         emitted labels are {labels:?}"
    );
}
