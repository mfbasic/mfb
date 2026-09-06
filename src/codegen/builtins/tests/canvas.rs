//! Codegen contracts for the canvas scene publish (`gen_present.rs`) and for the
//! scene's addressing (`scene_base.rs`).
//!
//! These lower a real `canvas::present` / `canvas::presentLayers` program **in
//! process** (`testutil::app_code_cached`) and inspect the emitted stream. The
//! `tests/` canvas suites cannot stand in for this: they shell out to a
//! separately built `target/release/mfb`, so nothing they run is observable from
//! the test process, and the publish body's ordering rules are exactly the kind
//! a passing end-to-end run does not notice — a reader that observes a bumped
//! revision beside a half-written scene renders a torn frame *sometimes*.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::{CodeFunction, NativeCodePlan};
use crate::codegen::error::constants::*;
use crate::testutil::{app_code_cached, code_function, CodeTarget};

/// A program that publishes both scene shapes, so one compile covers both bodies.
///
/// It has to be an `-app` build: `app::setMode(Canvas)` lowers to the host
/// toolkit's event loop, which only the app build mode declares an import for.
const PRESENT_SRC: &str = "\
IMPORT app
IMPORT canvas
IMPORT color

FUNC scene(r AS Float) AS List OF canvas::DrawItem
  LET c AS color::Color = color::rgb(10, 20, 30)
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

/// Both publish bodies, since every rule below holds for each.
fn both() -> [&'static CodeFunction; 2] {
    [publish_scene(), publish_layers()]
}

/// This body's `<tag>_<kind>_<n>` label.
fn tagged_label(f: &CodeFunction, kind: &str) -> String {
    let tag = if f.name.contains("Layers") {
        format!("canvas_present_layers_{kind}")
    } else {
        format!("canvas_present_{kind}")
    };
    Stream::of(f).label_starting(&tag)
}

/// The stack slot the scene base is parked in.
///
/// `scene_base` materializes the process-global with an `adrp`/`add` pair naming
/// [`CANVAS_SCENE_SYMBOL`] and the register allocator spills the result; every
/// later scene access reloads it from that one slot. Finding the slot from the
/// symbol rather than hardcoding an offset is what keeps these tests from going
/// stale the moment a frame grows — a hardcoded slot number turns a harmless
/// layout change into a red test that says nothing.
fn scene_slot(f: &CodeFunction) -> String {
    let stream = Stream::of(f);
    let materialized = stream.index_of(
        &format!("`add` completing the `{CANVAS_SCENE_SYMBOL}` address"),
        |i| i.op == CodeOp::AddPageOff && Stream::field(i, "symbol") == CANVAS_SCENE_SYMBOL,
    );
    let holder = Stream::field(&f.instructions[materialized], "dst");
    let spill = stream.index_after(
        materialized,
        &format!("spill of the scene base (`{holder}`)"),
        |i| i.op == CodeOp::StrU64 && Stream::field(i, "src") == holder,
    );
    Stream::field(&f.instructions[spill], "offset")
}

/// Every store **into the scene region**, in emission order, as scene offsets.
///
/// A store is attributed to the scene when its base register was last loaded
/// from the scene's own spill slot. Filtering by offset alone cannot work: the
/// frame's own slots are numbered from 0 as well, so `str [rsp+16]` and
/// `str [scene+16]` are otherwise indistinguishable.
fn scene_stores(f: &CodeFunction) -> Vec<(usize, String)> {
    let slot = scene_slot(f);
    let mut holders: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for i in &f.instructions {
        match i.op {
            CodeOp::LdrU64 => {
                let dst = Stream::field(i, "dst");
                holders.retain(|r| *r != dst);
                if Stream::field(i, "base") == "rsp" && Stream::field(i, "offset") == slot {
                    holders.push(dst);
                }
            }
            CodeOp::StrU64 if holders.contains(&Stream::field(i, "base")) => {
                let offset: usize = Stream::field(i, "offset")
                    .parse()
                    .expect("a store offset is numeric");
                out.push((offset, Stream::field(i, "src")));
            }
            // Any other definition of a register invalidates it as a scene base.
            _ => {
                let dst = Stream::field(i, "dst");
                if !dst.is_empty() {
                    holders.retain(|r| *r != dst);
                }
            }
        }
    }
    out
}

/// The scene offsets written, in order.
fn scene_store_offsets(f: &CodeFunction) -> Vec<usize> {
    scene_stores(f).into_iter().map(|(o, _)| o).collect()
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
    for f in both() {
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
    for f in both() {
        let offsets = scene_store_offsets(f);
        assert_eq!(
            offsets.last().copied(),
            Some(CANVAS_SCENE_REVISION_OFFSET),
            "{}: the revision (offset {CANVAS_SCENE_REVISION_OFFSET}) must be the LAST \
             scene store, or a reader can see a bumped revision beside a half-written \
             scene. Order was {offsets:?}",
            f.name
        );
        assert_eq!(
            offsets
                .iter()
                .filter(|o| **o == CANVAS_SCENE_REVISION_OFFSET)
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
    let flat = (CANVAS_SCENE_ITEMS_OFFSET, CANVAS_SCENE_COUNT_OFFSET);
    let layered = (CANVAS_SCENE_LAYERS_OFFSET, CANVAS_SCENE_LAYER_COUNT_OFFSET);
    for (f, (live_ptr, live_count), (dead_ptr, dead_count)) in [
        (publish_scene(), flat, layered),
        (publish_layers(), layered, flat),
    ] {
        let stores = scene_stores(f);
        let written = |offset: usize| -> Vec<&str> {
            stores
                .iter()
                .filter(|(o, _)| *o == offset)
                .map(|(_, src)| src.as_str())
                .collect()
        };
        for offset in [dead_ptr, dead_count] {
            let sources = written(offset);
            assert!(
                !sources.is_empty() && sources.iter().all(|src| *src == "xzr"),
                "{}: offset {offset} (the other shape's pair) must be zeroed on every \
                 publish; it was written with {sources:?}",
                f.name
            );
        }
        for offset in [live_ptr, live_count] {
            let sources = written(offset);
            assert!(
                sources.iter().any(|src| *src != "xzr"),
                "{}: offset {offset} must receive the published block, not zero",
                f.name
            );
        }
    }
}

/// The two bodies tag their labels apart, so a program using both assembles.
///
/// `SceneShape::tag` exists only for this; a shared prefix would emit two
/// definitions of the same label in one program — but only for a program that
/// calls *both* calls, which is why it needs its own test.
#[test]
fn the_two_publish_bodies_do_not_share_label_names() {
    let flat = body_labels(publish_scene());
    let layered = body_labels(publish_layers());
    let shared: Vec<&String> = flat.iter().filter(|l| layered.contains(l)).collect();
    assert!(
        shared.is_empty(),
        "canvas::present and canvas::presentLayers must not emit the same label \
         (a program calling both would define it twice); shared: {shared:?}"
    );
}

fn body_labels(f: &CodeFunction) -> Vec<String> {
    Stream::of(f)
        .labels()
        .into_iter()
        .map(|(_, name)| name)
        .filter(|n| n.starts_with("canvas_present"))
        .collect()
}

/// The skip returns FALSE and the publish returns TRUE.
///
/// The caller gates its render on this, so inverting it renders every skipped
/// frame and skips every changed one.
#[test]
fn the_skip_reports_false_and_the_publish_reports_true() {
    for f in both() {
        let stream = Stream::of(f);
        let skip = stream.label_at(&tagged_label(f, "skip"));
        let publish = stream.label_at(&tagged_label(f, "publish"));
        assert!(
            skip < publish,
            "{}: the skip exit precedes the publish",
            f.name
        );
        let (skip_reg, skip_value) = exit_result(f, skip);
        let (publish_reg, publish_value) = exit_result(f, publish);
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
/// register as the last thing before its epilogue, so the pair is the final two
/// immediates ahead of the `ret` this label reaches — not the first two after
/// the label, which for the publish exit are the reclaim's constants.
fn exit_result(f: &CodeFunction, from: usize) -> (String, String) {
    let stream = Stream::of(f);
    let ret = stream.index_after(from, "`ret` closing this exit", |i| i.op == CodeOp::Ret);
    let mut immediates: Vec<&_> = f.instructions[from..ret]
        .iter()
        .filter(|i| i.op == CodeOp::MovImm)
        .collect();
    let tag = immediates.pop();
    let value = immediates.pop();
    assert_eq!(
        tag.map(|i| Stream::field(i, "value")),
        Some(RESULT_OK_TAG.to_string()),
        "{}: the exit at {from} must return the OK tag",
        f.name
    );
    let value = value.expect("an exit sets a result value before its tag");
    (Stream::field(value, "dst"), Stream::field(value, "value"))
}

/// The wrong-mode gate runs before anything is allocated.
///
/// `prepend_wrong_mode_gate` splices the check in above the manual prologue
/// precisely so a call in the wrong mode returns without allocating. Emitting it
/// anywhere else leaks one deep copy of the scene per wrong-mode call.
#[test]
fn the_wrong_mode_gate_precedes_the_scene_copys_allocation() {
    for f in both() {
        let stream = Stream::of(f);
        let alloc = stream.index_of("call to the arena allocator", |i| {
            i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some("_mfb_arena_alloc")
        });
        let gate = stream.index_of("wrong-mode gate (a `*_mode_ok` label)", |i| {
            i.op == CodeOp::Label && i.get("name").is_some_and(|n| n.ends_with("_mode_ok"))
        });
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
/// `frame_now <= retired_frame`". Relaxing it by one — branch away only when
/// strictly less — frees a block the graphics thread may still be reading this
/// very frame, a use-after-free that only shows up under load.
#[test]
fn retired_scene_blocks_are_freed_only_after_a_frame_completes() {
    for f in both() {
        let stream = Stream::of(f);
        let gate = stream.index_of(
            "reclaim gate (an unsigned lower-or-same branch to `canvas_reclaim_done*`)",
            |i| {
                i.op == CodeOp::BranchLs
                    && i.get("target")
                        .is_some_and(|t| t.starts_with("canvas_reclaim_done"))
            },
        );
        let frees = |from: usize, to: usize| {
            stream
                .calls_between(from, to)
                .into_iter()
                .filter(|t| t == "_mfb_arena_free")
                .count()
        };
        assert_eq!(
            frees(gate, f.instructions.len()),
            3,
            "{}: all three retired blocks (items, hashes, layers) must be freed \
             behind the frame gate",
            f.name
        );
        assert_eq!(
            frees(0, gate),
            0,
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
    for f in both() {
        let offsets = scene_store_offsets(f);
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
            .position(|o| *o == CANVAS_SCENE_RETIRED_FRAME_OFFSET);
        let revision = offsets
            .iter()
            .position(|o| *o == CANVAS_SCENE_REVISION_OFFSET);
        assert!(
            stamp < revision,
            "{}: the displaced blocks are retired and stamped BEFORE the new scene \
             is published (stamp at {stamp:?}, revision at {revision:?})",
            f.name
        );
    }
}

/// The canvas program lowers on every backend that has an `-app` mode.
///
/// Everything above inspects the linux-x86_64 lowering, because the publish
/// body's ordering rules are arch-neutral. The rest of the canvas surface is
/// not: `canvas::getSize`, `setBytes`, `blitSurface`, the Metal seam and the
/// `term::` shared helpers each `match` on the platform family, and the arms
/// none of them takes on Linux are the ones nobody has ever lowered in process.
///
/// The contract is the same one the corpus states for console programs and the
/// acceptance matrix states slowly for app ones: every backend emits the whole
/// program. A canvas member that lowers on Linux and not on macOS is a build
/// failure nobody sees until a release runner reaches it.
#[test]
fn the_canvas_program_lowers_on_every_app_capable_backend() {
    let mut lowered = 0;
    let mut agreed: Option<Vec<String>> = None;
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let plan = app_code_cached(PRESENT_SRC, target);
        lowered += 1;
        let publish: Vec<String> = plan
            .functions
            .iter()
            .map(|f| f.name.clone())
            .filter(|name| name.starts_with("runtime.canvas."))
            .collect();
        assert!(
            publish.contains(&"runtime.canvas.publishScene".to_string()),
            "{}: the canvas runtime must emit publishScene",
            target.name()
        );
        let mut sorted = publish;
        sorted.sort();
        match &agreed {
            None => agreed = Some(sorted),
            Some(first) => assert_eq!(
                first,
                &sorted,
                "{}: the canvas runtime surface differs from the first backend's; \
                 a member emitted on one target and not another is a build failure \
                 nobody sees until a release runner reaches it",
                target.name()
            ),
        }
    }
    assert_eq!(
        lowered, 4,
        "four of the five backends have an -app mode (rv64 is console-only)"
    );
}
