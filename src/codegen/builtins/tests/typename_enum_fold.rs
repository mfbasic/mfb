//! `typeName(Enum.Member)`, `typeName(callable(…))` and `typeName(e.code)` fold the
//! same way inside a concatenation as on their own.
//!
//! `typeName` of an enum member names an enum TYPE, not a binding: the member's
//! target is the type's spelling, so the operand walk has no local to type. The
//! builder resolves it (bug-874, `static_type_name_for_fold`), but the data-object
//! pre-pass that interns every folded string (`static_type_name_for_fold_with_types`)
//! did not, so `"x" & typeName(Colour.Red)` folded to the literal `xColour` that
//! nothing had interned and the build died with `string literal 'xColour' has no
//! data object` on valid source.
//!
//! The same pre-pass lacked the callable-local arm (bug-569/672): `typeName(f(1))`
//! where `f` is a `FUNC` local folds to the function type's return in the builder
//! but was untyped in the pre-pass, and likewise `typeName(pick(x))` for a module
//! function whose return is not a builtin type (`"w" & typeName(pick(FALSE))`).
//!
//! And the compiler's own `Error` record, which no declaration in the module
//! describes: `"c" & typeName(e.code)` inside a `TRAP` handler folded to `cInteger`
//! with nothing interning it, because the pre-pass's field table did not know
//! `Error`'s fields.
//!
//! And a stateful resource: `typeName(h.state)` and `typeName(h.state.pos)` named
//! no type at all ("cannot determine typeName argument type") because the fold's
//! member lookup knew record fields but not `.state`.
//!
//! The assertion is on the interned data object itself: the folded literal must be
//! in the plan, spelled exactly as the type is.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

const ENUM_IN_CONCAT: &str = "\
IMPORT fs
IMPORT io

ENUM Colour
  Red, Green
END ENUM

TYPE Dot
  x AS Integer
END TYPE

TYPE Tag
  name AS String
END TYPE

UNION Shape
  Dot
  Tag
END UNION

FUNC twice(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC pick(flag AS Boolean) AS Shape
  IF flag THEN
    RETURN Dot[x := 1]
  END IF
  RETURN Tag[name := \"t\"]
END FUNC

TYPE Cursor
  pos AS Integer
END TYPE

FUNC risky(n AS Integer) AS Integer
  IF n < 0 THEN
    FAIL error(1, \"negative\")
  END IF
  RETURN n
END FUNC

FUNC main() AS Integer
  LET caught AS Integer = risky(-1) TRAP(e)
    io::print(\"c\" & typeName(e.code))
    io::print(\"m\" & typeName(e.message))
    io::print(\"s\" & typeName(e.source.line))
    RECOVER 0
  END TRAP
  RES h AS fs::File STATE Cursor = fs::createTempFile()
  io::print(\"p\" & typeName(h.state.pos))
  io::print(\"q\" & typeName(h.state))
  LET f AS FUNC(Integer) AS Integer = twice
  LET base AS Integer = 2
  LET g AS FUNC(Integer) AS Integer = LAMBDA(v AS Integer) -> v + base
  io::print(\"y\" & typeName(f(1)))
  io::print(\"z\" & typeName(g(1)))
  io::print(\"w\" & typeName(pick(FALSE)))
  LET a AS Integer = 1
  io::print(typeName(a) & typeName(Colour.Red))
  io::print(\"x\" & typeName(Colour.Green))
  RETURN 0
END FUNC
";

#[test]
fn a_folded_type_name_in_a_concatenation_is_interned() {
    let plan = code_for_src_cached(ENUM_IN_CONCAT, CodeTarget::LinuxX86_64, Console);
    for folded in [
        "IntegerColour",
        "xColour",
        "yInteger",
        "zInteger",
        "wShape",
        "cInteger",
        "mString",
        "sInteger",
        "pInteger",
        "qCursor",
    ] {
        assert!(
            plan.data_objects
                .iter()
                .any(|object| object.value.contains(folded)),
            "a `typeName(...)` inside a concatenation folds to `{folded}`, and the \
             pre-pass must intern that literal; the data objects are {:?}",
            plan.data_objects
                .iter()
                .map(|object| object.value.as_str())
                .collect::<Vec<_>>()
        );
    }
}
