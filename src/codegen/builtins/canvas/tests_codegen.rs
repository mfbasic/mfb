//! Codegen contracts for the canvas scene publish (`gen_present.rs`) and for the
//! scene's addressing (`scene_base.rs`).
//!
//! These lower a real `canvas::present` / `canvas::presentLayers` program **in
//! process** (`testutil::app_code_cached`) and inspect the emitted stream. The
//! `tests/` canvas suites cannot stand in for this: they shell out to a
//! separately built `target/release/mfb`, so nothing they run is observable from
//! the test process, and the publish body's ordering rules are exactly the kind
//! that a passing end-to-end run does not notice — a reader that observes a
//! bumped revision beside a half-written scene renders a torn frame *sometimes*.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::{CodeFunction, CodeInstruction, NativeCodePlan};
use crate::codegen::error::constants::*;
use crate::testutil::{app_code_cached, code_function, CodeTarget};

/// A program that publishes both scene shapes, so one compile covers both bodies.
///
/// It has to be an `-app` build: `app::setMode(Canvas)` lowers to the host
/// toolkit's event loop, which only the app build mode declares an import for.
const PRESENT_SRC: &str = "\
IMPORT app
IMPORT canvas

FUNC scene(r AS Float) AS List OF canvas::DrawItem
  LET c AS canvas::Color = canvas::rgb(10, 20, 30)
  LET a AS canvas::DrawItem = canvas::Circle[x := 1.0, y := 2.0, radius := r, paint := canvas::fill(c)]
  RETURN [a]
END FUNC

FUNC layered(r AS Float) AS List OF canvas::DrawLayer
  RETURN [canvas::DrawLayer[items := scene(r)]]
END FUNC

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  canvas::present(scene(5.0))
  canvas::presentLayers(layered(5.0))
  RETURN 0
END FUNC
";

fn program() -> &'static NativeCodePlan {
    app_code_cached(PRESENT_SRC, CodeTarget::LinuxX86_64)
}

/// The lowered body of `canvas::present`.
fn publish_scene() -> &'static CodeFunction {
    code_function(program(), "runtime.canvas.publishScene")
}

/// The lowered body of `canvas::presentLayers`.
fn publish_layers() -> &'static CodeFunction {
    code_function(program(), "runtime.canvas.publishLayers")
}

fn field(i: &CodeInstruction, name: &str) -> String {
    i.get(name).unwrap_or_default()
}

/// The stack slot the scene base is parked in.
///
/// `scene_base` materializes the process-global with an `adrp`/`add` pair naming
/// [`CANVAS_SCENE_SYMBOL`] and the register allocator spills the result; every
/// later scene access reloads it from that one slot. Finding the slot from the
/// symbol (rather than hardcoding an offset) is what keeps these tests from
/// going stale the moment a frame grows — a hardcoded slot number turns a
/// harmless layout change into a red test that says nothing.
fn scene_slot(f: &CodeFunction) -> String {
    let mut holder: Option<String> = None;
    for i in &f.instructions {
        if i.op == CodeOp::AddPageOff && field(i, "symbol") == CANVAS_SCENE_SYMBOL {
            holder = Some(field(i, "dst"));
        }
        if i.op == CodeOp::StrU64 && holder.as_deref() == Some(field(i, "src").as_str()) {
            return field(i, "offset");
        }
    }
    panic!(
        "no spill of the `{CANVAS_SCENE_SYMBOL}` address found in {}",
        f.name
    )
}

/// Every store **into the scene region**, in emission order, as scene offsets.
///
/// A store is attributed to the scene when its base register was last loaded
/// from the scene's own spill slot. Filtering by offset alone cannot work: the
/// frame's own slots are numbered from 0 as well, so `str [rsp+16]` and
/// `str [scene+16]` are indistinguishable without tracking the base.
fn scene_stores(f: &CodeFunction) -> Vec<(usize, String)> {
    let slot = scene_slot(f);
    let mut holders: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for i in &f.instructions {
        match i.op {
            CodeOp::LdrU64 => {
                let dst = field(i, "dst");
                holders.retain(|r| *r != dst);
                if field(i, "base") == "rsp" && field(i, "offset") == slot {
                    holders.push(dst);
                }
            }
            CodeOp::StrU64 if holders.contains(&field(i, "base")) => {
                let offset: usize = field(i, "offset").parse().expect("numeric store offset");
                out.push((offset, field(i, "src")));
            }
            // Any other definition of a register invalidates it as a scene base.
            _ => {
                let dst = field(i, "dst");
                if !dst.is_empty() {
                    holders.retain(|r| *r != dst);
                }
            }
        }
    }
    out
}

fn label_index(f: &CodeFunction, name: &str) -> usize {
    f.instructions
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no label `{name}` in {}", f.name))
}

// --- scene_base.rs --------------------------------------------------------

/// The scene is a **process-global**, not arena state.
///
/// `.ai/canvas-threading.md` §2: arena state is per-thread and in an `--app`
/// build the *worker* runs the entry, so a scene published into arena state is
/// invisible to the graphics thread — it would render blank frames forever, and
/// a blank frame is a legal frame, so nothing would report it. The only
/// mechanical evidence that the scene is not in arena state is that every access
/// goes through a relocation against the global symbol.
#[test]
fn the_scene_is_addressed_through_the_process_global_symbol() {
    for f in [publish_scene(), publish_layers()] {
        let addressed = f
            .relocations
            .iter()
            .filter(|r| r.to == CANVAS_SCENE_SYMBOL)
            .count();
        assert!(
            addressed >= 2,
            "{} must address the scene through `{CANVAS_SCENE_SYMBOL}` (an adrp/add \
             relocation pair), not through arena state; found {addressed} relocations",
            f.name
        );
        assert!(
            !scene_stores(f).is_empty(),
            "{} must write the scene region",
            f.name
        );
    }
}

// --- gen_present.rs -------------------------------------------------------

/// The revision is written **last**, after every other scene store.
///
/// It is the word a reader gates on, so publishing it before the pointers and
/// counts are in place makes a half-written scene observable. Nothing about a
/// torn frame is deterministic, so only the store order proves it.
#[test]
fn the_revision_is_the_final_scene_store() {
    for f in [publish_scene(), publish_layers()] {
        let stores = scene_stores(f);
        let (last_offset, _) = *stores.last().expect("the publish writes the scene");
        assert_eq!(
            last_offset,
            CANVAS_SCENE_REVISION_OFFSET,
            "{}: the revision (offset {CANVAS_SCENE_REVISION_OFFSET}) must be the LAST \
             scene store, or a reader can see a bumped revision beside a half-written \
             scene. Order was {:?}",
            f.name,
            stores.iter().map(|(o, _)| *o).collect::<Vec<_>>()
        );
        assert_eq!(
            stores
                .iter()
                .filter(|(o, _)| *o == CANVAS_SCENE_REVISION_OFFSET)
                .count(),
            1,
            "{}: the revision is bumped exactly once per publish",
            f.name
        );
    }
}

/// Publishing one shape clears the other shape's pointer *and* its count.
///
/// A reader decides which shape is installed with one test (`layers != 0`), so a
/// stale pointer left in the other pair would make it render the previous
/// shape's block — freed or not — instead of the scene just published.
#[test]
fn publishing_one_shape_zeroes_the_other_shapes_pair() {
    let cases = [
        (
            publish_scene(),
            (CANVAS_SCENE_ITEMS_OFFSET, CANVAS_SCENE_COUNT_OFFSET),
            (CANVAS_SCENE_LAYERS_OFFSET, CANVAS_SCENE_LAYER_COUNT_OFFSET),
        ),
        (
            publish_layers(),
            (CANVAS_SCENE_LAYERS_OFFSET, CANVAS_SCENE_LAYER_COUNT_OFFSET),
            (CANVAS_SCENE_ITEMS_OFFSET, CANVAS_SCENE_COUNT_OFFSET),
        ),
    ];
    for (f, (live_ptr, live_count), (dead_ptr, dead_count)) in cases {
        let stores = scene_stores(f);
        let value_at = |offset: usize| -> Vec<&str> {
            stores
                .iter()
                .filter(|(o, _)| *o == offset)
                .map(|(_, src)| src.as_str())
                .collect()
        };
        for offset in [dead_ptr, dead_count] {
            let written = value_at(offset);
            assert!(
                !written.is_empty() && written.iter().all(|src| *src == "xzr"),
                "{}: offset {offset} (the other shape's pair) must be zeroed on every \
                 publish; it was written with {written:?}",
                f.name
            );
        }
        for offset in [live_ptr, live_count] {
            let written = value_at(offset);
            assert!(
                written.iter().any(|src| *src != "xzr"),
                "{}: offset {offset} must receive the published block, not zero",
                f.name
            );
        }
    }
}

/// This body's `<tag>_<kind>_<n>` label.
///
/// The trailing number is `CodeBuilder::label`'s per-function counter, so it is
/// not stable across an edit to any earlier label in the same body — matching on
/// the prefix is what keeps these tests from going red for a renumbering.
fn tagged_label(f: &CodeFunction, kind: &str) -> String {
    let tag = if f.name.contains("Layers") {
        format!("canvas_present_layers_{kind}")
    } else {
        format!("canvas_present_{kind}")
    };
    f.instructions
        .iter()
        .filter(|i| i.op == CodeOp::Label)
        .filter_map(|i| i.get("name"))
        .find(|n| n.starts_with(&tag))
        .unwrap_or_else(|| panic!("no `{tag}*` label in {}", f.name))
}

/// This body's publish label.
fn publish_label(f: &CodeFunction) -> String {
    tagged_label(f, "publish")
}

/// The two bodies tag their labels apart, so a program using both assembles.
///
/// `SceneShape::tag` exists only for this; a shared prefix would emit two
/// definitions of the same label in one program and fail at assembly time — but
/// only for a program that calls *both* calls, which is why it needs its own test.
#[test]
fn the_two_publish_bodies_do_not_share_label_names() {
    let flat: Vec<String> = body_labels(publish_scene());
    let layered: Vec<String> = body_labels(publish_layers());
    let shared: Vec<&String> = flat.iter().filter(|l| layered.contains(l)).collect();
    assert!(
        shared.is_empty(),
        "canvas::present and canvas::presentLayers must not emit the same label \
         (a program calling both would define it twice); shared: {shared:?}"
    );
}

fn body_labels(f: &CodeFunction) -> Vec<String> {
    f.instructions
        .iter()
        .filter(|i| i.op == CodeOp::Label)
        .filter_map(|i| i.get("name"))
        .filter(|n| n.starts_with("canvas_present"))
        .collect()
}

/// The skip returns FALSE and the publish returns TRUE.
///
/// The caller gates its render on this, so inverting it renders every skipped
/// frame and skips every changed one.
#[test]
fn the_skip_reports_false_and_the_publish_reports_true() {
    for f in [publish_scene(), publish_layers()] {
        let skip = label_index(f, &tagged_label(f, "skip"));
        let publish = label_index(f, &publish_label(f));
        assert!(
            skip < publish,
            "{}: the skip exit precedes the publish",
            f.name
        );
        let (skip_reg, skip_value) = result_value_after(f, skip);
        let (publish_reg, publish_value) = result_value_after(f, publish);
        // The same register in both exits, so the pair cannot pass by having one
        // exit set the *tag* register to the value this test wants to see.
        assert_eq!(
            skip_reg, publish_reg,
            "{}: both exits must report through the same result-value register",
            f.name
        );
        assert_eq!(
            skip_value, "0",
            "{}: the frame-skip exit must report FALSE, or the caller re-renders \
             every skipped frame and the skip buys nothing",
            f.name
        );
        assert_eq!(
            publish_value, "1",
            "{}: the publish must report TRUE so the caller renders the scene it \
             just installed",
            f.name
        );
    }
}

/// The `(register, value)` an exit beginning at `from` returns.
///
/// Read positionally rather than by naming `RESULT_VALUE_REGISTER`: that
/// constant is a neutral role token (`abi::mfb_return(1)`) and the stream under
/// test is post-register-allocation, where it has already been realized to the
/// backend's physical register. An exit sets the value register and then the tag
/// register as the last thing it does before its epilogue, so the pair is the
/// final two immediates ahead of the `Ret` this label reaches — not the first
/// two after the label, which for the publish exit are the reclaim's constants.
fn result_value_after(f: &CodeFunction, from: usize) -> (String, String) {
    let ret = f.instructions[from..]
        .iter()
        .position(|i| i.op == CodeOp::Ret)
        .map(|n| from + n)
        .unwrap_or_else(|| panic!("no `ret` after index {from} in {}", f.name));
    let mut immediates: Vec<&CodeInstruction> = f.instructions[from..ret]
        .iter()
        .filter(|i| i.op == CodeOp::MovImm)
        .collect();
    let tag = immediates.pop();
    let value = immediates
        .pop()
        .unwrap_or_else(|| panic!("exit at {from} in {} sets no result pair", f.name));
    assert_eq!(
        tag.map(|i| field(i, "value")),
        Some(RESULT_OK_TAG.to_string()),
        "{}: the exit at {from} must return the OK tag",
        f.name
    );
    (field(value, "dst"), field(value, "value"))
}

/// The wrong-mode gate runs before anything is allocated.
///
/// `prepend_wrong_mode_gate` splices the check in above the manual prologue
/// precisely so a call in the wrong mode returns without allocating. Emitting it
/// anywhere else leaks one deep copy of the scene per wrong-mode call.
#[test]
fn the_wrong_mode_gate_precedes_the_scene_copys_allocation() {
    for f in [publish_scene(), publish_layers()] {
        let alloc = f
            .instructions
            .iter()
            .position(|i| {
                i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some("_mfb_arena_alloc")
            })
            .unwrap_or_else(|| panic!("{} must allocate the scene copy", f.name));
        let gate = f
            .instructions
            .iter()
            .position(|i| {
                i.op == CodeOp::Label && i.get("name").is_some_and(|n| n.ends_with("_mode_ok"))
            })
            .unwrap_or_else(|| panic!("{} must carry a wrong-mode gate", f.name));
        assert!(
            gate < alloc,
            "{}: the wrong-mode gate must be spliced in above the allocation \
             (gate at {gate}, arena_alloc at {alloc}), or every wrong-mode call \
             leaks one deep copy of the scene",
            f.name
        );
    }
}

/// Retired blocks are freed only once a frame has completed since retirement.
///
/// The gate is `frame_now > retired_frame`, emitted as "branch away when
/// `frame_now <= retired_frame`". Relaxing it by one — `branch away when <` —
/// frees a block the graphics thread may still be reading this very frame, which
/// is a use-after-free that only shows up under load.
#[test]
fn retired_scene_blocks_are_freed_only_after_a_frame_completes() {
    for f in [publish_scene(), publish_layers()] {
        let done = f
            .instructions
            .iter()
            .enumerate()
            .find(|(_, i)| {
                i.op == CodeOp::BranchLs
                    && i.get("target")
                        .is_some_and(|t| t.starts_with("canvas_reclaim_done"))
            })
            .map(|(n, _)| n)
            .unwrap_or_else(|| {
                panic!(
                    "{}: the reclaim must skip on `frame_now <= retired_frame` \
                     (an unsigned lower-or-same branch)",
                    f.name
                )
            });
        let frees_after = f.instructions[done..]
            .iter()
            .filter(|i| {
                i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some("_mfb_arena_free")
            })
            .count();
        assert_eq!(
            frees_after, 3,
            "{}: all three retired blocks (items, hashes, layers) must be freed \
             behind the frame gate",
            f.name
        );
        assert!(
            !f.instructions[..done].iter().any(|i| {
                i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some("_mfb_arena_free")
            }),
            "{}: nothing may be freed before the frame gate — the graphics thread \
             may still be reading it",
            f.name
        );
    }
}

/// The retired slots are stamped with the frame counter on every publish.
///
/// Without the stamp the gate compares against whatever was there, and the
/// reclaim either never fires (a 200-frame animation grew ~0.11 MB a frame) or
/// fires immediately (use-after-free).
#[test]
fn every_publish_retires_the_displaced_blocks_and_stamps_the_frame() {
    for f in [publish_scene(), publish_layers()] {
        let offsets: Vec<usize> = scene_stores(f).into_iter().map(|(o, _)| o).collect();
        for retired in [
            CANVAS_SCENE_RETIRED_ITEMS_OFFSET,
            CANVAS_SCENE_RETIRED_HASHES_OFFSET,
            CANVAS_SCENE_RETIRED_LAYERS_OFFSET,
            CANVAS_SCENE_RETIRED_FRAME_OFFSET,
        ] {
            assert!(
                offsets.contains(&retired),
                "{}: offset {retired} must be written on every publish; wrote {offsets:?}",
                f.name
            );
        }
        let stamp = offsets
            .iter()
            .position(|o| *o == CANVAS_SCENE_RETIRED_FRAME_OFFSET)
            .expect("the frame stamp");
        let live = offsets
            .iter()
            .position(|o| *o == CANVAS_SCENE_REVISION_OFFSET)
            .expect("the revision");
        assert!(
            stamp < live,
            "{}: the displaced blocks are retired and stamped BEFORE the new scene \
             is published",
            f.name
        );
    }
}
