//! plan-145-E: a reallocation inside a field self-update reallocates the RECORD.
//!
//! `filter` over a variable-width list may repack it (`emit_repack_list_data`,
//! the compaction's out-of-order path), and `union` grows its set
//! (`emit_map_reserve`, `lower_map_set_in_place`). On a plain local the new block
//! replaces the list's own. At a record's last inlined field the list is bytes
//! inside the record block, so the allocation must be the whole record: the
//! prefix `[0, fieldOffset)` copied verbatim (`emit_inline_grow_split`'s
//! `inline_grow_prefix` copy), the collection built at the sub-block, and the old
//! RECORD freed. A lowering that freed the sub-block address instead would hand
//! `arena_free` a pointer into the middle of a live allocation.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

const FIELD: &str = "\
IMPORT collections
IMPORT io

TYPE Box
  n AS Integer
  words AS List OF String
END TYPE

TYPE Tags
  n AS Integer
  s AS Set OF String
END TYPE

FUNC keep(w AS String) AS Boolean
  RETURN len(w) < 3
END FUNC

FUNC main() AS Integer
  MUT b AS Box = Box[n := 1, words := [\"a\", \"bbbb\", \"cc\"]]
  b = WITH b { words := collections::filter(b.words, keep) }
  MUT t AS Tags = Tags[n := 1, s := Set OF String { \"a\" }]
  LET more AS Set OF String = Set OF String { \"b\", \"c\" }
  t = WITH t { s := collections::union(t.s, more) }
  io::print(toString(len(b.words)) & toString(len(t.s)))
  RETURN 0
END FUNC
";

const LOCAL: &str = "\
IMPORT collections
IMPORT io

FUNC keep(w AS String) AS Boolean
  RETURN len(w) < 3
END FUNC

FUNC main() AS Integer
  MUT words AS List OF String = [\"a\", \"bbbb\", \"cc\"]
  words = collections::filter(words, keep)
  MUT s AS Set OF String = Set OF String { \"a\" }
  LET more AS Set OF String = Set OF String { \"b\", \"c\" }
  s = collections::union(s, more)
  io::print(toString(len(words)) & toString(len(s)))
  RETURN 0
END FUNC
";

/// How many labels of `main` start with `stem`.
fn labels_named(source: &str, stem: &str) -> usize {
    let plan = code_for_src_cached(source, CodeTarget::LinuxX86_64, Console);
    let main = code_function(plan, "main");
    Stream::of(main)
        .labels()
        .into_iter()
        .filter(|(_, name)| name.starts_with(stem))
        .count()
}

#[test]
fn an_inline_repack_copies_the_record_prefix_and_a_plain_one_does_not() {
    // Both field routes ran in place (their arms' labels), and each reallocation
    // they can reach splits the new RECORD block: the filter's repack, the
    // union's reserve, and every insert's grow.
    assert!(
        labels_named(FIELD, "inplace_filter_loop") > 0,
        "the field filter must run its in-place arm"
    );
    assert!(
        labels_named(FIELD, "set_repack_copy_loop") > 0,
        "the field filter must reach the repack"
    );
    let prefix = labels_named(FIELD, "inline_grow_prefix");
    assert!(
        prefix >= 3,
        "each reallocation at the field (the repack, the reserve, the insert grows) \
         must copy the record prefix; found {prefix} prefix copies"
    );
    // The same operations on plain locals reallocate the collection itself.
    assert_eq!(
        labels_named(LOCAL, "inline_grow_prefix"),
        0,
        "a plain local's reallocation has no record prefix to copy"
    );
    assert!(labels_named(LOCAL, "set_repack_copy_loop") > 0);
}
