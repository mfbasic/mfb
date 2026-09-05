//! `canvas::present` deep-copies the scene and publishes it safely (plan-98-B
//! Phase 2).
//!
//! Two claims, and neither is observable by running a program: nothing can read
//! the published scene back until the renderer exists (plan-98-D), so a runtime
//! test can only show that `present` does not crash. These are therefore
//! build-only `--ncode` checks, the same shape the rest of the codegen-invariant
//! suite uses.
//!
//! What is actually being proven here:
//!
//! * **The scene is copied, not aliased.** The renderer reads the installed scene
//!   at arbitrary times after `present` returns, so a scene pointing at caller
//!   storage would be read after that storage was reused. `present` must allocate.
//!   The *transitivity* of the copy is inherited, not newly invented: an MFBASIC
//!   collection is a self-contained flat block, and `copy_flat_block` — the
//!   codebase's existing deep-copy primitive, shared with value-copy semantics and
//!   thread transfer — is what `present` calls.
//! * **The publish order cannot expose a half-written scene.** The revision is the
//!   field a reader gates on, so it must be stored *after* the pointer and the
//!   count. Storing it first would let a reader observe a bumped revision beside
//!   the previous frame's pointer.
//!
//! Plus one leak guard: the wrong-mode gate must precede the allocation, or every
//! `present` from the wrong mode would strand an arena block.

mod common;

use serde_json::Value;
use std::process::Command;

/// A `--app` `--ncode` build. `common::build_ncode` has no `-app`, and `canvas` is
/// importable only in app mode.
fn app_ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg("-ncode")
        .arg(&project)
        .output()
        .expect("run mfb build -app -ncode");
    assert!(
        output.status.success(),
        "mfb build -app -ncode failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let text =
        std::fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode dump");
    let plan: Value = serde_json::from_str(&text).expect("parse ncode json");
    let _ = std::fs::remove_dir_all(&project);
    plan
}

/// As `app_ncode`, without `-app`.
///
/// A program that does not import `canvas` cannot be built with `-app` — the `app`
/// package requires app mode — so the one test that checks what a *non*-canvas program
/// emits needs the plain form.
fn ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-ncode")
        .arg(&project)
        .output()
        .expect("run mfb build -ncode");
    assert!(
        output.status.success(),
        "mfb build -ncode failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let text =
        std::fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode dump");
    let plan: Value = serde_json::from_str(&text).expect("parse ncode json");
    let _ = std::fs::remove_dir_all(&project);
    plan
}

fn function<'a>(plan: &'a Value, symbol: &str) -> &'a Value {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))
}

fn instructions<'a>(plan: &'a Value, symbol: &str) -> &'a Vec<Value> {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
}

/// The index of the first instruction satisfying `pred`.
fn position(ins: &[Value], pred: impl Fn(&Value) -> bool) -> Option<usize> {
    ins.iter().position(pred)
}

fn calls(plan: &Value, symbol: &str, target: &str) -> usize {
    function(plan, symbol)["relocations"]
        .as_array()
        .map(|rs| {
            rs.iter()
                .filter(|r| r["to"].as_str() == Some(target))
                .count()
        })
        .unwrap_or(0)
}

/// The helper that does the publishing.
///
/// It was `_mfb_rt_canvas_canvas_present` when plan-98-B wrote these tests. plan-98-C
/// made `canvas::present` a `Body::mfb` member — `IF canvas::publishScene(items) THEN
/// render` — so the publish itself, and every property asserted below, moved into
/// `publishScene`. The behaviours are unchanged; only the function that carries them
/// is different.
const PUBLISH: &str = "_mfb_rt_canvas_canvas_publishScene";

/// A scene built entirely inside a callee's frame, so nothing it names outlives
/// the function that made it.
const SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     FUNC scene(RES f AS canvas::Font) AS List OF canvas::DrawItem\n\
    \x20 LET c AS canvas::Color = canvas::rgb(1, 2, 3)\n\
    \x20 LET pts AS List OF canvas::Point = [canvas::Point[x := 1.0, y := 2.0]]\n\
    \x20 LET a AS canvas::DrawItem = canvas::Polygon[points := pts, paint := canvas::fill(c)]\n\
    \x20 LET b AS canvas::DrawItem = canvas::Text[x := 0.0, y := 0.0, text := \"hi\", font := f, size := 9.0, paint := canvas::fill(c)]\n\
    \x20 RETURN [a, b]\n\
     END FUNC\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 RES fnt AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n\
    \x20   RETURN 1\n\
    \x20 END TRAP\n\
    \x20 canvas::present(scene(fnt))\n\
    \x20 RETURN 0\n\
     END FUNC\n";

/// `present` must allocate. Publishing the caller's pointer would be cheaper and
/// would pass every runtime test that exists today — and would hand the renderer a
/// pointer into storage the program is free to reuse the moment `present` returns.
#[test]
fn present_allocates_a_copy_of_the_scene() {
    let plan = app_ncode("canvas_present_copy", SOURCE);
    assert!(
        calls(&plan, PUBLISH, "_mfb_arena_alloc") > 0,
        "canvas::present must copy the scene into the arena; no allocation means \
         it published the caller's own block"
    );
}

/// The revision is what a reader gates on, so it must be written last. If it were
/// bumped before the pointer, a reader could see the new revision alongside the
/// previous frame's items.
#[test]
fn the_revision_is_published_after_the_items_and_count() {
    let plan = app_ncode("canvas_present_order", SOURCE);
    let ins = instructions(&plan, PUBLISH);
    assert_eq!(
        scene_stores(ins),
        vec![16, 8, 32, 40, 0],
        "publish order must be items(+16), count(+8), then the layered pair cleared \
         (+32, +40), then revision(+0) LAST — the revision is what a reader gates on, \
         so a reader must never see it bumped beside the previous frame's pointer"
    );
}

/// The offsets of every store into the scene region, in emitted order.
///
/// Anchored on the publish label rather than on a base register. The scene block is
/// process-global (plan-98-D Phase 2 — arena state is per-thread, so a graphics
/// thread could not see a scene published into it), so its address arrives in a vreg
/// that the allocator spills and reloads, giving a *different* physical base for each
/// store. Stack traffic is excluded by its `sp` base; everything left after the
/// publish label is a scene store.
/// The frame is excluded on BOTH spellings via `common::is_stack_base` — see its doc
/// comment for why one spelling is not enough. plan-116-H's box-2228 acceptance row is
/// what surfaced it: `removeGroup`'s first "store" read as `+176` on Linux x86-64, an
/// offset that cannot be a group-slot word, because a slot is 64 bytes and its words
/// live at 0..56.
fn scene_stores(ins: &[Value]) -> Vec<i64> {
    let publish =
        label_at(ins, "canvas_present_publish").expect("the publish path must have its own label");
    ins[publish..]
        .iter()
        .filter(|i| i["op"].as_str() == Some("str_u64") && !common::is_stack_base(i))
        .filter_map(|i| i["offset"].as_str().and_then(|o| o.parse::<i64>().ok()))
        // Only the LIVE scene fields. The publish path also writes the retirement
        // bookkeeping (48..72, plan-98-D Phase 3), which is not part of the scene a
        // reader sees and has no ordering requirement against the revision.
        .filter(|offset| PUBLISHED_OFFSETS.contains(offset))
        .collect()
}

/// The scene fields a reader observes: revision, count, items, layers, layerCount.
const PUBLISHED_OFFSETS: &[i64] = &[0, 8, 16, 32, 40];

/// The index of the first `label` instruction whose name contains `needle`.
fn label_at(ins: &[Value], needle: &str) -> Option<usize> {
    ins.iter().position(|i| {
        i["op"].as_str() == Some("label") && i["name"].as_str().is_some_and(|n| n.contains(needle))
    })
}

/// plan-98-B Phase 3: re-presenting identical content must publish nothing, so an
/// animation loop that redraws an unchanged frame does not make the renderer
/// redraw.
///
/// The skip must **bypass the revision bump**, which is the whole mechanism: the
/// revision is what a reader gates on, so a skip that still bumped it would be a
/// skip in name only. Asserting the revision store sits after the publish label —
/// and that the skip label and its `ret` come first — is what pins that.
#[test]
fn an_identical_re_present_skips_the_publish() {
    let plan = app_ncode("canvas_present_skip", SOURCE);
    let ins = instructions(&plan, PUBLISH);

    // It must read the currently-installed scene to have anything to compare
    // against, and compare it byte-wise (the loop the compare helper emits).
    assert!(
        label_at(ins, "canvas_present_same").is_some(),
        "present must byte-compare the new scene against the installed one"
    );

    let skip = label_at(ins, "canvas_present_skip").expect("a skip path");
    let publish = label_at(ins, "canvas_present_publish").expect("a publish path");
    assert!(
        skip < publish,
        "the skip path returns before the publish path"
    );

    // Every scene-region store must be past the publish label, so a skipped frame
    // publishes nothing at all — the revision included, which is the whole mechanism.
    assert_eq!(
        scene_stores(ins),
        vec![16, 8, 32, 40, 0],
        "the publish path must be the only thing that writes the scene region"
    );
    // And nothing writes it between the skip label and the publish label — that span
    // is the early return.
    let stray = ins[skip..publish].iter().any(|i| {
        i["op"].as_str() == Some("str_u64")
            && !common::is_stack_base(i)
            && i["offset"]
                .as_str()
                .and_then(|o| o.parse::<i64>().ok())
                .is_some_and(|offset| PUBLISHED_OFFSETS.contains(&offset))
    });
    assert!(
        !stray,
        "the skip path must not touch the scene region at all"
    );
}

/// The wrong-mode gate must come before the allocation. A gate placed after it
/// would strand an arena block on every `present` from the wrong mode — and the
/// program would still behave correctly, so nothing but this would catch it.
#[test]
fn the_mode_gate_precedes_the_allocation() {
    let plan = app_ncode("canvas_present_gate", SOURCE);
    let ins = instructions(&plan, PUBLISH);
    let gate = position(ins, |i| {
        i["op"].as_str() == Some("cmp_imm") && i["rhs"].as_str() == Some("2")
    })
    .expect("present must test the mode against Canvas (2)");
    let alloc = position(ins, |i| {
        i["op"].as_str() == Some("bl") && i["target"].as_str() == Some("_mfb_arena_alloc")
    })
    .expect("present must call the arena allocator");
    assert!(
        gate < alloc,
        "the Canvas mode gate (instruction {gate}) must precede the arena \
         allocation (instruction {alloc}), or a wrong-mode call leaks a block"
    );
    assert!(
        calls(&plan, PUBLISH, "_mfb_str_error_wrong_mode") > 0,
        "the gate must raise ErrWrongMode"
    );
}

const SET_GROUP: &str = "_mfb_rt_canvas_canvas_setGroup";

/// A program that installs a group and then removes one that was never installed.
const GROUP_SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 LET c AS canvas::Color = canvas::rgb(1, 2, 3)\n\
    \x20 LET a AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(c)]\n\
    \x20 canvas::setGroup(\"panel\", [a])\n\
    \x20 canvas::removeGroup(\"absent\")\n\
    \x20 RETURN 0\n\
     END FUNC\n";

/// `setGroup` must allocate — twice.
///
/// The items for the reason `present_allocates_a_copy_of_the_scene` gives: publishing
/// the caller's block would hand the graphics thread a pointer into storage the program
/// is free to reuse. And the **name**, which is the one a reader might not expect: the
/// table outlives the caller's binding, so a slot holding the caller's `String` would
/// leave the scan reading a name out of reclaimed memory.
///
/// Asserted as a count rather than "> 0" so that dropping either copy fails here, which
/// a single allocation would not.
#[test]
fn set_group_copies_both_the_items_and_the_name() {
    let plan = app_ncode("canvas_set_group_copy", GROUP_SOURCE);
    let allocs = calls(&plan, SET_GROUP, "_mfb_arena_alloc");
    assert!(
        allocs >= 2,
        "canvas::setGroup made {allocs} allocation(s); it must copy BOTH the item list \
         and the name — a slot pointing at either of the caller's blocks is a pointer \
         into storage the program may reuse or drop"
    );
}

/// The slot's `name` word is written **last** when a slot is published.
///
/// `name` is the discriminator: a graphics thread scanning the table treats a non-zero
/// name as "this slot is real, follow its pointers". So a slot must never be visible
/// under a name before its `items` and `count` are there. This is the same
/// publish-then-flag rule `the_revision_is_published_after_the_items_and_count` pins for
/// the scene region.
///
/// **Asserted as the ordering rather than as a literal sequence**, deliberately.
/// plan-116-G Phase 5 later inserted the retire-and-stamp stores ahead of the install,
/// which changed the exact list and broke a version of this test that pinned it —
/// correctly, but for a reason that had nothing to do with what the test protects. The
/// relations below hold under any addition that does not actually publish a name over a
/// half-written slot, and fail under one that does.
#[test]
fn a_group_slot_is_published_name_last() {
    let plan = app_ncode("canvas_set_group_order", GROUP_SOURCE);
    let ins = instructions(&plan, SET_GROUP);
    let install = label_at(ins, "canvas_set_group_install")
        .expect("the install path must have its own label");
    let offsets: Vec<i64> = ins[install..]
        .iter()
        .filter(|i| i["op"].as_str() == Some("str_u64") && !common::is_stack_base(i))
        .filter_map(|i| i["offset"].as_str().and_then(|o| o.parse::<i64>().ok()))
        .collect();

    assert_eq!(
        offsets.last(),
        Some(&0),
        "the LAST store on the install path must be the name (+0); it is what a scanning          graphics thread gates on, so anything written after it is written into a slot          that is already live. Stores were {offsets:?}",
    );
    // `position` finds the FIRST occurrence, which is the conservative one here: the
    // retire clears `items` before the install writes it, so requiring "some items store
    // precedes the name" would be satisfied by the clear alone. Requiring the LAST items
    // store to precede the name is the real claim.
    let name_at = offsets.iter().rposition(|&o| o == 0).expect("a name store");
    for (word, label) in [(8i64, "items"), (16, "count"), (24, "revision")] {
        let at = offsets
            .iter()
            .rposition(|&o| o == word)
            .unwrap_or_else(|| panic!("no {label} store on the install path: {offsets:?}"));
        assert!(
            at < name_at,
            "`{label}` (+{word}) is written after the name (+0), so a slot becomes              visible under its name before {label} is in it: {offsets:?}",
        );
    }
}

/// `removeGroup` clears the name **first**, and retires the buffer rather than freeing
/// the one it is removing.
///
/// Name first because it is the discriminator: clearing it is what makes the slot
/// invisible to a concurrent scan, and it has to happen before anything else about the
/// slot changes. The buffer then moves to the retired word — a frame may be mid-copy of
/// it, so the release waits for the drain gate (plan-116-G Phase 5, §13 of
/// `.ai/canvas-threading.md`).
///
/// The "frees nothing" half is now stated as *what it does with the live buffer* rather
/// than as an arena-free count. `emit_retire_current_items` legitimately frees a
/// **prior** retired buffer when one is still present, so counting frees would forbid
/// something correct; what must never happen is the block being removed going straight
/// to a free, and the store to `RETIRED_ITEMS` is the positive evidence that it does not.
#[test]
fn remove_group_clears_the_name_first_and_retires_the_buffer() {
    let plan = app_ncode("canvas_remove_group_order", GROUP_SOURCE);
    let ins = instructions(&plan, "_mfb_rt_canvas_canvas_removeGroup");
    let done =
        label_at(ins, "canvas_remove_group_scan_done").expect("the scan must end at its own label");
    let offsets: Vec<i64> = ins[done..]
        .iter()
        .filter(|i| i["op"].as_str() == Some("str_u64") && !common::is_stack_base(i))
        .filter_map(|i| i["offset"].as_str().and_then(|o| o.parse::<i64>().ok()))
        .collect();

    assert_eq!(
        offsets.first(),
        Some(&0),
        "the FIRST store after the scan must clear the name (+0). It is the          discriminator, so until it is zero a concurrent scan still treats the slot as          live — and everything after this point is changing that slot. Stores were          {offsets:?}",
    );
    assert!(
        offsets.contains(&48),
        "the live items block was never stored to RETIRED_ITEMS (+48), so `removeGroup`          did not retire it. Either it leaked the block or it freed it outright — and          freeing it here races a graphics thread that may be mid-copy: {offsets:?}",
    );
    let retired_at = offsets.iter().position(|&o| o == 48).unwrap();
    let items_cleared = offsets
        .iter()
        .position(|&o| o == 8)
        .unwrap_or_else(|| panic!("the live `items` word (+8) was never cleared: {offsets:?}"));
    assert!(
        retired_at < items_cleared,
        "`items` (+8) was cleared before it was copied to RETIRED_ITEMS (+48), so the          buffer was lost rather than retired: {offsets:?}",
    );
}

/// The group table is emitted only for a program that uses `canvas`.
///
/// `CANVAS_MAX_GROUPS` is 256 slots of 64 bytes plus a header — 16,392 bytes of literal
/// zeroes in the data section. That size was chosen deliberately (plan-116-G's **G4**)
/// on the argument that it is carried only by canvas programs, and this is the assertion
/// that makes the argument true rather than intended: the data object sits behind
/// `module_uses_canvas(module)` in `engine/builder/mod.rs`, beside the scene region and
/// the font table.
///
/// Worth pinning rather than assuming because the gate is one `if` around three pushes,
/// and a fourth added outside it would be invisible — every canvas test would still
/// pass, and every program in the language would grow by 16 KB.
#[test]
fn the_group_table_is_absent_from_a_program_that_does_not_use_canvas() {
    const PLAIN: &str = "IMPORT io\n\
         FUNC main AS Integer\n\
        \x20 io::print(\"no canvas here\")\n\
        \x20 RETURN 0\n\
         END FUNC\n";
    let plan = ncode("canvas_group_table_gate", PLAIN);
    let text = plan.to_string();
    for symbol in [
        "_mfb_rt_canvas_groups",
        "_mfb_rt_canvas_scene",
        "_mfb_rt_canvas_fonts",
    ] {
        assert!(
            !text.contains(symbol),
            "`{symbol}` was emitted into a program that never mentions canvas. The three \
             canvas globals share one `module_uses_canvas` gate, so a data object added \
             outside it grows every program in the language — and no canvas test would \
             notice.",
        );
    }
}
