//! Which lowering a program receives, for the collection self-update arms and the
//! shapes around them that no other program in the corpus takes.
//!
//! Every row is a program, the function whose labels are read, the label families
//! that function MUST define, and the families it must NOT. A label family is the
//! stable name of a code shape (`inplace_mapvalues_unwind_*`); the trailing serial
//! churns, so the rows compare stems. The negative column is what turns a row from
//! "this lowered" into a contract: a self-update that declines must fall back to
//! the copying lowering (`replace_list_*`, `transform_call_*`, `merge_*`), never
//! emit a half of the in-place one.
//!
//! The reason on each row is the property it pins; the programs are small and each
//! is aimed at one decision in `collection/assign/**`, `collection/list/**`,
//! `collection/map/**` or `collection/compare/**`. This file holds the rows for
//! replace / transform / mapValues / merge / repeat, the record-temp scratch,
//! self-slice appends and element compares; `lowering_shapes_fields.rs` holds the
//! rows for record fields, set algebra, compaction, sort and thread sends.

use std::collections::BTreeSet;

use super::stems::stems_in;

pub(super) struct Row {
    pub(super) name: &'static str,
    pub(super) source: &'static str,
    pub(super) function: &'static str,
    pub(super) have: &'static [&'static str],
    pub(super) lack: &'static [&'static str],
    pub(super) why: &'static str,
}

fn defines(stems: &BTreeSet<String>, family: &str) -> bool {
    stems.iter().any(|stem| stem.starts_with(family))
}

/// Check every row; report every failure rather than the first.
pub(super) fn assert_rows(rows: &[Row]) {
    let mut failures = Vec::new();
    for row in rows {
        let stems = stems_in(row.source, row.function);
        for family in row.have {
            if !defines(&stems, family) {
                failures.push(format!(
                    "{}: `{}` defines no `{family}*` label ({})",
                    row.name, row.function, row.why
                ));
            }
        }
        for family in row.lack {
            if defines(&stems, family) {
                failures.push(format!(
                    "{}: `{}` defines a `{family}*` label it must not ({})",
                    row.name, row.function, row.why
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const RW_DECLINE_OTHER: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET xs AS List OF String = ["a", "bb", "a"]
  MUT ys AS List OF String = ["q"]
  ys = collections::replace(xs, "a", "z")
  io::print(toString(len(ys)))
  RETURN 0
END FUNC
"####;
const RW_FIELD_NOT_LAST: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  names AS List OF String
  tail AS String
END TYPE

FUNC up(s AS String) AS String
  RETURN s & "!"
END FUNC

FUNC main() AS Integer
  MUT r AS Rec = Rec[names := ["a", "bb"], tail := "t"]
  r = WITH r { names := collections::replace(r.names, "a", "a longer replacement") }
  r = WITH r { names := collections::transform(r.names, up) }
  io::print(toString(len(r.names)))
  RETURN 0
END FUNC
"####;
const RW_FIELD_READS_OWNER: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  tail AS String
  names AS List OF String
END TYPE

FUNC main() AS Integer
  MUT r AS Rec = Rec[tail := "t", names := ["a", "bb"]]
  r = WITH r { names := collections::replace(r.names, r.tail, "a longer replacement") }
  r = WITH r { names := collections::replace(r.names, "a", r.tail) }
  io::print(toString(len(r.names)))
  RETURN 0
END FUNC
"####;
const RW_TRANSFORM_RETYPE: &str = r####"IMPORT collections
IMPORT io

FUNC label(n AS Integer) AS String
  RETURN toString(n)
END FUNC

FUNC relabel(xs AS List OF Integer) AS List OF String
  RETURN collections::transform(xs, label)
END FUNC

FUNC main() AS Integer
  io::print(toString(len(relabel([1, 2, 3]))))
  RETURN 0
END FUNC
"####;
const MV_STRING_FAIL: &str = r####"IMPORT collections
IMPORT io

FUNC tag(s AS String) AS String
  IF len(s) = 0 THEN FAIL error(1, "empty")
  RETURN s & "!"
END FUNC

FUNC main() AS Integer
  MUT m AS Map OF String TO String = Map OF String TO String { "a" := "x", "b" := "yy" }
  m = collections::mapValues(m, tag)
  io::print(toString(len(m)))
  RETURN 0
END FUNC
"####;
const MV_INTEGER_FAIL: &str = r####"IMPORT collections
IMPORT io

FUNC halve(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(1, "neg")
  RETURN n / 2
END FUNC

FUNC main() AS Integer
  MUT m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 4, "b" := 6 }
  m = collections::mapValues(m, halve)
  io::print(toString(len(m)))
  RETURN 0
END FUNC
"####;
const MERGE_ALIAS_AND_STRINGS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT m AS Map OF String TO String = Map OF String TO String { "a" := "x" }
  LET n AS Map OF String TO String = Map OF String TO String { "a" := "longer", "c" := "zzz" }
  m = collections::merge(m, m, TRUE)
  m = collections::merge(m, n, TRUE)
  m = collections::merge(m, n, FALSE)
  io::print(toString(len(m)))
  RETURN 0
END FUNC
"####;
const MERGE_FIELD: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  m AS Map OF String TO String
  tail AS String
END TYPE

TYPE Rec2
  tail AS String
  m AS Map OF String TO String
END TYPE

FUNC main() AS Integer
  MUT r AS Rec = Rec[m := Map OF String TO String { "a" := "x" }, tail := "t"]
  MUT s AS Rec2 = Rec2[tail := "t", m := Map OF String TO String { "a" := "x" }]
  LET n AS Map OF String TO String = Map OF String TO String { "a" := "longer", "c" := "zzz" }
  r = WITH r { m := collections::merge(r.m, n, TRUE) }
  s = WITH s { m := collections::merge(s.m, n, TRUE) }
  s = WITH s { m := collections::merge(s.m, s.m, TRUE) }
  io::print(toString(len(r.m) + len(s.m)))
  RETURN 0
END FUNC
"####;
const MV_FIELD: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  m AS Map OF String TO String
  tail AS String
END TYPE

TYPE Rec2
  tail AS String
  m AS Map OF String TO String
END TYPE

FUNC tag(s AS String) AS String
  RETURN s & "!"
END FUNC

FUNC main() AS Integer
  MUT r AS Rec = Rec[m := Map OF String TO String { "a" := "x" }, tail := "t"]
  MUT s AS Rec2 = Rec2[tail := "t", m := Map OF String TO String { "a" := "x" }]
  r = WITH r { m := collections::mapValues(r.m, tag) }
  s = WITH s { m := collections::mapValues(s.m, tag) }
  io::print(toString(len(r.m) + len(s.m)))
  RETURN 0
END FUNC
"####;
const REPEAT_STRINGS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF String = ["a", "bb"]
  xs = collections::repeat(xs, "zzzz", 3)
  xs = collections::repeat(xs, "q", 2, 1)
  xs = collections::repeat(xs, "tail", 4, 5)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const REPEAT_LISTS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF List OF Integer = [[1], [2, 3]]
  xs = collections::repeat(xs, [7, 8, 9], 3)
  xs = collections::repeat(xs, [4], 2, 2)
  xs = collections::repeat(xs, [5, 5], 6, 5)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const REPEAT_FIELD: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  xs AS List OF String
  tail AS String
END TYPE

TYPE Rec2
  tail AS String
  xs AS List OF String
END TYPE

FUNC main() AS Integer
  MUT r AS Rec = Rec[xs := ["a"], tail := "t"]
  MUT s AS Rec2 = Rec2[tail := "t", xs := ["a"]]
  r = WITH r { xs := collections::repeat(r.xs, "zzzzzzzz", 5) }
  s = WITH s { xs := collections::repeat(s.xs, "zzzzzzzz", 5) }
  s = WITH s { xs := collections::repeat(s.xs, s.tail, 5, 1) }
  io::print(toString(len(r.xs) + len(s.xs)))
  RETURN 0
END FUNC
"####;
const REPEAT_VALUE_FORMS: &str = r####"IMPORT collections
IMPORT io

FUNC grow(xs AS List OF String) AS List OF String
  RETURN collections::repeat(xs, "pad", 3, 0)
END FUNC

FUNC main() AS Integer
  LET a AS List OF String = grow(["x"])
  LET b AS List OF List OF Integer = collections::repeat([[1]], [2, 2], 4)
  io::print(toString(len(a) + len(b)))
  RETURN 0
END FUNC
"####;
const REPEAT_GRAPH: &str = r####"IMPORT collections
IMPORT io

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node
  kids AS List OF Tree
END TYPE

UNION Tree
  Node
  Leaf
END UNION

FUNC main() AS Integer
  MUT ts AS List OF Tree = [Leaf[v := 1]]
  ts = collections::repeat(ts, Node[kids := [Leaf[v := 2]]], 3)
  ts = collections::repeat(ts, Leaf[v := 9], 2, 1)
  io::print(toString(len(ts)))
  RETURN 0
END FUNC
"####;
const RECORD_TEMPS: &str = r####"IMPORT collections
IMPORT io

TYPE Body
  x AS Float
  vx AS Float
END TYPE

FUNC main() AS Integer
  MUT xs AS List OF Body = [Body[x := 1.0, vx := 2.0], Body[x := 3.0, vx := 4.0]]
  MUT n AS Integer = 0
  FOR j = 0 TO 1
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    IF j = 0 THEN
      xs = collections::set(xs, 0, WITH a { x := a.x + b.vx })
    ELSE
      xs = collections::set(xs, 1, WITH b { x := b.x + a.vx })
    END IF
    MATCH j
      CASE 0
        xs = collections::set(xs, 0, WITH a { vx := a.vx + 1.0 })
      CASE ELSE
        xs = collections::set(xs, 1, WITH b { vx := b.vx + 1.0 })
    END MATCH
  NEXT
  WHILE n < 2
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    xs = collections::set(xs, 0, WITH a { x := a.x + b.vx })
    n = n + 1
  END WHILE
  DO
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    xs = collections::set(xs, 0, WITH a { x := a.x + b.vx })
    n = n + 1
  LOOP UNTIL n > 3
  FOR EACH item IN xs
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    xs = collections::set(xs, 1, WITH b { x := b.x + a.vx })
  NEXT
  xs = collections::append(xs, Body[x := 0.0, vx := 0.0]) TRAP(e)
    RECOVER xs
  END TRAP
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const SELF_SLICE_APPEND: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF Integer = [1, 2, 3, 4, 5]
  LET a AS Integer = 1
  LET b AS Integer = 3
  xs = collections::append(xs, collections::mid(xs, a + 1, b - a))
  xs = collections::append(xs, collections::mid(xs, 0, 2))
  xs = collections::append(xs, collections::mid(xs, a * 2, 1))
  xs = collections::append(xs, collections::mid(xs, len(xs) - 1, 1))
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const ROWS: &[Row] = &[
    Row {
        name: "rw_decline_other",
        source: RW_DECLINE_OTHER,
        function: "main",
        have: &["replace_list_copy_loop"],
        lack: &["inplace_replace"],
        why: "a replace whose result is bound to ANOTHER local is not a self-update, so it copies",
    },
    Row {
        name: "rw_field_not_last",
        source: RW_FIELD_NOT_LAST,
        function: "main",
        have: &["replace_list_copy_loop", "transform_call_loop"],
        lack: &["inplace_replace", "inplace_transform"],
        why: "replace/transform may repack the list, which at a field is only sound for the last inlined field",
    },
    Row {
        name: "rw_field_reads_owner",
        source: RW_FIELD_READS_OWNER,
        function: "main",
        have: &["replace_list_copy_loop"],
        lack: &["inplace_replace"],
        why: "an operand that reads the owner may be a view the repack frees, so the arm declines",
    },
    Row {
        name: "rw_transform_retype",
        source: RW_TRANSFORM_RETYPE,
        function: "relabel",
        have: &["transform_call_loop"],
        lack: &["inplace_transform"],
        why: "RETURN transform(x, f) to ANOTHER element type cannot rewrite x in place",
    },
    Row {
        name: "mv_string_fail",
        source: MV_STRING_FAIL,
        function: "main",
        have: &["inplace_mapvalues_unwind", "inplace_mapvalues_room_loop"],
        lack: &[],
        why: "a String-valued mapValues parks its results, reserves room, and unwinds on failure",
    },
    Row {
        name: "mv_integer_fail",
        source: MV_INTEGER_FAIL,
        function: "main",
        have: &["inplace_mapvalues_loop"],
        lack: &["inplace_mapvalues_unwind", "inplace_mapvalues_room"],
        why: "an Integer-valued mapValues owns nothing to unwind and needs no room",
    },
    Row {
        name: "merge_alias_and_strings",
        source: MERGE_ALIAS_AND_STRINGS,
        function: "main",
        have: &["inplace_merge_loop", "inplace_borrow_str_done"],
        lack: &[],
        why: "a String-valued merge borrows the other map's payloads into scratch",
    },
    Row {
        name: "merge_field",
        source: MERGE_FIELD,
        function: "main",
        have: &["inplace_merge_loop", "merge_loop"],
        lack: &[],
        why: "merge at the last inlined field is in place; at an earlier field it copies",
    },
    Row {
        name: "mv_field",
        source: MV_FIELD,
        function: "main",
        have: &["inplace_mapvalues_loop"],
        lack: &[],
        why: "mapValues at the last inlined field is in place",
    },
    Row {
        name: "repeat_strings",
        source: REPEAT_STRINGS,
        function: "main",
        have: &["list_repeat_write", "list_repeat_realloc"],
        lack: &[],
        why: "repeat over variable-width elements reserves once, then writes",
    },
    Row {
        name: "repeat_lists",
        source: REPEAT_LISTS,
        function: "main",
        have: &["list_repeat_write", "list_repeat_realloc"],
        lack: &[],
        why: "repeat over nested lists reserves once, then writes",
    },
    Row {
        name: "repeat_field",
        source: REPEAT_FIELD,
        function: "main",
        have: &["list_repeat_write", "inline_grow_prefix_done"],
        lack: &[],
        why: "repeat at the last inlined field grows the owner",
    },
    Row {
        name: "repeat_value_forms",
        source: REPEAT_VALUE_FORMS,
        function: "grow",
        have: &["list_repeat_write"],
        lack: &[],
        why: "the value form of repeat shares the in-place core",
    },
    Row {
        name: "repeat_graph",
        source: REPEAT_GRAPH,
        function: "main",
        have: &["list_repeat_write", "graph_value_drop_skip", "shallow_block_free_skip"],
        lack: &[],
        why: "repeat of a cycle-typed element builds private copies, then frees only their list block",
    },
    Row {
        name: "record_temps",
        source: RECORD_TEMPS,
        function: "main",
        have: &["su_scratch_alloc_ok", "set_inplace_valid"],
        lack: &[],
        why: "a set(xs, i, WITH ...) loop builds its WITH items in the record-temp scratch",
    },
    Row {
        name: "self_slice_append",
        source: SELF_SLICE_APPEND,
        function: "main",
        have: &["self_slice_append_data_done"],
        lack: &[],
        why: "append(xs, mid(xs, a + 1, b - a)) is a self-slice append over fixed-width elements",
    },
];

#[test]
fn each_program_takes_the_lowering_its_row_names() {
    assert_rows(ROWS);
}
