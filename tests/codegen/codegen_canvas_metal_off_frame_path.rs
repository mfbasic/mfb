//! bug-1087: the Metal pipeline is never on a frame's critical path (bug-996's rule,
//! extended to Metal by the owner's ruling of 2026-10-17).
//!
//! macOS compiles the canvas shader from MSL the first time an application name asks
//! for it, and keeps the result per application: 4.7 s headless and 5.6 to 13 s in a
//! window for an app name never run before (`tools/canvas-bench/bench.sh <mfb>
//! coldstart`). The render loop used to ask `canvas::metalReady()` inside the first
//! frame, and that call was the build: nothing was drawn until it returned.
//!
//! The rule now, as for Vulkan: asking whether the pipeline is ready calls nothing, and
//! the build is `canvas::metalBuild`, which runs on a thread of its own unless the
//! program asked for the GPU by name (`MFB_CANVAS_GPU`), where it runs on the caller as
//! it always did.
//!
//! ## Why this runs the emitted code rather than a program
//!
//! A headless run (every cargo test) is software unless `MFB_CANVAS_GPU` is set, and a
//! set variable builds on the graphics thread, so no headless program reaches the
//! on-a-thread build; a window does, and opening windows is not a test's to do. So, as
//! `codegen_canvas_vulkan_off_frame_path.rs` does for Vulkan, these run the functions a
//! macOS app build emits, in the `-ncode` interpreter, against a model of the thread
//! library and Metal: what the readiness question calls, when the thread is created,
//! and what is in the ready word when it is.

#[path = "../common/mod.rs"]
mod common;

use common::ncode_emu::{Externals, Machine};
use common::{build_ncode_app, temp_project};
use serde_json::Value;
use std::sync::OnceLock;

const SOURCE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 400.0], radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([dot])
END FUNC
"#;

const TARGET: &str = "macos-aarch64";

const GRAPHICS_STATE: &str = "_mfb_rt_canvas_graphics";
/// `GRAPHICS_OFFSET_MTL_READY` (`src/codegen/runtime/canvas/mod.rs`).
const READY: u64 = 368;
const UNTRIED: u64 = 0;
const BUILT: u64 = 1;
const FAILED: u64 = 2;
const BUILDING: u64 = 3;

const IS_READY: &str = "_mfb_rt_canvas_canvas_metalReady";
const BUILD: &str = "_mfb_rt_canvas_canvas_metalBuild";
const BUILDER_ENTRY: &str = "_mfb_macapp_metal_builder";

/// `canvas::metalBuild`'s argument: where the build runs.
const ON_ITS_OWN_THREAD: u64 = 0;
const ON_THIS_THREAD: u64 = 1;

fn ncode() -> &'static Value {
    static PLAN: OnceLock<Value> = OnceLock::new();
    PLAN.get_or_init(|| {
        let project = temp_project("canvas_metal_off_frame", SOURCE);
        let ncode = build_ncode_app(&project, TARGET, "canvas_metal_off_frame");
        let _ = std::fs::remove_dir_all(&project);
        ncode
    })
}

/// Where an MFBASIC runtime member leaves its `Boolean` answer.
fn answer(machine: &Machine) -> u64 {
    machine.reg("x1")
}

fn ready_word(machine: &mut Machine) -> u64 {
    let state = machine.address_of(GRAPHICS_STATE);
    machine.read_u64(state + READY)
}

fn set_ready_word(machine: &mut Machine, value: u64) {
    let state = machine.address_of(GRAPHICS_STATE);
    machine.write_uint(state + READY, 8, value);
}

/// A platform that answers nothing: any call at all fails the run, naming it.
struct NoPlatform;

impl Externals for NoPlatform {
    fn call(&mut self, _machine: &mut Machine, _name: &str) -> Option<u64> {
        None
    }
}

/// The thread library and Metal, as far as starting the build reaches them. This Mac
/// has no Metal device (`MTLCreateSystemDefaultDevice` answers nil), so a build that
/// runs ends "failed".
#[derive(Default)]
struct Platform {
    /// Whether the thread library can start a thread.
    threads_fail: bool,
    /// The start routine each created thread was given, and the ready word at that
    /// moment.
    created: Vec<(String, u64)>,
    /// Threads given back to the system (`pthread_detach`).
    released: Vec<u64>,
    /// Device probes (`MTLCreateSystemDefaultDevice`), each answered nil.
    probes: usize,
    /// Autorelease pools pushed and popped.
    pools: (usize, usize),
}

const THREAD: u64 = 0x7ead;
const POOL: u64 = 0x9001;

impl Externals for Platform {
    fn call(&mut self, m: &mut Machine, name: &str) -> Option<u64> {
        match name.trim_start_matches('_') {
            "pthread_create" => {
                let routine = m.symbol_at(m.arg(2)).unwrap_or("?").to_string();
                let ready = ready_word(m);
                self.created.push((routine, ready));
                if self.threads_fail {
                    return Some(35); // EAGAIN on Darwin
                }
                m.write_uint(m.arg(0), 8, THREAD);
                Some(0)
            }
            "pthread_detach" => {
                self.released.push(m.arg(0));
                Some(0)
            }
            "MTLCreateSystemDefaultDevice" => {
                self.probes += 1;
                Some(0)
            }
            "objc_autoreleasePoolPush" => {
                self.pools.0 += 1;
                Some(POOL)
            }
            "objc_autoreleasePoolPop" => {
                assert_eq!(m.arg(0), POOL, "the pool popped is not the one pushed");
                self.pools.1 += 1;
                Some(0)
            }
            _ => None,
        }
    }
}

/// The frame path's question. It is asked on every frame the GPU is selected for and,
/// in a `--debug` build, by every stats line — so whatever it calls is on the frame
/// path. It must call nothing and change nothing, whatever state the build is in.
#[test]
fn asking_whether_the_metal_pipeline_is_ready_calls_nothing() {
    for (state, expected) in [(UNTRIED, 0), (BUILDING, 0), (FAILED, 0), (BUILT, 1)] {
        let mut machine = Machine::new(ncode());
        set_ready_word(&mut machine, state);
        machine.call(IS_READY, &[], &mut NoPlatform);
        assert_eq!(
            answer(&machine),
            expected,
            "canvas::metalReady() with the ready word at {state}"
        );
        assert_eq!(
            ready_word(&mut machine),
            state,
            "the question changed the ready word from {state}"
        );
    }
}

/// Starting the build claims it (so no second thread is ever started), creates one
/// thread whose start routine is the builder's entry, and hands the thread back to the
/// system: nobody joins it, because nothing may wait for a shader compile.
#[test]
fn the_metal_build_starts_on_a_thread_of_its_own() {
    let mut machine = Machine::new(ncode());
    let mut platform = Platform::default();
    machine.call(BUILD, &[ON_ITS_OWN_THREAD], &mut platform);
    assert_eq!(
        platform.created,
        vec![(BUILDER_ENTRY.to_string(), BUILDING)],
        "one thread, entered at the builder, created after the claim"
    );
    assert_eq!(platform.released, vec![THREAD]);
    assert_eq!(platform.probes, 0, "the caller probed for a device");
    assert_eq!(ready_word(&mut machine), BUILDING);

    // Asked again while it runs, and again after it finished either way.
    for state in [BUILDING, BUILT, FAILED] {
        set_ready_word(&mut machine, state);
        machine.call(BUILD, &[ON_ITS_OWN_THREAD], &mut NoPlatform);
        machine.call(BUILD, &[ON_THIS_THREAD], &mut NoPlatform);
        assert_eq!(ready_word(&mut machine), state);
    }
}

/// No thread, no GPU: the frames stay software, as with no Metal device at all.
#[test]
fn a_metal_builder_that_cannot_be_started_is_a_failed_build() {
    let mut machine = Machine::new(ncode());
    let mut platform = Platform {
        threads_fail: true,
        ..Platform::default()
    };
    machine.call(BUILD, &[ON_ITS_OWN_THREAD], &mut platform);
    assert_eq!(platform.created.len(), 1);
    assert!(platform.released.is_empty());
    assert_eq!(ready_word(&mut machine), FAILED);
    machine.call(IS_READY, &[], &mut NoPlatform);
    assert_eq!(answer(&machine), 0);
}

/// `MFB_CANVAS_GPU` set: the build runs on the caller, to its end, as it always did.
/// Here there is no Metal device, so its end is "failed".
#[test]
fn asked_for_by_name_the_metal_build_runs_on_the_calling_thread() {
    let mut machine = Machine::new(ncode());
    let mut platform = Platform::default();
    machine.call(BUILD, &[ON_THIS_THREAD], &mut platform);
    assert!(platform.created.is_empty(), "a thread was started");
    assert_eq!(platform.probes, 1);
    assert_eq!(ready_word(&mut machine), FAILED);
}

/// The builder thread's whole life: build inside its own autorelease pool, publish the
/// answer, return. A machine with no Metal device publishes "failed", and the thread
/// library gets back every register it keeps across the start routine.
#[test]
fn the_metal_builder_thread_publishes_its_answer_and_returns() {
    let mut machine = Machine::new(ncode());
    let kept = [
        "x19", "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28", "x29",
    ];
    for (index, register) in kept.iter().enumerate() {
        machine.set_reg(register, 0xC0DE_0000 + index as u64);
    }
    // The claim was made for it by the thread that started it.
    set_ready_word(&mut machine, BUILDING);
    let mut platform = Platform::default();
    machine.call(BUILDER_ENTRY, &[0], &mut platform);
    assert_eq!(platform.probes, 1);
    assert!(platform.created.is_empty());
    assert_eq!(platform.pools, (1, 1), "one pool pushed and popped");
    assert_eq!(ready_word(&mut machine), FAILED);
    for (index, register) in kept.iter().enumerate() {
        assert_eq!(
            machine.reg(register),
            0xC0DE_0000 + index as u64,
            "the builder's entry returned with {register} changed"
        );
    }
}

fn instructions<'a>(plan: &'a Value, symbol: &str) -> &'a Vec<Value> {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"] == symbol)
        .unwrap_or_else(|| panic!("no {symbol} in the code plan"))["instructions"]
        .as_array()
        .expect("instructions array")
}

/// The hand-off is one word. The builder writes every Metal object's word with plain
/// stores and the ready word LAST, with a store-release; the graphics thread reads the
/// ready word first, with a load-acquire, and the objects only after it reads "built".
#[test]
fn the_metal_ready_word_is_published_with_a_release_and_read_with_an_acquire() {
    let plan = ncode();
    let build = instructions(plan, "_mfb_macapp_metal_init");
    let releases = build.iter().filter(|i| i["op"] == "stlr_u64").count();
    assert!(
        releases >= 3,
        "the build publishes building, built and failed with a store-release each; \
         found {releases}"
    );
    let ready_offset = READY.to_string();
    let plain = build
        .iter()
        .filter(|i| i["op"] == "str_u64" && i["offset"] == ready_offset.as_str())
        .count();
    assert_eq!(
        plain, 0,
        "the build writes the ready word with a plain store"
    );
    let ready = instructions(plan, IS_READY);
    assert!(
        ready.iter().any(|i| i["op"] == "ldar_u64"),
        "canvas::metalReady reads the ready word with a plain load"
    );
}
