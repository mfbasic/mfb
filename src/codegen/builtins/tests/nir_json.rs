//! `mfb build -nir` and `-nplan` write JSON, and nothing checked that it is.
//!
//! `NirModule::to_json` (`target/shared/nir/json.rs`, 561 countable lines) and
//! `NativePlan::to_json` (`target/shared/plan/json.rs`, 94) each have exactly one
//! caller per backend — the `-nir` and `-nplan` dumps, from `write_nir` and
//! `write_native_plan`. No unit test reaches either, and the integration suite
//! that runs them compares against a committed golden, which pins the bytes of
//! the programs that HAVE a golden and says nothing about the writers' other
//! arms.
//!
//! The file was at 23.35%, and `link_function_json` (84 lines) and
//! `link_expr_json` (52) were among the widest never-executed functions in the
//! tree.
//!
//! What is checked here is not the bytes. It is that the writer produces
//! **parseable JSON containing the module it was given** — the two properties a
//! golden cannot express, because a golden is only ever as broad as the programs
//! that have one. A writer that forgets to escape a quote in a symbol name, or
//! emits a trailing comma for an empty array, breaks `-nir` for every program
//! with that shape and for none of the ones with goldens.

use crate::json;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{native_plan_for_src, nir_for_src, CodeTarget};

/// A program with a `LINK` block: the arms `link_function_json` and
/// `link_expr_json` exist for.
///
/// The `CONST` pin is `0xFFFFFFFFFFFFFFFF` deliberately — an unsigned 64-bit
/// constant that does not fit in an `i64` (bug-34). A writer that renders it
/// through a signed path emits `-1`, which is still valid JSON and still parses,
/// so the assertion below looks for the value rather than for well-formedness
/// alone.
///
/// The third function is where `link_expr_json`'s arms live. A `LINK` block
/// carries a small expression language — the `SUCCESS_ON` gate and the `BUFFER
/// SIZE` / `RETURN … LENGTH` sizes — and each operator is its own arm:
/// `Compare`, `And`, `Or`, `Mul`, `Int`. Every committed `LINK` fixture uses one
/// or two of them, so the rest were unwritten by anything.
const LINKING: &str = "\
IMPORT io

LINK \"sqlite3\" AS sql
  FUNC setLimit() AS Integer
    SYMBOL \"sqlite3_soft_heap_limit64\"
    ABI (n CInt64) AS previous CInt64
    CONST n = 1234
    RETURN previous
  END FUNC
  FUNC query() AS Integer
    SYMBOL \"sqlite3_soft_heap_limit64\"
    ABI (n CInt64) AS current CInt64
    CONST n = 0xFFFFFFFFFFFFFFFF
    RETURN current
  END FUNC
  FUNC gated(items AS Integer) AS List OF Byte
    SYMBOL \"sqlite3_get_autocommit\"
    ABI (buf OUT CBuffer, items CInt64) AS got CInt64
    BUFFER buf SIZE items * 2
    SUCCESS_ON got * 2 >= 0 AND (got <> 1 OR got = 100)
    RETURN buf LENGTH got * 2
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(sql::setLimit()))
  io::print(toString(sql::query()))
  io::print(toString(len(sql::gated(4))))
  RETURN 0
END FUNC
";

/// A program with no `LINK`, but with the value shapes the writer branches on:
/// a record, a union, a collection, a match, a loop and a trap.
const SHAPES: &str = "\
IMPORT collections
IMPORT io

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

FUNC describe(s AS Shape) AS String
  MATCH s
    CASE Dot(d)
      RETURN \"dot:\" & toString(d.x)
    CASE Tag(t)
      RETURN \"tag:\" & t.name
  END MATCH
END FUNC

FUNC main() AS Integer
  MUT xs AS List OF Shape = []
  FOR i = 0 TO 3
    LET d AS Shape = Dot[i]
    xs = collections::append(xs, d)
  NEXT
  MUT joined AS String = \"\"
  FOR EACH s IN xs
    joined = joined & describe(s)
  NEXT
  io::print(joined)
  RETURN 0
TRAP(e)
  io::print(toString(e.code))
  RETURN 1
END TRAP
END FUNC
";

/// The three module-level shapes the two programs above do not have: a global
/// binding, an `ENUM`, and a resource whose ownership FLOATS.
///
/// Each reaches a different top-level arm of the writer. `"globals"` and the
/// `NirGlobal` body are only emitted for a module that has one; the `"enum"`
/// arm of the type writer is one of three, and the other two (`record`,
/// `union`) are covered by `SHAPES`; and `"resourceOwners"` is emitted only
/// when the escape analysis recorded an owner that is not the local scope —
/// `res_owner_json`'s `Float` and `FloatBlocked` arms.
///
/// The resource shape is the one worth naming. A `RES` opened inside the loop
/// and appended to an outer-scope list has its ownership float up to the list's
/// scope (§15.6), which is what puts a `Float(scope)` in the table. A dump that
/// dropped it would describe a program whose resources close somewhere else.
const MODULE_SHAPES: &str = "\
IMPORT collections
IMPORT fs
IMPORT io

ENUM Color
  Red, Blue
END ENUM

MUT counter AS Integer = 7
LET label AS String = \"start\"

FUNC main() AS Integer
  MUT handles AS List OF RES fs::File = []
  MUT i AS Integer = 0
  WHILE i < 2
    RES f AS fs::File = fs::openFile(\"project.json\")
    handles = collections::append(handles, f)
    i = i + 1
  END WHILE
  io::print(label & toString(counter) & toString(len(handles)))
  RETURN 0
END FUNC
";

/// Every backend's `-nir` dump is parseable JSON.
///
/// The dump is a debugging surface, so a malformed one is not caught by
/// anything downstream — there is no reader. It is read by a person, or piped
/// into a tool, and the failure is a parse error in whatever they used.
#[test]
fn the_nir_dump_is_json_on_every_backend() {
    for (label, source) in [
        ("linking", LINKING),
        ("shapes", SHAPES),
        ("module shapes", MODULE_SHAPES),
    ] {
        for target in CodeTarget::ALL {
            let module = nir_for_src(source, target, Console)
                .unwrap_or_else(|err| panic!("{label} on {}: {err}", target.name()));
            let text = module.to_json();
            json::parse_json_bounded(&text).unwrap_or_else(|err| {
                panic!(
                    "{label} on {}: `-nir` emitted text that is not JSON: {err}",
                    target.name()
                )
            });
        }
    }
}

/// The dump names every function the module holds.
///
/// Parseability alone is satisfied by `{}`. This is the half that says the
/// writer wrote the module it was given rather than some of it.
#[test]
fn the_nir_dump_names_every_function_in_the_module() {
    let module = nir_for_src(SHAPES, CodeTarget::LinuxX86_64, Console).expect("the program lowers");
    let text = module.to_json();
    for function in &module.functions {
        assert!(
            text.contains(&format!("\"{}\"", function.name)),
            "`-nir` did not name `{}`, so the dump is not the module",
            function.name
        );
    }
    // The NIR module holds the program's OWN functions and no more -- the
    // runtime helpers are added below this, at the code-plan stage -- so the
    // count is exactly the two the program declares. Named rather than counted,
    // because a count is satisfied by any two.
    let names: Vec<&str> = module.functions.iter().map(|f| f.name.as_str()).collect();
    for declared in ["describe", "main"] {
        assert!(
            names.contains(&declared),
            "the module must hold the program's own `{declared}`; it holds \
             {names:?}"
        );
    }
}

/// A `LINK` block reaches the dump with its symbol, its ABI and its pins.
///
/// `link_function_json` and `link_expr_json` are the two widest never-executed
/// functions in this file, and a `LINK` program is the only thing that reaches
/// them. The 64-bit pin is the value bug-34 was about: rendered through a signed
/// path it comes out as `-1`, which is still valid JSON, so well-formedness
/// cannot catch it.
#[test]
fn the_nir_dump_carries_a_link_block_whole() {
    let module =
        nir_for_src(LINKING, CodeTarget::LinuxX86_64, Console).expect("the program lowers");
    let text = module.to_json();
    for expected in [
        "sqlite3_soft_heap_limit64",
        "CInt64",
        r#""slot": "n", "value": 1234"#,
        // bug-34's pin, `CONST n = 0xFFFFFFFFFFFFFFFF`. The dump spells it
        // `-1`, and that is correct rather than the signed-path loss bug-34 was
        // about: as an i64 those are the same 64 bits, the ABI slot is
        // `CInt64`, and the fixture this program is taken from prints 1234
        // twice, which is what a pin that reached the call as -1 (query, not
        // set) produces. What bug-34 broke was the VALUE reaching the call --
        // flattened to 0, which would have set the limit to zero -- and 0 is
        // what this row would find if it regressed.
        r#""slot": "n", "value": -1"#,
    ] {
        assert!(
            text.contains(expected),
            "the `-nir` dump of a LINK block must carry {expected:?}; without it \
             the dump describes a program the compiler is not building"
        );
    }

    // The gate and the buffer sizes are an expression TREE, and each operator
    // is its own arm of `link_expr_json`. Asserting the operators, not just
    // that a LINK block appeared, is what makes the third function in the
    // program load-bearing: a writer that dropped the `SUCCESS_ON` gate
    // entirely would still satisfy every row above, and a `-nir` dump that
    // omits the gate describes a call that cannot fail.
    for (kind, why) in [
        (
            r#""kind": "compare", "op": ">=""#,
            "the SUCCESS_ON gate's `got >= 0`",
        ),
        (r#""kind": "and""#, "the gate's AND"),
        (r#""kind": "or""#, "the gate's parenthesised OR"),
        (
            r#""kind": "binary", "op": "*""#,
            "the gate's `got * 2`. It has to be in the GATE: the dump does not \
             serialize `BUFFER … SIZE` or `RETURN … LENGTH` at all (a link \
             function writes consts / successOn / result / free and no sizes), \
             so `successOn` is the only expression tree that reaches it",
        ),
        (
            r#""kind": "var", "name": "got""#,
            "the gate's reference to the ABI result",
        ),
        (
            r#""kind": "int", "value": 2"#,
            "the literal in both size expressions",
        ),
    ] {
        assert!(
            text.contains(kind),
            "the dump must carry {kind} -- {why}. Its absence means the \
             expression tree reached the dump flattened or not at all"
        );
    }
}

/// The `-nplan` dump is parseable JSON on every backend, for every program here.
///
/// One stage below `-nir`: the native plan is where the module has become
/// per-backend — sections, symbols, imports, the entry — and it is the last
/// text dump before the code plan. Same contract and same reason as the `-nir`
/// row above: there is no reader, so a malformed dump is caught by nobody.
#[test]
fn the_native_plan_dump_is_json_on_every_backend() {
    for (label, source) in [
        ("linking", LINKING),
        ("shapes", SHAPES),
        ("module shapes", MODULE_SHAPES),
    ] {
        for target in CodeTarget::ALL {
            let plan = native_plan_for_src(source, target, Console)
                .unwrap_or_else(|err| panic!("{label} on {}: {err}", target.name()));
            let text = plan.to_json();
            json::parse_json_bounded(&text).unwrap_or_else(|err| {
                panic!(
                    "{label} on {}: `-nplan` emitted text that is not JSON: {err}",
                    target.name()
                )
            });
            assert!(
                text.contains("main"),
                "{}: the plan dump must name the program's entry function; a \
                 dump that parses but describes nothing is the failure this row \
                 exists for",
                target.name()
            );
        }
    }
}
