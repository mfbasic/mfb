//! bug-496 (audit-3 MEM-12): `g = <op>(g, f(...))` / `GS & f(...)` where `f`
//! reassigns the global `g`/`GS` must consume operand 0 as it was BEFORE the
//! call, not the block the call's reassignment freed.
//!
//! Operand 0 lowers to a pointer into the global's current block; a later
//! operand's call reassigns the global, whose `StoreGlobal` frees that block; the
//! op then reads the freed block. For `&` the byte length at offset 0 is the
//! arena's free-node link, so operand 0's bytes silently vanish (`[tail] len=4`
//! instead of `[abcdefghtail] len=12`); for `collections::append` the recycled
//! COUNT/DATA_LENGTH words become a nonsense allocation size (`7-701-0001`).
//!
//! The fix (`src/codegen/engine/value/operand_snapshot.rs`) deep-copies such an
//! operand into a statement-scope temporary when it is lowered, before the later
//! operand runs. The defined semantics are unchanged: `.ai/collections.md`
//! already specifies operand 0 as the pre-call value, so the nested write is
//! (correctly) lost — `evict` storing `[99]` never shows in the result — and only
//! operand 0's bytes are preserved.
//!
//! The controls pin the narrowness: a plain local (which no callee can reach) and
//! a global followed only by pure native builtins emit no snapshot slot, so the
//! in-place `x = append(x, <pure expr>)` fast path and ordinary `s & f()` on a
//! local are byte-for-byte untouched.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, build_project, run_capture_with_env, temp_project};

/// MEM-12 verbatim: `other` reassigns a DIFFERENT global (control), `same`
/// reassigns the left operand itself.
const CONCAT: &str = "\
IMPORT io\n\
IMPORT strings\n\
MUT GS AS String = \"abcdefgh\"\n\
MUT G2 AS String = \"12345678\"\n\
FUNC other(v AS Integer) AS String\n\
  G2 = \"XY\"\n\
  RETURN \"tail\"\n\
END FUNC\n\
FUNC same(v AS Integer) AS String\n\
  GS = \"XY\"\n\
  RETURN \"tail\"\n\
END FUNC\n\
FUNC main() AS Integer\n\
  LET a AS String = GS & other(1)\n\
  io::print(\"other -> [\" & a & \"] len=\" & toString(strings::byteLen(a)))\n\
  LET b AS String = GS & same(1)\n\
  io::print(\"same  -> [\" & b & \"] len=\" & toString(strings::byteLen(b)))\n\
  io::print(\"GS=\" & GS)\n\
  RETURN 0\n\
END FUNC\n";

/// The `collections::append` global-evictor variant: `evict` replaces `G` while
/// the append's operand 0 still points at the old block.
const APPEND_EVICTOR: &str = "\
IMPORT io\n\
IMPORT collections\n\
MUT G AS List OF Integer = [1, 2, 3]\n\
FUNC evict(v AS Integer) AS Integer\n\
  G = [99]\n\
  RETURN v\n\
END FUNC\n\
FUNC main() AS Integer\n\
  FOR r = 0 TO 2\n\
    G = collections::append(G, evict(r))\n\
  NEXT\n\
  MUT s AS String = \"\"\n\
  FOR EACH v IN G\n\
    s = s & toString(v) & \",\"\n\
  NEXT\n\
  io::print(s)\n\
  RETURN 0\n\
END FUNC\n";

/// Shapes that must NOT snapshot: a local operand (unreachable by any callee)
/// with a user call after it, the pure in-place append, and a global followed
/// only by native builtins.
const NO_SNAPSHOT_CONTROLS: &str = "\
IMPORT io\n\
IMPORT collections\n\
MUT GS AS String = \"abc\"\n\
FUNC f(v AS Integer) AS Integer\n\
  RETURN v\n\
END FUNC\n\
FUNC g(v AS Integer) AS String\n\
  RETURN \"t\"\n\
END FUNC\n\
FUNC main() AS Integer\n\
  MUT xs AS List OF Integer = []\n\
  MUT s AS String = \"a\"\n\
  FOR i = 0 TO 4\n\
    xs = collections::append(xs, i * 2)\n\
    xs = collections::append(xs, f(i))\n\
    s = s & g(i)\n\
  NEXT\n\
  LET t AS String = GS & toString(len(xs)) & s\n\
  io::print(t)\n\
  RETURN 0\n\
END FUNC\n";

/// bug-737: a global operand followed by a user call that CANNOT store to that
/// global. `examples/brogue`'s `layerAt` is `collections::get(pmapLayers,
/// layerIndex(x, y, layer))`: the index helper is pure, yet the rule "any later
/// user call" copied the whole 9,164-cell layer list on every read — 98% of the
/// 6.1 GB one `dig` allocated. A read-only callee (`peek`, which reads `G`
/// itself), a pure one (`idx`) and one that stores a DIFFERENT global (`bump`)
/// can none of them free `G`'s block, so none of these operands is copied.
const NO_STORE_SIBLINGS: &str = "\
IMPORT io\n\
IMPORT collections\n\
MUT G AS List OF Integer = [10, 20, 30, 40]\n\
MUT GS AS String = \"ab\"\n\
MUT COUNTER AS Integer = 0\n\
FUNC idx(i AS Integer) AS Integer\n\
  RETURN i MOD 4\n\
END FUNC\n\
FUNC peek(i AS Integer) AS Integer\n\
  RETURN collections::get(G, i MOD 4)\n\
END FUNC\n\
FUNC bump(i AS Integer) AS Integer\n\
  COUNTER = COUNTER + 1\n\
  RETURN i MOD 4\n\
END FUNC\n\
FUNC tag(i AS Integer) AS String\n\
  RETURN toString(i)\n\
END FUNC\n\
FUNC readAt(i AS Integer) AS Integer\n\
  RETURN collections::get(G, idx(i))\n\
END FUNC\n\
FUNC main() AS Integer\n\
  MUT total AS Integer = 0\n\
  FOR i = 0 TO 7\n\
    total = total + collections::get(G, idx(i)) + collections::get(G, peek(i) / 10 - 1)\n\
    total = total + collections::get(G, bump(i)) + readAt(i)\n\
    G = collections::set(G, idx(i), collections::get(G, idx(i)) + 1)\n\
  NEXT\n\
  LET s AS String = GS & tag(total)\n\
  io::print(s & \" \" & toString(COUNTER))\n\
  RETURN 0\n\
END FUNC\n";

/// The reach is transitive: `relay` stores nothing itself but calls `evict`,
/// which replaces `G`, so the append's operand 0 is still copied.
const TRANSITIVE_EVICTOR: &str = "\
IMPORT io\n\
IMPORT collections\n\
MUT G AS List OF Integer = [1, 2, 3]\n\
FUNC evict(v AS Integer) AS Integer\n\
  G = [99]\n\
  RETURN v\n\
END FUNC\n\
FUNC relay(v AS Integer) AS Integer\n\
  RETURN evict(v) + 0\n\
END FUNC\n\
FUNC main() AS Integer\n\
  FOR r = 0 TO 2\n\
    G = collections::append(G, relay(r))\n\
  NEXT\n\
  MUT s AS String = \"\"\n\
  FOR EACH v IN G\n\
    s = s & toString(v) & \",\"\n\
  NEXT\n\
  io::print(s)\n\
  RETURN 0\n\
END FUNC\n";

/// Stack slots of `main` allocated by the operand snapshot (`operand_snapshot`).
fn snapshot_slots(source: &str, name: &str) -> usize {
    snapshot_slots_in(source, name, "main")
}

/// Stack slots of function `function` allocated by the operand snapshot.
fn snapshot_slots_in(source: &str, name: &str, function: &str) -> usize {
    let project = temp_project(name, source);
    let ncode = build_ncode(&project, "macos-aarch64", name);
    let main = ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["name"].as_str() == Some(function))
        .expect("the function");
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

#[test]
fn concat_operand_survives_a_call_that_reassigns_the_global() {
    let (code, stdout, stderr) = run(CONCAT, "rt_operand_snapshot_concat");
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(
        stdout, "other -> [abcdefghtail] len=12\nsame  -> [abcdefghtail] len=12\nGS=XY\n",
        "operand 0 must be GS as it was before `same` ran; the nested write is \
         what `GS` holds afterwards"
    );
    // Only `same` can reach a store of `GS`, so only its line snapshots. The
    // `other` line needs no copy (bug-737): `other` stores `G2`, which leaves
    // `GS`'s block alone — bug-496's own repro printed that line correctly with
    // no snapshot at all, before the fix.
    assert_eq!(snapshot_slots(CONCAT, "codegen_operand_snapshot_concat"), 1);
}

#[test]
fn append_operand_survives_an_evictor_that_reassigns_the_global() {
    let (code, stdout, stderr) = run(APPEND_EVICTOR, "rt_operand_snapshot_append");
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    // Value semantics: each append extends G as it was before `evict` ran, so
    // the evictor's `[99]` never survives and the original elements do.
    assert_eq!(stdout, "1,2,3,0,1,2,\n");
    assert_eq!(
        snapshot_slots(APPEND_EVICTOR, "codegen_operand_snapshot_append"),
        1
    );
}

#[test]
fn locals_and_pure_native_siblings_emit_no_snapshot() {
    assert_eq!(
        snapshot_slots(NO_SNAPSHOT_CONTROLS, "codegen_operand_snapshot_controls"),
        0
    );
    let (code, stdout, stderr) = run(NO_SNAPSHOT_CONTROLS, "rt_operand_snapshot_controls");
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(stdout, "abc10attttt\n");
}

#[test]
fn a_global_operand_before_a_call_that_cannot_store_it_is_not_copied() {
    assert_eq!(
        snapshot_slots(NO_STORE_SIBLINGS, "codegen_operand_snapshot_no_store"),
        0,
        "no later call here can reach a StoreGlobal of `G` or `GS`, so no operand \
         may be copied (bug-737)"
    );
    assert_eq!(
        snapshot_slots_in(
            NO_STORE_SIBLINGS,
            "codegen_operand_snapshot_no_store_reader",
            "readAt"
        ),
        0,
        "`collections::get(G, idx(i))` with a pure `idx` must read `G` in place"
    );
    let (code, stdout, stderr) = run(NO_STORE_SIBLINGS, "rt_operand_snapshot_no_store");
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(stdout, "ab816 8\n");
}

#[test]
fn a_call_that_reaches_the_store_through_a_helper_still_snapshots() {
    let (code, stdout, stderr) = run(TRANSITIVE_EVICTOR, "rt_operand_snapshot_transitive");
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(stdout, "1,2,3,0,1,2,\n");
    assert_eq!(
        snapshot_slots(TRANSITIVE_EVICTOR, "codegen_operand_snapshot_transitive"),
        1
    );
}
