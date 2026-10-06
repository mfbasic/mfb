//! bug-895: a growing in-place field self-update whose item reads the owner.
//!
//! `r = WITH r { xs := collections::append(r.xs, r.s) }` runs in place at the
//! owner's last inlined field: the grow reallocates the owner's block and frees the
//! old one. The item was lowered without a copy, so `r.s` was still a view into the
//! OLD block when the grow copied it in — after the free had entropy-scrubbed it.
//! The new element was garbage (or, for an inlined record item, a segfault).
//! `mfb spec memory collections`, *Self-updates*: "The operation's result and every
//! other value are exactly what the copying form would give."

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const TYPES: &str = "\
IMPORT io
IMPORT collections
IMPORT strings

TYPE R2
  n AS Integer
  s AS String
  xs AS List OF String
END TYPE

TYPE RM
  s AS String
  m AS Map OF String TO String
END TYPE

TYPE RI
  s AS String
  m AS Map OF Integer TO String
END TYPE

TYPE RS
  s AS String
  st AS Set OF String
END TYPE

TYPE Pt
  name AS String
  n AS Integer
END TYPE

TYPE C
  p AS Pt
  ps AS List OF Pt
END TYPE

TYPE Inner
  s AS String
  xs AS List OF String
END TYPE

TYPE Outer
  tag AS String
  inner AS Inner
END TYPE

FUNC addSelf(r AS R2) AS R2
  RETURN WITH r { xs := collections::append(r.xs, r.s) }
END FUNC

FUNC long(c AS String) AS String
  RETURN strings::repeat(c, 100)
END FUNC
";

/// (name, body of `main`, expected stdout).
const CASES: &[(&str, &str, &str)] = &[
    (
        "append",
        "  MUT w AS R2 = R2[1, long(\"s\"), [\"a\", \"b\", \"c\"]]
  w = WITH w { xs := collections::append(w.xs, w.s) }
  io::print(toString(collections::get(w.xs, 3) = long(\"s\")))",
        "TRUE\n",
    ),
    (
        "append_short",
        "  MUT w AS R2 = R2[1, \"short\", [\"a\", \"b\", \"c\"]]
  w = WITH w { xs := collections::append(w.xs, w.s) }
  io::print(\"[\" & collections::get(w.xs, 3) & \"]\")",
        "[short]\n",
    ),
    (
        "prepend",
        "  MUT w AS R2 = R2[1, long(\"p\"), [\"a\", \"b\", \"c\"]]
  w = WITH w { xs := collections::prepend(w.xs, w.s) }
  io::print(toString(collections::get(w.xs, 0) = long(\"p\")))",
        "TRUE\n",
    ),
    (
        "insert",
        "  MUT w AS R2 = R2[1, long(\"i\"), [\"a\", \"b\", \"c\"]]
  w = WITH w { xs := collections::insert(w.xs, 1, w.s) }
  io::print(toString(collections::get(w.xs, 1) = long(\"i\")))",
        "TRUE\n",
    ),
    (
        "alias_forwarded",
        "  MUT w AS R2 = R2[1, long(\"a\"), [\"a\", \"b\", \"c\"]]
  LET alias = w.s
  w = WITH w { xs := collections::append(w.xs, alias) }
  io::print(toString(collections::get(w.xs, 3) = long(\"a\")))",
        "TRUE\n",
    ),
    (
        "map_set_key_and_value",
        "  MUT r AS RM = RM[long(\"k\"), Map OF String TO String { \"x\" := \"y\" }]
  r = WITH r { m := collections::set(r.m, r.s, r.s) }
  io::print(toString(len(r.m)) & \" \" & toString(collections::get(r.m, long(\"k\")) = long(\"k\")))",
        "2 TRUE\n",
    ),
    (
        "map_set_value",
        "  MUT r AS RI = RI[long(\"v\"), Map OF Integer TO String { 1 := \"y\" }]
  r = WITH r { m := collections::set(r.m, 2, r.s) }
  io::print(toString(collections::get(r.m, 2) = long(\"v\")))",
        "TRUE\n",
    ),
    (
        "set_add",
        "  MUT a AS RS = RS[long(\"E\"), Set OF String {\"q\"}]
  a = WITH a { st := collections::add(a.st, a.s) }
  io::print(toString(len(a.st)) & \" \" & toString(collections::contains(a.st, long(\"E\"))))",
        "2 TRUE\n",
    ),
    (
        "record_item",
        "  MUT c AS C = C[Pt[long(\"n\"), 5], [Pt[\"a\", 1]]]
  c = WITH c { ps := collections::append(c.ps, c.p) }
  LET q AS Pt = collections::get(c.ps, 1)
  io::print(toString(q.n) & \" \" & toString(q.name = long(\"n\")))",
        "5 TRUE\n",
    ),
    (
        "return_with_param",
        "  MUT w AS R2 = R2[1, long(\"r\"), [\"a\", \"b\", \"c\"]]
  w = addSelf(w)
  io::print(toString(collections::get(w.xs, 3) = long(\"r\")))",
        "TRUE\n",
    ),
    (
        "nested_inner_item",
        "  MUT o AS Outer = Outer[\"t\", Inner[long(\"z\"), [\"a\"]]]
  o = WITH o { inner := WITH o.inner { xs := collections::append(o.inner.xs, o.inner.s) } }
  io::print(toString(collections::get(o.inner.xs, 1) = long(\"z\")))",
        "TRUE\n",
    ),
    (
        "nested_outer_item",
        "  MUT o AS Outer = Outer[long(\"o\"), Inner[\"s\", [\"a\"]]]
  o = WITH o { inner := WITH o.inner { xs := collections::append(o.inner.xs, o.tag) } }
  io::print(toString(collections::get(o.inner.xs, 1) = long(\"o\")))",
        "TRUE\n",
    ),
];

#[test]
fn a_growing_field_update_copies_an_owner_item_before_the_grow() {
    let mut failures = Vec::new();
    for (name, body, want) in CASES {
        let source = format!("{TYPES}\nFUNC main() AS Nothing\n{body}\nEND FUNC\n");
        let project = common::temp_project(&format!("field_grow_owner_{name}"), &source);
        for level in ["0", "1", "2"] {
            let exe = common::build_project_opt(&project, level);
            let output = Command::new(&exe).output().expect("run program");
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !output.status.success() || stdout != *want {
                failures.push(format!(
                    "{name} -O{level}: want {want:?}, got {stdout:?} ({}) stderr {:?}",
                    common::exit_description(&output.status),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
