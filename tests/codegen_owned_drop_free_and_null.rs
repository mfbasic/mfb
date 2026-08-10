//! bug-440: an owned flat value dropped by `emit_owned_value_drop` must be
//! FREED-AND-NULLED — the cleanup slot is zeroed right after `_mfb_arena_free` —
//! so a drop re-reached without an intervening store (a loop body whose owned
//! temp came from a short-circuit-evaluated initializer, e.g. a record-returning
//! call in a `WHILE` condition) sees 0 and skips instead of double-freeing the
//! stale pointer. The once-only prologue zero-init is not enough across loop
//! iterations; the second free is a non-immediate double-free that corrupts the
//! arena free-list and any live block that reused the freed one.
//!
//! The runtime symptom (garbage read back from a corrupted sibling record) is
//! entropy-scrub-dependent and so flaky, but the codegen fix is deterministic:
//! every `owned_value_free_skip_*` cleanup ends with a zero-store to its slot
//! before the skip label. This inspects the `.ncode` dump for that invariant,
//! which fails loudly if the free-and-null is ever dropped (regressing the
//! double-free). Pinned to `macos-aarch64` so the zero register (`xzr`) and the
//! dump shape are deterministic regardless of the host running the test.

mod common;
use common::{build_ncode, temp_project};

// A record-returning helper called in a WHILE condition creates an owned record
// temp whose drop lives in the (loop-nested) inner-while scope — the exact shape
// that double-freed before the fix. No `term` needed: the buggy cleanup is the
// owned-value drop itself, independent of what the loop body does.
const SOURCE: &str = "\
TYPE TStyle\n\
  bold AS Boolean\n\
  underline AS Boolean\n\
END TYPE\n\
\n\
FUNC styleAt(index AS Integer) AS TStyle\n\
  IF index = 1 THEN\n\
    RETURN TStyle[TRUE, FALSE]\n\
  END IF\n\
  RETURN TStyle[FALSE, FALSE]\n\
END FUNC\n\
\n\
FUNC main AS Integer\n\
  MUT sink AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 3\n\
    LET st AS TStyle = styleAt(i)\n\
    MUT j AS Integer = i + 1\n\
    WHILE j < 3 AND styleAt(j).bold = st.bold AND styleAt(j).underline = st.underline\n\
      j = j + 1\n\
    END WHILE\n\
    IF st.bold THEN\n\
      sink = sink + 1\n\
    END IF\n\
    i = j\n\
  END WHILE\n\
  RETURN sink\n\
END FUNC\n";

#[test]
fn owned_value_drop_frees_and_nulls_the_slot() {
    let project = temp_project("codegen_owned_drop_free_and_null", SOURCE);
    let ncode = build_ncode(&project, "macos-aarch64", "codegen_owned_drop_free_and_null");

    let functions = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array");

    let mut cleanup_labels = 0usize;
    for func in functions {
        let insts = match func["instructions"].as_array() {
            Some(insts) => insts,
            None => continue,
        };
        for (idx, inst) in insts.iter().enumerate() {
            let is_cleanup_label = inst["op"].as_str() == Some("label")
                && inst["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("owned_value_free_skip"));
            if !is_cleanup_label {
                continue;
            }
            cleanup_labels += 1;
            // The freed path falls through to the skip label, so the instruction
            // right before the label is the free-and-null zero-store (the null-guard
            // path branched straight to the label without freeing). Before the fix
            // this slot was the `bl _mfb_arena_free` itself — no zeroing.
            let prev = insts
                .get(idx - 1)
                .unwrap_or_else(|| panic!("cleanup label at index 0 in {}", func["name"]));
            let op = prev["op"].as_str().unwrap_or("");
            assert_ne!(
                op, "bl",
                "owned-value drop in `{}` frees without zeroing its slot (bug-440 \
                 double-free regression): {} is preceded by {}",
                func["name"], inst, prev
            );
            assert_eq!(
                op, "str_u64",
                "expected a free-and-null zero-store before {}, got {}",
                inst, prev
            );
            assert_eq!(
                prev["src"].as_str(),
                Some("xzr"),
                "the store before {} should null the slot with xzr, got {}",
                inst,
                prev
            );
            assert_eq!(
                prev["base"].as_str(),
                Some("sp"),
                "the free-and-null store should target a stack slot, got {}",
                prev
            );
        }
    }

    assert!(
        cleanup_labels > 0,
        "no owned_value_free_skip cleanups found — the fixture no longer exercises \
         emit_owned_value_drop, so this test would vacuously pass"
    );
}
