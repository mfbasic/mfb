//! Regression test for the thread-transfer deep copy of a **non-flat `Map`/`Set`**
//! (found while reproducing bug-538).
//!
//! `copy_collection_to_current_arena` sized the destination block by hand as
//! `HEADER + capacity*ENTRY + dataCapacity` and left out a map's or set's
//! hash-bucket region — the `capacity << 4` bytes `emit_reserve_map_buckets` adds
//! to every other allocation path, and that the single authority
//! (`emit_inlined_block_size_from_ptr_slot`) has always included. The whole source
//! block was then byte-copied over it, so the destination inherited
//! `BUCKETS_READY = 1` while owning no bucket region at all: the first probe read
//! past the block, and the lazy `build_buckets` rebuild WROTE past it. That is
//! bug-02's failure mode (`regex prog.names` corrupting the arena free list),
//! re-instantiated in the transfer copier.
//!
//! Only a **non-flat** map reaches that code — a flat one is copied by
//! `copy_flat_block`/`copy_collection_tight`, which reserve the region and mark it
//! not-ready — so the shape needs a map whose value type is recursive. Measured
//! before the fix (`mfb` at main `7b0f93c08`): the program below printed
//! `v={u:{},v:sC,}`, silently losing the whole inner map; the same program under
//! `json::Json` made `json::get` answer `{}` for a key that was present.
//!
//! Uses a REAL transfer of a real recursive type, not a proxy: the value is read
//! back after `thread::waitFor`, when the worker arena is gone, so a short block
//! or a stale index is observable rather than benign.

mod common;

use std::process::Command;

const SRC: &str = "IMPORT io\n\
IMPORT collections\n\
IMPORT thread\n\
TYPE ObjNode\n  fields AS Map OF String TO Tree\nEND TYPE\n\
TYPE LeafNode\n  text AS String\nEND TYPE\n\
UNION Tree\n  ObjNode\n  LeafNode\nEND UNION\n\
ISOLATED FUNC build(w AS ThreadWorker OF String TO Tree, seed AS String) AS Tree\n\
  LET inner AS Tree = ObjNode[Map OF String TO Tree { \"n\" := LeafNode[seed & \"A\"], \"m\" := LeafNode[seed & \"B\"] }]\n\
  RETURN ObjNode[Map OF String TO Tree { \"u\" := inner, \"v\" := LeafNode[seed & \"C\"] }]\n\
END FUNC\n\
FUNC show(t AS Tree) AS String\n\
  MATCH t\n\
    CASE LeafNode(l)\n      RETURN l.text\n\
    CASE ObjNode(o)\n\
      MUT out AS String = \"{\"\n\
      FOR EACH k IN collections::keys(o.fields)\n\
        out = out & k & \":\" & show(collections::get(o.fields, k)) & \",\"\n\
      NEXT\n\
      RETURN out & \"}\"\n\
  END MATCH\n\
END FUNC\n\
FUNC main AS Integer\n\
  LET t AS Thread OF String TO Tree = thread::start(build, \"s\")\n\
  LET v AS Tree = thread::waitFor(t)\n\
  io::print(\"v=\" & show(v))\n\
  ' A second, INDEPENDENT transfer of the same shape: the first copy must not\n\
  ' have written over anything the second one needs.\n\
  LET t2 AS Thread OF String TO Tree = thread::start(build, \"z\")\n\
  LET v2 AS Tree = thread::waitFor(t2)\n\
  io::print(\"v2=\" & show(v2))\n\
  io::print(\"v-again=\" & show(v))\n\
  RETURN 0\n\
END FUNC\n";

#[test]
fn a_transferred_non_flat_map_keeps_every_entry_probeable() {
    let project = common::temp_project("recursive_map_transfer", SRC);
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run built program");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the transferred recursive tree must be readable:\n{text}"
    );
    // `collections::keys` preserves insertion order, so the rendering is exact.
    assert!(
        text.contains("v={u:{n:sA,m:sB,},v:sC,}"),
        "the inner map lost entries across the transfer (the bucket region was \
         not allocated, so the probe read past the block):\n{text}"
    );
    assert!(
        text.contains("v2={u:{n:zA,m:zB,},v:zC,}"),
        "a second transfer must copy the same shape correctly:\n{text}"
    );
    assert!(
        text.contains("v-again={u:{n:sA,m:sB,},v:sC,}"),
        "the first transferred value must survive the second transfer — a short \
         block whose lazy bucket rebuild wrote past it would corrupt a \
         neighbour:\n{text}"
    );
}
