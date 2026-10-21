//! bug-784 element updates, in process: `LET h = get(hs, k)` then
//! `hs = set(hs, k, WITH h { ... })` over a record nested in a list element
//! lowers to the element-update forms of `engine/control/builder_control/
//! element_update.rs` and `field_with.rs` (replace-in-place, write-back, `INTO`
//! destination calls, hidden binds), where the same updates through an owned
//! copy (`ident(get(..))`, which the borrow analysis does not see) take the
//! general copying `WITH` and `set`. The programs are those of
//! `tests/runtime/rt_record_nested_update.rs`, lowered instead of run.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

const PRELUDE: &str = "\
IMPORT io
IMPORT collections
IMPORT math

TYPE Wide
  id AS Integer
  count AS Integer
  total AS Integer
  flags AS Integer
  pos AS Float
  vel AS Float
  energy AS Float
  name AS String
  tag AS String
  weight AS Float
END TYPE

TYPE Holder
  id AS Integer
  label AS String
  inner AS Wide
  touches AS Integer
END TYPE

TYPE FHolder
  label AS String
  inner AS Wide
  score AS Float
END TYPE

TYPE LateHolder
  inner AS Wide
  label AS String
  touches AS Integer
END TYPE

FUNC stepWide(w AS Wide, i AS Integer, tag AS String) AS Wide
  RETURN WITH w { count := w.count + 1, total := w.total + i MOD 7, flags := (w.flags * 5 + i) MOD 1024, pos := w.pos + w.vel * 0.25, vel := w.vel * 0.5 + toFloat(i MOD 4), energy := w.energy + w.vel, tag := tag }
END FUNC

FUNC stepSelf(w AS Wide, i AS Integer) AS Wide
  RETURN WITH w { count := w.count + 1, total := w.count + w.total + i MOD 3, tag := w.name, pos := w.pos + w.vel }
END FUNC

FUNC newWide(id AS Integer) AS Wide
  RETURN Wide[id := id, count := 0, total := id, flags := id MOD 1024, pos := toFloat(id) * 0.5, vel := 1.0, energy := 0.0, name := \"wide-\" & toString(id), tag := \"\", weight := toFloat(id) * 0.125]
END FUNC

FUNC ident(h AS Holder) AS Holder
  RETURN h
END FUNC

FUNC identF(h AS FHolder) AS FHolder
  RETURN h
END FUNC

FUNC identL(h AS LateHolder) AS LateHolder
  RETURN h
END FUNC

FUNC wideSum(w AS Wide) AS Integer
  RETURN w.id + w.count + w.total + w.flags + math::floor(w.pos * 4.0) + math::floor(w.vel * 1024.0) + math::floor(w.energy) + len(w.name) * 3 + len(w.tag) * 5 + math::floor(w.weight * 8.0)
END FUNC

FUNC holders() AS List OF Holder
  MUT hs AS List OF Holder = []
  FOR k = 0 TO 63
    hs = collections::append(hs, Holder[id := k, label := \"holder-\" & toString(k), inner := newWide(k), touches := 0])
  NEXT
  RETURN hs
END FUNC

FUNC fholders() AS List OF FHolder
  MUT hs AS List OF FHolder = []
  FOR k = 0 TO 63
    hs = collections::append(hs, FHolder[label := \"f-\" & toString(k), inner := newWide(k), score := 0.25])
  NEXT
  RETURN hs
END FUNC

FUNC lateholders() AS List OF LateHolder
  MUT hs AS List OF LateHolder = []
  FOR k = 0 TO 63
    hs = collections::append(hs, LateHolder[inner := newWide(k), label := \"late-\" & toString(k), touches := 0])
  NEXT
  RETURN hs
END FUNC

FUNC sumHolders(hs AS List OF Holder) AS Integer
  MUT acc AS Integer = 0
  FOR EACH h IN hs
    acc = (acc * 31 + h.id + h.touches * 7 + len(h.label) + wideSum(h.inner)) MOD 1000000007
  NEXT
  RETURN acc
END FUNC

FUNC sumF(hs AS List OF FHolder) AS Integer
  MUT acc AS Integer = 0
  FOR EACH h IN hs
    acc = (acc * 31 + math::floor(h.score * 4.0) + len(h.label) + wideSum(h.inner)) MOD 1000000007
  NEXT
  RETURN acc
END FUNC

FUNC sumL(hs AS List OF LateHolder) AS Integer
  MUT acc AS Integer = 0
  FOR EACH h IN hs
    acc = (acc * 31 + h.touches * 7 + len(h.label) + wideSum(h.inner)) MOD 1000000007
  NEXT
  RETURN acc
END FUNC
";

type Case = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<u64>,
);

const CASES: &[Case] = &[
    (
        // The benchmark row: each element always sees the same tag (k and i share
        // parity), so after its first update `inner` keeps its size.
        "nested",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedsame",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedmut",
        "  MUT hs AS List OF Holder = holders()",
        "      MUT h AS Holder = collections::get(hs, k)\n      h = WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 }\n      hs = collections::set(hs, k, h)",
        "      MUT h AS Holder = ident(collections::get(hs, k))\n      h = WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 }\n      hs = collections::set(hs, k, h)",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedinner",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := WITH h.inner { count := h.inner.count + 1, pos := h.inner.pos + h.inner.vel }, touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := WITH h.inner { count := h.inner.count + 1, pos := h.inner.pos + h.inner.vel }, touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedinner_mut",
        "  MUT hs AS List OF Holder = holders()",
        "      MUT h AS Holder = collections::get(hs, k)\n      h = WITH h { touches := h.touches + h.inner.count, inner := WITH h.inner { count := h.inner.count + 1, total := h.touches } }\n      hs = collections::set(hs, k, h)",
        "      MUT h AS Holder = ident(collections::get(hs, k))\n      h = WITH h { touches := h.touches + h.inner.count, inner := WITH h.inner { count := h.inner.count + 1, total := h.touches } }\n      hs = collections::set(hs, k, h)",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        "nestedtouch",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        // The same element alternates between two tag lengths: every update
        // changes `inner`'s size and takes the size-change branch.
        "nested_resize",
        "  MUT hs AS List OF Holder = holders()",
        "      LET kk AS Integer = (i / 2) MOD 64\n      LET h AS Holder = collections::get(hs, kk)\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "      LET kk AS Integer = (i / 2) MOD 64\n      LET h AS Holder = ident(collections::get(hs, kk))\n      IF i MOD 2 = 0 THEN\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagEven), touches := h.touches + 1 })\n      ELSE\n        hs = collections::set(hs, kk, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })\n      END IF",
        "sumHolders(hs)",
        None,
    ),
    (
        // A `Float` scalar beside the replaced field, both branches.
        "float_scalar_resize",
        "  MUT hs AS List OF FHolder = fholders()",
        "      LET kk AS Integer = (i / 3) MOD 64\n      LET h AS FHolder = collections::get(hs, kk)\n      MUT t AS String = tagOdd\n      IF i MOD 3 = 0 THEN\n        t = tagEven\n      END IF\n      hs = collections::set(hs, kk, WITH h { score := h.score * 0.5 + toFloat(i MOD 5), inner := stepWide(h.inner, i, t) })",
        "      LET kk AS Integer = (i / 3) MOD 64\n      LET h AS FHolder = identF(collections::get(hs, kk))\n      MUT t AS String = tagOdd\n      IF i MOD 3 = 0 THEN\n        t = tagEven\n      END IF\n      hs = collections::set(hs, kk, WITH h { score := h.score * 0.5 + toFloat(i MOD 5), inner := stepWide(h.inner, i, t) })",
        "sumF(hs)",
        None,
    ),
    (
        // `inner` is not the last inlined field: a same-size `inner` still leaves
        // `label`'s offset where it was, so it is written in place too.
        "not_last_inlined",
        "  MUT hs AS List OF LateHolder = lateholders()",
        "      LET h AS LateHolder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "      LET h AS LateHolder = identL(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + 1 })",
        "sumL(hs)",
        Some(0),
    ),
    (
        // The callee reads the destination it writes: `total` reads the OLD
        // `count`, `tag` copies the destination's own `name` span.
        "overlap_callee_reads_destination",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepSelf(h.inner, i), touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepSelf(h.inner, i), touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        // The `tag` argument is a view into the destination itself (its `name`).
        "overlap_argument_views_destination",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.name), touches := h.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.name), touches := h.touches + 1 })",
        "sumHolders(hs)",
        Some(0),
    ),
    (
        // The `tag` argument is the very span the callee replaces.
        "overlap_argument_is_replaced_span",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.tag & \"x\"), touches := h.touches + 1 })\n      LET g AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH g { inner := stepWide(g.inner, i, g.inner.tag), touches := g.touches + 1 })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, h.inner.tag & \"x\"), touches := h.touches + 1 })\n      LET g AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH g { inner := stepWide(g.inner, i, g.inner.tag), touches := g.touches + 1 })",
        "sumHolders(hs)",
        None,
    ),
    (
        // A later update reads the destination: it must see the OLD `inner`, so
        // the destination variant is not used.
        "overlap_later_update_reads_destination",
        "  MUT hs AS List OF Holder = holders()",
        "      LET h AS Holder = collections::get(hs, k)\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + h.inner.count })",
        "      LET h AS Holder = ident(collections::get(hs, k))\n      hs = collections::set(hs, k, WITH h { inner := stepWide(h.inner, i, tagOdd), touches := h.touches + h.inner.count })",
        "sumHolders(hs)",
        Some(1),
    ),
];

fn program(setup: &str, body: &str, result: &str) -> String {
    format!(
        "{PRELUDE}
FUNC main() AS Integer
  LET tagEven AS String = \"even-step\"
  LET tagOdd AS String = \"odd\"
{setup}
  MUT i AS Integer = 0
  FOR rep = 1 TO 2
    FOR k = 0 TO 63
{body}
      i = i + 1
    NEXT
  NEXT
  io::print(toString({result}))
  RETURN 0
END FUNC
"
    )
}

fn stems(source: &str) -> Vec<String> {
    let plan = code_for_src_cached(source, CodeTarget::LinuxX86_64, Console);
    let main = code_function(plan, "main");
    let mut stems: Vec<String> = Stream::of(main)
        .labels()
        .into_iter()
        .map(|(_, name)| {
            name.trim_end_matches(|c: char| c.is_ascii_digit())
                .trim_end_matches('_')
                .to_string()
        })
        .collect();
    stems.sort();
    stems.dedup();
    stems
}

/// `(case, stems only the borrowing spelling emits, stems only the copying
/// spelling emits)`. A stem is a label with its serial dropped.
const SHAPES: &[(&str, &[&str], &[&str])] = &[
    (
        "nested",
        &[
            "element_replace_write_back",
            "element_replace_done",
            "field_replace_wloop",
            "into_result_kept",
        ],
        &[],
    ),
    (
        "nestedsame",
        &["element_replace_write_back", "into_result_kept"],
        &[],
    ),
    (
        "nestedmut",
        &[
            "element_replace_write_back",
            "into_result_kept",
            "record_inline_block_wloop",
        ],
        &["record_replace_rebuild", "record_replace_prefix_wloop"],
    ),
    // Scalars-only updates write the element's own bytes: no list rebuild at all.
    (
        "nestedinner",
        &[],
        &[
            "list_insert_alloc_ok",
            "list_remove_alloc_ok",
            "record_build_alloc_ok",
        ],
    ),
    (
        "nestedinner_mut",
        &[],
        &["list_insert_alloc_ok", "list_remove_alloc_ok"],
    ),
    (
        "nestedtouch",
        &[],
        &["list_insert_alloc_ok", "list_remove_alloc_ok"],
    ),
    (
        "nested_resize",
        &["element_replace_write_back", "into_result_kept"],
        &[],
    ),
    (
        "float_scalar_resize",
        &["element_replace_write_back", "into_result_kept"],
        &[],
    ),
    (
        "not_last_inlined",
        &["element_replace_write_back", "into_result_kept"],
        &[],
    ),
    (
        "overlap_callee_reads_destination",
        &["element_replace_write_back", "into_result_kept"],
        &[],
    ),
    (
        "overlap_argument_views_destination",
        &["element_replace_write_back", "into_result_kept"],
        &[],
    ),
    (
        "overlap_argument_is_replaced_span",
        &["element_replace_write_back", "into_result_kept"],
        &[],
    ),
    // A later update reads the destination, so the call's result block is kept
    // (no `into_result_kept` stem: the destination variant is not used).
    (
        "overlap_later_update_reads_destination",
        &["element_replace_write_back"],
        &[],
    ),
];

/// An update through a borrowed `get` binding lowers to the element-replace /
/// element-update forms; the same update through an owned copy takes the
/// general `WITH` and `set`. If the borrow analysis stopped recognising the
/// binding, the borrowing spelling would emit the contrast's stems and the
/// run-time allocation budget of the integration test would be lost silently.
#[test]
fn borrowed_element_updates_lower_to_the_element_forms_and_copies_do_not() {
    for (name, setup, body, contrast, result, _) in CASES {
        let (_, only_borrow, only_copy) = SHAPES
            .iter()
            .find(|(case, _, _)| case == name)
            .unwrap_or_else(|| panic!("no expected shape for case {name}"));
        let borrow = stems(&program(setup, body, result));
        let copy = stems(&program(setup, contrast, result));
        for stem in *only_borrow {
            assert!(
                borrow.iter().any(|s| s == stem),
                "{name}: borrowing spelling lacks {stem}"
            );
            assert!(
                !copy.iter().any(|s| s == stem),
                "{name}: copying spelling has {stem}"
            );
        }
        for stem in *only_copy {
            assert!(
                copy.iter().any(|s| s == stem),
                "{name}: copying spelling lacks {stem}"
            );
            assert!(
                !borrow.iter().any(|s| s == stem),
                "{name}: borrowing spelling has {stem}"
            );
        }
    }
}
