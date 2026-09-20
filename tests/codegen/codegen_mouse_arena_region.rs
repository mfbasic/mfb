//! plan-94-A Phase 2: the mouse-state arena region is **appended**, so no existing
//! arena-state offset moves.
//!
//! The arena-state chain is a sequence of conditionally-reserved regions: the
//! program's globals and `LINK`/`FREE` slots, then `term::` state when the program
//! uses `term::`, then the `app::` presentation-mode word when it uses `app::`, and
//! now the mouse-state region when it uses `enableMouse`/`pollMouse`. Every region
//! is addressed as a fixed byte offset off the pinned arena-state register, so
//! inserting the new one anywhere but the END would silently move
//! `term_state_offset` and `presentation_mode_offset` for every program that uses
//! both `term::` and `app::` — renumbering slots that dozens of hand-written
//! emitters address by constant, and churning every golden for programs that never
//! mention the mouse.
//!
//! Appending is therefore the design (plan-94-A §4.4a, and the rejected alternative
//! of growing `TERM_STATE_SLOTS`), and this is the check that it stays the design.
//! Two otherwise-identical `--app` programs are compiled — one using `term::` and
//! `app::`, one the same plus a single `term::pollMouse()` — and the entry frame's
//! arena-state zero-init is compared. The assertions are:
//!
//!   1. The non-mouse program's zero-store offsets are a strict PREFIX of the mouse
//!      program's. This is the real claim: not merely "the frame grew", but "every
//!      slot that existed before is at the same byte offset it was at before".
//!   2. The frame grows by exactly `MOUSE_STATE_SLOTS * 8` — the region is reserved
//!      once, at its declared size, with nothing rounded in or padded out.
//!   3. `_mfb_rt_mouse_mode` — the process-global mode word a UI-thread handler
//!      reads (§4.4b) — is emitted for the mouse program and ABSENT from the
//!      non-mouse one, so a program that never asks for mouse keeps its exact
//!      data-object set.
//!
//! No mouse input is decoded at this point (plan-94-A lands inert stubs); this is
//! purely about where the storage B, C, D and E will use has been put.
//!
//! macOS is the target because `--app` + `app::setMode` is what forces all three
//! earlier regions to be present at once, which is the arrangement the appending
//! claim is actually about.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, temp_project};

use serde_json::Value;

/// Slots in the mouse-state region — mirrors `MOUSE_STATE_SLOTS`
/// (`src/codegen/error/constants/error_constants.rs`): ring pointer, head, tail,
/// parse length, then the 32-byte partial-sequence buffer.
const MOUSE_STATE_SLOTS: usize = 8;

/// Uses `term::` and `app::`, so BOTH the term-state region and the
/// presentation-mode word are reserved — the arrangement a mouse region has to
/// append past without disturbing.
const WITHOUT_MOUSE: &str = "\
IMPORT app\n\
IMPORT io\n\
IMPORT term\n\
\n\
FUNC main AS Integer\n\
  app::setMode(app::Mode.Console)\n\
  term::on()\n\
  term::drawText(0, 0, \"hi\")\n\
  term::sync()\n\
  term::off()\n\
  io::print(\"done\")\n\
  RETURN 0\n\
END FUNC\n";

/// Byte-for-byte the same program plus one `term::pollMouse()`. The single added
/// call is what must move the frame — and nothing else.
const WITH_MOUSE: &str = "\
IMPORT app\n\
IMPORT io\n\
IMPORT term\n\
\n\
FUNC main AS Integer\n\
  app::setMode(app::Mode.Console)\n\
  term::on()\n\
  term::drawText(0, 0, \"hi\")\n\
  term::sync()\n\
  term::off()\n\
  io::print(\"done\")\n\
  LET event AS term::MouseEvent = term::pollMouse()\n\
  RETURN 0\n\
END FUNC\n";

/// The program entry's `(frame bytes, arena-state zero-store offsets)`.
///
/// The entry reserves its frame with one `sub_sp`, pins the arena base in `x19`,
/// zeroes the fixed arena-state header with a loop, and then zeroes each reserved
/// global slot with an explicit `str_u64 xzr, [x19 + K]`. Those explicit stores are
/// the region chain made visible: one per slot, in offset order.
fn entry_frame(source: &str, name: &str) -> (usize, Vec<usize>) {
    let project = temp_project(name, source);
    let ncode = build_ncode_app(&project, name);
    let entry = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some("_mfb_macapp_program"))
        .expect("ncode has the macOS app worker program entry");
    let instructions = entry["instructions"]
        .as_array()
        .expect("program entry has instructions");

    let frame = instructions
        .iter()
        .find(|i| i["op"].as_str() == Some("sub_sp"))
        .and_then(|i| i["imm"].as_str())
        .and_then(|s| s.parse::<usize>().ok())
        .expect("program entry reserves its frame with sub_sp");

    let zero_stores = instructions
        .iter()
        .filter(|i| {
            i["op"].as_str() == Some("str_u64")
                && i["src"].as_str() == Some("xzr")
                && i["base"].as_str() == Some("x19")
        })
        .filter_map(|i| i["offset"].as_str().and_then(|s| s.parse::<usize>().ok()))
        .collect();

    (frame, zero_stores)
}

/// `build_ncode` with `-app`: `app::` refuses to compile in a console build, and the
/// whole point of this fixture is having the presentation-mode word present.
fn build_ncode_app(project: &std::path::Path, name: &str) -> Value {
    // `build_ncode` does not pass `-app`, so drive the binary directly with the
    // same contract (dump beside the project, parse it).
    let output = std::process::Command::new(common::mfb_exe())
        .arg("build")
        .arg("-ncode")
        .arg("-app")
        .arg("-target")
        .arg("macos-aarch64")
        .arg(project)
        .output()
        .expect("run mfb build -ncode -app");
    assert!(
        output.status.success(),
        "mfb build -ncode -app failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let path = project.join(format!("{name}.ncode"));
    let text = std::fs::read_to_string(&path).expect("read ncode dump");
    serde_json::from_str(&text).expect("parse ncode json")
}

fn mouse_mode_globals(source: &str, name: &str) -> Vec<String> {
    let project = temp_project(name, source);
    let ncode = build_ncode_app(&project, name);
    ncode["dataObjects"]
        .as_array()
        .expect("ncode has a dataObjects array")
        .iter()
        .filter_map(|o| o["symbol"].as_str())
        .filter(|s| s.contains("mouse"))
        .map(str::to_string)
        .collect()
}

#[test]
fn mouse_region_is_appended_and_moves_no_existing_arena_offset() {
    let (plain_frame, plain_slots) = entry_frame(WITHOUT_MOUSE, "mouse_arena_plain");
    let (mouse_frame, mouse_slots) = entry_frame(WITH_MOUSE, "mouse_arena_mouse");

    assert!(
        !plain_slots.is_empty(),
        "the control program should reserve arena-state slots; found none, so this \
         test is measuring nothing"
    );

    // (1) THE claim: every pre-existing slot keeps its exact byte offset. A prefix
    // check is stronger than comparing counts — it fails if the new region were
    // inserted before or between the existing ones, which is precisely the mistake
    // that would renumber `term_state_offset` / `presentation_mode_offset`.
    assert_eq!(
        &mouse_slots[..plain_slots.len()],
        &plain_slots[..],
        "adding term::pollMouse moved an existing arena-state slot.\n  \
         without mouse: {plain_slots:?}\n  with mouse:    {mouse_slots:?}\n\
         The mouse region must be APPENDED past the presentation-mode word \
         (plan-94-A §4.4a)."
    );

    // (2) Exactly one region of exactly the declared size is added.
    assert_eq!(
        mouse_slots.len() - plain_slots.len(),
        MOUSE_STATE_SLOTS,
        "expected exactly MOUSE_STATE_SLOTS new zero-initialized slots"
    );
    assert_eq!(
        mouse_frame - plain_frame,
        MOUSE_STATE_SLOTS * 8,
        "entry frame should grow by exactly the mouse region's size"
    );

    // And they are genuinely past everything, not merely equal in count.
    assert!(
        mouse_slots[plain_slots.len()] > *plain_slots.last().expect("non-empty"),
        "the first mouse slot must sit past the last pre-existing slot"
    );
}

#[test]
fn mouse_mode_word_is_emitted_only_for_a_mouse_program() {
    assert_eq!(
        mouse_mode_globals(WITHOUT_MOUSE, "mouse_global_plain"),
        Vec::<String>::new(),
        "a program that never mentions the mouse must not carry the mode word"
    );
    assert_eq!(
        mouse_mode_globals(WITH_MOUSE, "mouse_global_mouse"),
        vec!["_mfb_rt_mouse_mode".to_string()],
        "a mouse program must carry the process-global mode word (plan-94-A §4.4b)"
    );
}
