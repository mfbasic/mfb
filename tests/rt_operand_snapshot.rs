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

/// Stack slots of `main` allocated by the operand snapshot (`operand_snapshot`).
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

#[test]
fn concat_operand_survives_a_call_that_reassigns_the_global() {
    let (code, stdout, stderr) = run(CONCAT, "rt_operand_snapshot_concat");
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(
        stdout, "other -> [abcdefghtail] len=12\nsame  -> [abcdefghtail] len=12\nGS=XY\n",
        "operand 0 must be GS as it was before `same` ran; the nested write is \
         what `GS` holds afterwards"
    );
    // Both lines carry a user call after the global operand, so both snapshot.
    assert_eq!(snapshot_slots(CONCAT, "codegen_operand_snapshot_concat"), 2);
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
