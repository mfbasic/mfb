//! bug-665: a module-level global passed as a call argument must keep the value
//! it had at the call, even when the callee reassigns that global.
//!
//! Arguments are borrowed: `clobber(g)` hands the callee a pointer to `g`'s
//! current block, and the callee's `g = …` lowers through `NirOp::StoreGlobal`,
//! which frees that block (the bug-47 old-block free). The parameter then reads
//! freed arena memory: a nonsense allocation size (`7-701-0001`), or silently
//! empty bytes for a `String`. bug-496's operand snapshot only covered an operand
//! followed by a LATER sibling's call; a lone `f(g)` has no later sibling.
//!
//! The fix (`src/codegen/engine/value/operand_snapshot.rs` +
//! `src/codegen/engine/value/store_reach.rs`) deep-copies such an argument into a
//! statement-scope temporary, but only when the call can actually reach a
//! `StoreGlobal` of that global. The controls pin the narrowness: a callee that
//! only reads the global, `len(g)`, and a higher-order builtin whose callback
//! cannot write it emit no copy at all.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, build_project, run_capture_with_env, temp_project};

/// The bug report's reproduction, verbatim.
const REPRO: &str = "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"alpha\", \"beta\", \"gamma\"]

SUB clobber(xs AS List OF String)
  g = collections::append(g, \"delta\")
  g = collections::append(g, \"epsilon\")
  MUT pad AS List OF String = [\"zzzzzzzzzzzzzzzzzzzzzzzzz\", \"yyyyyyyyyyyyyyyyyyyyyyyy\", \"xxxxxxxxxxxxxxxxxxxxxxx\"]
  io::print(\"param: \" & collections::get(xs, 0) & \" \" & collections::get(xs, 2) & \" len=\" & toString(len(xs)))
  io::print(toString(len(pad)))
END SUB

SUB main()
  clobber(g)
  io::print(\"g len=\" & toString(len(g)))
END SUB
";

/// The same callee reached through a `FUNC` value: the walk cannot name the
/// target, so it must fail closed and copy.
const FUNC_VALUE: &str = "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"alpha\", \"beta\", \"gamma\"]

SUB clobber(xs AS List OF String)
  g = collections::append(g, \"delta\")
  g = collections::append(g, \"epsilon\")
  MUT pad AS List OF String = [\"zzzzzzzzzzzzzzzzzzzzzzzzz\", \"yyyyyyyyyyyyyyyyyyyyyyyy\", \"xxxxxxxxxxxxxxxxxxxxxxx\"]
  io::print(\"param: \" & collections::get(xs, 0) & \" \" & collections::get(xs, 2) & \" len=\" & toString(len(xs)))
  io::print(toString(len(pad)))
END SUB

SUB main()
  LET f AS FUNC(List OF String) AS Nothing = clobber
  f(g)
  io::print(\"g len=\" & toString(len(g)))
END SUB
";

/// A `String` global: the freed block's length word is the arena's free-node
/// link, so the parameter silently reads as empty rather than crashing.
const STRING_GLOBAL: &str = "\
IMPORT io

MUT s AS String = \"the original global string value\"

SUB clobber(x AS String)
  s = \"a replacement that is also quite long\"
  LET pad AS String = \"0123456789012345678901234567890123456789\" & toString(len(s))
  io::print(\"param: \" & x & \" pad=\" & toString(len(pad)))
END SUB

SUB main()
  clobber(s)
  io::print(\"s=\" & s)
END SUB
";

/// Higher-order builtins walking `g` while their callback reassigns it: a named
/// `SUB` handed to `forEach`, and a `LAMBDA` handed to `filter` whose body calls
/// a function that writes `g`.
const HOF_CALLBACKS: &str = "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"alpha\", \"beta\", \"gamma\"]

SUB grow(v AS String)
  g = collections::append(g, v & \"!\")
  MUT pad AS List OF String = [\"zzzzzzzzzzzzzzzzzzzzzzzzz\", \"yyyyyyyyyyyyyyyyyyyyyyyy\", \"xxxxxxxxxxxxxxxxxxxxxxx\"]
  io::print(\"visit \" & v & \" pad=\" & toString(len(pad)))
END SUB

FUNC keep(v AS String) AS Boolean
  g = collections::append(g, v & \"?\")
  MUT pad AS List OF String = [\"zzzzzzzzzzzzzzzzzzzzzzzzz\", \"yyyyyyyyyyyyyyyyyyyyyyyy\", \"xxxxxxxxxxxxxxxxxxxxxxx\"]
  RETURN len(pad) = 3 AND v <> \"beta\"
END FUNC

SUB main()
  collections::forEach(g, grow)
  io::print(\"g len=\" & toString(len(g)))
  LET kept AS List OF String = collections::filter(g, LAMBDA(v AS String) -> keep(v))
  io::print(\"kept len=\" & toString(len(kept)) & \" g len=\" & toString(len(g)))
END SUB
";

/// A field of a global record: `r.items` is a pointer into `r`'s block, which
/// the callee's `r = …` frees.
const RECORD_FIELD: &str = "\
IMPORT collections
IMPORT io

TYPE Bag
  label AS String
  items AS List OF String
END TYPE

MUT r AS Bag = Bag[label := \"bag\", items := [\"one\", \"two\", \"three\"]]

SUB clobber(xs AS List OF String)
  r = Bag[label := \"new\", items := [\"n\"]]
  MUT pad AS List OF String = [\"zzzzzzzzzzzzzzzzzzzzzzzzz\", \"yyyyyyyyyyyyyyyyyyyyyyyy\", \"xxxxxxxxxxxxxxxxxxxxxxx\"]
  io::print(\"param: \" & collections::get(xs, 0) & \" \" & collections::get(xs, 2) & \" len=\" & toString(len(xs)) & \" pad=\" & toString(len(pad)))
END SUB

SUB main()
  clobber(r.items)
  io::print(\"r.label=\" & r.label)
END SUB
";

/// Calls that cannot reach a write of `g` keep borrowing: a read-only callee
/// (which itself calls a read-only helper), `len(g)`, a `forEach` whose callback
/// only reads, and a callee that writes a DIFFERENT global.
const NO_COPY_CONTROLS: &str = "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"alpha\", \"beta\", \"gamma\"]
MUT other AS List OF String = []

FUNC first(xs AS List OF String) AS String
  RETURN collections::get(xs, 0)
END FUNC

SUB show(xs AS List OF String)
  io::print(first(xs) & \" \" & toString(len(xs)))
END SUB

SUB note(v AS String)
  io::print(\"note \" & v)
END SUB

SUB touchOther(xs AS List OF String)
  other = collections::append(other, collections::get(xs, 1))
END SUB

SUB main()
  show(g)
  io::print(toString(len(g)))
  collections::forEach(g, note)
  touchOther(g)
  io::print(\"other=\" & collections::get(other, 0))
END SUB
";

/// A loop that reassigns `g` inside the callee every iteration: with the copy
/// freed at the end of each call statement, the live arena does not grow with
/// the iteration count.
fn leak_source(n: usize) -> String {
    format!(
        "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"alpha\", \"beta\", \"gamma\"]
MUT total AS Integer = 0

SUB clobber(xs AS List OF String)
  g = collections::append(g, \"delta\")
  total = total + len(xs)
END SUB

FUNC main AS Integer
  FOR i = 1 TO {n}
    g = [\"alpha\", \"beta\", \"gamma\"]
    clobber(g)
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
"
    )
}

/// Stack slots of `main` allocated by the operand snapshot.
fn snapshot_slots(source: &str, name: &str) -> usize {
    let project = temp_project(name, source);
    let ncode = build_ncode(&project, "macos-aarch64", name);
    let main = ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["name"].as_str() == Some("main"))
        .expect("main function");
    let count = main["stackSlots"]
        .as_array()
        .expect("stackSlots")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some("operand_snapshot"))
        .count();
    let _ = std::fs::remove_dir_all(&project);
    count
}

fn run(source: &str, name: &str) -> (i32, String, String) {
    let project = temp_project(name, source);
    let executable = build_project(&project);
    let result = run_capture_with_env(&executable, &[]);
    let _ = std::fs::remove_dir_all(&project);
    result
}

fn assert_runs(source: &str, name: &str, expected: &str) {
    let (code, stdout, stderr) = run(source, name);
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(stdout, expected);
}

#[test]
fn a_user_callee_that_reassigns_the_global_leaves_the_parameter_intact() {
    assert_runs(
        REPRO,
        "rt_global_arg_repro",
        "param: alpha gamma len=3\n3\ng len=5\n",
    );
    assert_eq!(snapshot_slots(REPRO, "codegen_global_arg_repro"), 1);
}

#[test]
fn a_callee_reached_through_a_func_value_fails_closed() {
    assert_runs(
        FUNC_VALUE,
        "rt_global_arg_func_value",
        "param: alpha gamma len=3\n3\ng len=5\n",
    );
}

#[test]
fn a_string_global_argument_survives_its_reassignment() {
    assert_runs(
        STRING_GLOBAL,
        "rt_global_arg_string",
        "param: the original global string value pad=42\ns=a replacement that is also quite long\n",
    );
}

#[test]
fn a_higher_order_builtin_walks_the_value_from_the_call() {
    // Each builtin walks `g` as it was at the call; the callbacks' appends land
    // in `g` and are what the next statement sees.
    assert_runs(
        HOF_CALLBACKS,
        "rt_global_arg_hof",
        "visit alpha pad=3\nvisit beta pad=3\nvisit gamma pad=3\ng len=6\nkept len=5 g len=12\n",
    );
    assert_eq!(snapshot_slots(HOF_CALLBACKS, "codegen_global_arg_hof"), 2);
}

#[test]
fn a_field_of_a_global_record_survives_the_records_reassignment() {
    assert_runs(
        RECORD_FIELD,
        "rt_global_arg_field",
        "param: one three len=3 pad=3\nr.label=new\n",
    );
}

#[test]
fn calls_that_cannot_write_the_global_keep_borrowing() {
    assert_runs(
        NO_COPY_CONTROLS,
        "rt_global_arg_controls",
        "alpha 3\n3\nnote alpha\nnote beta\nnote gamma\nother=beta\n",
    );
    assert_eq!(
        snapshot_slots(NO_COPY_CONTROLS, "codegen_global_arg_controls"),
        0
    );
}

#[cfg(unix)]
#[test]
fn the_argument_copy_is_freed_every_call() {
    use common::debug_report::{arena_lines, build_debug, counter, run_ok};
    let live = |n: usize| {
        let name = format!("dbg_global_arg_leak_{n}");
        let exe = build_debug(&name, &leak_source(n));
        let (stdout, stderr) = run_ok(&name, &exe);
        assert_eq!(stdout, format!("{}\n", 3 * n));
        counter(&name, &arena_lines(&name, &stderr), 0, "live_bytes")
    };
    let small = live(500);
    let large = live(1000);
    assert!(
        large.saturating_sub(small) < 500 * 16,
        "500 more calls must not leave 500 more copies live: {small} -> {large}"
    );
}
