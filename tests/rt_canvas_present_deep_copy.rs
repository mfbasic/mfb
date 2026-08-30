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

/// The pinned arena-state register the scene region is addressed off.
const ARENA_STATE_REGISTER: &str = "x19";

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

const PRESENT: &str = "_mfb_rt_canvas_canvas_present";

/// A scene built entirely inside a callee's frame, so nothing it names outlives
/// the function that made it.
const SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     FUNC scene() AS List OF DrawItem\n\
    \x20 LET c AS Color = canvas::rgb(1, 2, 3)\n\
    \x20 LET pts AS List OF Point = [Point[x := 1.0, y := 2.0]]\n\
    \x20 LET a AS DrawItem = Polygon[points := pts, paint := canvas::fill(c)]\n\
    \x20 LET b AS DrawItem = Text[x := 0.0, y := 0.0, text := \"hi\", font := FontRef[id := 1], size := 9.0, paint := canvas::fill(c)]\n\
    \x20 RETURN [a, b]\n\
     END FUNC\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(Mode.Canvas)\n\
    \x20 canvas::present(scene())\n\
    \x20 RETURN 0\n\
     END FUNC\n";

/// `present` must allocate. Publishing the caller's pointer would be cheaper and
/// would pass every runtime test that exists today — and would hand the renderer a
/// pointer into storage the program is free to reuse the moment `present` returns.
#[test]
fn present_allocates_a_copy_of_the_scene() {
    let plan = app_ncode("canvas_present_copy", SOURCE);
    assert!(
        calls(&plan, PRESENT, "_mfb_arena_alloc") > 0,
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
    let ins = instructions(&plan, PRESENT);
    let scene_stores: Vec<i64> = ins
        .iter()
        .filter(|i| {
            i["op"].as_str() == Some("str_u64") && i["base"].as_str() == Some(ARENA_STATE_REGISTER)
        })
        .filter_map(|i| i["offset"].as_str().and_then(|o| o.parse::<i64>().ok()))
        .collect();
    assert!(
        scene_stores.len() >= 3,
        "expected the three scene publishes (items, count, revision); got \
         {scene_stores:?}"
    );
    let tail = &scene_stores[scene_stores.len() - 3..];
    let base = tail[2];
    assert_eq!(
        tail,
        [base + 16, base + 8, base],
        "publish order must be items(+16), count(+8), revision(+0) — the revision \
         last, since it is what a reader gates on"
    );
}

/// The wrong-mode gate must come before the allocation. A gate placed after it
/// would strand an arena block on every `present` from the wrong mode — and the
/// program would still behave correctly, so nothing but this would catch it.
#[test]
fn the_mode_gate_precedes_the_allocation() {
    let plan = app_ncode("canvas_present_gate", SOURCE);
    let ins = instructions(&plan, PRESENT);
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
        calls(&plan, PRESENT, "_mfb_str_error_wrong_mode") > 0,
        "the gate must raise ErrWrongMode"
    );
}
