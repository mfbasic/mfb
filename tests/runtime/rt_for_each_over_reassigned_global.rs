//! bug-666: `FOR EACH v IN g` over a module-level global must visit exactly the
//! elements `g` held at loop entry, even when the body reassigns `g`.
//!
//! `lower_for_each` stores the iterable's block pointer for the whole loop and
//! reads its count once. A `Global` iterable is `g`'s live block, and the body's
//! `g = …` lowers through `NirOp::StoreGlobal`, which frees that block (the
//! bug-47 old-block free) — the next step reads freed memory (`7-701-0001`). A
//! plain local is already safe (its `Assign` fallback does not free a live
//! iterable); a global had no equivalent.
//!
//! The fix lowers the iterable owned — one statement-scope deep copy per loop
//! entry, freed when the `FOR EACH` statement ends on every exit edge — but only
//! when the body can reach a `StoreGlobal` of that global
//! (`src/codegen/engine/value/store_reach.rs`). A loop whose body only reads the
//! global keeps borrowing it.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, build_project, run_capture_with_env, temp_project};

/// The bug report's reproduction, verbatim.
const REPRO: &str = "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"one\", \"two\", \"three\"]

SUB main()
  FOR EACH v IN g
    g = collections::append(g, v & \"!\")
    MUT pad AS List OF String = [\"qqqqqqqqqqqqqqqqqqqqqqqq\", \"wwwwwwwwwwwwwwwwwwwwwwwww\", \"eeeeeeeeeeeeeeeeeeeeeeee\"]
    io::print(\"loop sees: \" & v & \" pad=\" & toString(len(pad)))
  NEXT
  io::print(\"final len=\" & toString(len(g)))
END SUB
";

/// The write happens in a function the body calls.
const CALLED_FUNCTION: &str = "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"one\", \"two\", \"three\"]

SUB grow(v AS String)
  g = collections::append(g, v & \"!\")
END SUB

SUB main()
  FOR EACH v IN g
    grow(v)
    MUT pad AS List OF String = [\"qqqqqqqqqqqqqqqqqqqqqqqq\", \"wwwwwwwwwwwwwwwwwwwwwwwww\", \"eeeeeeeeeeeeeeeeeeeeeeee\"]
    io::print(\"loop sees: \" & v & \" pad=\" & toString(len(pad)))
  NEXT
  io::print(\"final len=\" & toString(len(g)))
END SUB
";

/// The iterable is a field of a global record the body reassigns.
const RECORD_FIELD: &str = "\
IMPORT collections
IMPORT io

TYPE Bag
  label AS String
  items AS List OF String
END TYPE

MUT r AS Bag = Bag[label := \"bag\", items := [\"one\", \"two\", \"three\"]]

SUB main()
  FOR EACH v IN r.items
    r = WITH r { items := collections::append(r.items, v & \"!\") }
    MUT pad AS List OF String = [\"qqqqqqqqqqqqqqqqqqqqqqqq\", \"wwwwwwwwwwwwwwwwwwwwwwwww\", \"eeeeeeeeeeeeeeeeeeeeeeee\"]
    io::print(\"loop sees: \" & v & \" pad=\" & toString(len(pad)))
  NEXT
  io::print(\"final len=\" & toString(len(r.items)))
END SUB
";

/// `Map` and `Set` globals take the entry-table arms of `lower_for_each`, not
/// the list arm; the same free reaches them.
const MAP_AND_SET: &str = "\
IMPORT collections
IMPORT io

MUT m AS Map OF String TO String = Map OF String TO String { \"a\" := \"apple\", \"b\" := \"banana\", \"c\" := \"cherry\" }
MUT s AS Set OF String = collections::toSet([\"x\", \"y\", \"z\"])

SUB main()
  FOR EACH e IN m
    m = collections::set(m, e.key & e.key, e.value & \"!\")
    MUT pad AS List OF String = [\"qqqqqqqqqqqqqqqqqqqqqqqq\", \"wwwwwwwwwwwwwwwwwwwwwwwww\", \"eeeeeeeeeeeeeeeeeeeeeeee\"]
    io::print(\"map sees: \" & e.key & \"=\" & e.value & \" pad=\" & toString(len(pad)))
  NEXT
  io::print(\"map len=\" & toString(len(m)))
  FOR EACH v IN s
    s = collections::add(s, v & v)
    MUT pad AS List OF String = [\"qqqqqqqqqqqqqqqqqqqqqqqq\", \"wwwwwwwwwwwwwwwwwwwwwwwww\", \"eeeeeeeeeeeeeeeeeeeeeeee\"]
    io::print(\"set sees: \" & v & \" pad=\" & toString(len(pad)))
  NEXT
  io::print(\"set len=\" & toString(len(s)))
END SUB
";

/// A loop over a global whose body only reads it (and writes a different
/// global) keeps borrowing: no copy.
const NO_COPY_CONTROL: &str = "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"one\", \"two\", \"three\"]
MUT seen AS List OF String = []

SUB note(v AS String)
  seen = collections::append(seen, v)
END SUB

SUB main()
  FOR EACH v IN g
    note(v & toString(len(g)))
  NEXT
  io::print(toString(len(seen)) & \" \" & collections::get(seen, 2))
END SUB
";

/// Every exit edge of the loop frees the copy: fall-through, `EXIT FOR`, and a
/// `RETURN` from inside the body.
fn leak_source(n: usize) -> String {
    format!(
        "\
IMPORT collections
IMPORT io

MUT g AS List OF String = [\"one\", \"two\", \"three\"]
MUT total AS Integer = 0

SUB whole()
  FOR EACH v IN g
    g = collections::append(g, v)
    total = total + 1
  NEXT
END SUB

SUB early()
  FOR EACH v IN g
    g = collections::append(g, v)
    total = total + 1
    IF v = \"two\" THEN EXIT FOR
  NEXT
END SUB

FUNC ret AS Integer
  FOR EACH v IN g
    g = collections::append(g, v)
    IF v = \"one\" THEN RETURN 1
  NEXT
  RETURN 0
END FUNC

FUNC main AS Integer
  FOR i = 1 TO {n}
    g = [\"one\", \"two\", \"three\"]
    whole()
    g = [\"one\", \"two\", \"three\"]
    early()
    g = [\"one\", \"two\", \"three\"]
    total = total + ret()
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
"
    )
}

/// Stack slots of `function` allocated by the operand snapshot.
fn snapshot_slots(source: &str, name: &str, function: &str) -> usize {
    let project = temp_project(name, source);
    let ncode = build_ncode(&project, "macos-aarch64", name);
    let found = ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["name"].as_str() == Some(function))
        .expect("function");
    let count = found["stackSlots"]
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

const EXPECTED: &str =
    "loop sees: one pad=3\nloop sees: two pad=3\nloop sees: three pad=3\nfinal len=6\n";

#[test]
fn a_body_that_reassigns_the_global_iterates_the_entry_value() {
    assert_runs(REPRO, "rt_for_each_global_repro", EXPECTED);
    assert_eq!(
        snapshot_slots(REPRO, "codegen_for_each_global_repro", "main"),
        1
    );
}

#[test]
fn a_body_that_reassigns_the_global_through_a_call_iterates_the_entry_value() {
    assert_runs(CALLED_FUNCTION, "rt_for_each_global_call", EXPECTED);
}

#[test]
fn a_field_of_a_global_record_iterates_the_entry_value() {
    assert_runs(RECORD_FIELD, "rt_for_each_global_field", EXPECTED);
}

#[test]
fn map_and_set_globals_iterate_the_entry_value() {
    assert_runs(
        MAP_AND_SET,
        "rt_for_each_global_map_set",
        "map sees: a=apple pad=3\nmap sees: b=banana pad=3\nmap sees: c=cherry pad=3\nmap len=6\n\
         set sees: x pad=3\nset sees: y pad=3\nset sees: z pad=3\nset len=6\n",
    );
}

#[test]
fn a_body_that_only_reads_the_global_keeps_borrowing() {
    assert_runs(NO_COPY_CONTROL, "rt_for_each_global_control", "3 three3\n");
    assert_eq!(
        snapshot_slots(NO_COPY_CONTROL, "codegen_for_each_global_control", "main"),
        0
    );
}

#[cfg(unix)]
#[test]
fn the_loop_copy_is_freed_on_every_exit_edge() {
    use common::debug_report::{arena_lines, build_debug, counter, run_ok};
    let live = |n: usize| {
        let name = format!("dbg_for_each_global_leak_{n}");
        let exe = build_debug(&name, &leak_source(n));
        let (stdout, stderr) = run_ok(&name, &exe);
        // 3 (whole) + 2 (early) + 1 (ret's RETURN 1) per iteration.
        assert_eq!(stdout, format!("{}\n", 6 * n));
        counter(&name, &arena_lines(&name, &stderr), 0, "live_bytes")
    };
    let small = live(500);
    let large = live(1000);
    assert!(
        large.saturating_sub(small) < 500 * 16,
        "500 more loops must not leave 500 more copies live: {small} -> {large}"
    );
}
