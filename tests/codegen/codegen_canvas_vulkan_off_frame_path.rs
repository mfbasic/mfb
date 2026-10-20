//! bug-996: the Vulkan pipeline is never on a frame's critical path.
//!
//! On a real GPU driver `vkCreateGraphicsPipelines` for the canvas shader takes tens
//! of seconds the first time an executable name is seen (38 s on box 206's Intel UHD
//! 630). The render loop used to ask `canvas::vulkanReady()` inside the first frame,
//! and that call was the build: no frame, software or GPU, could be drawn until it
//! returned.
//!
//! The rule now: asking whether the pipeline is ready calls nothing, and the build is
//! `canvas::vulkanBuild`, which runs on a thread of its own unless the program asked
//! for the GPU by name (`MFB_CANVAS_GPU`), where it runs on the caller as it always did.
//!
//! ## Why this runs the emitted code rather than a program
//!
//! The development host has no Vulkan backend (`canvas::vulkanReady` is a constant
//! `FALSE` on macOS), and the box with a slow driver is a real machine. So these run
//! the functions a Linux or Windows build emits, in the `-ncode` interpreter, against a
//! model of the thread library and the loader: what the readiness question calls, when
//! the thread is created, and what is in the ready word when it is.

#[path = "../common/mod.rs"]
mod common;

use common::ncode_emu::{Abi, Externals, Machine};
use common::{build_ncode_app, temp_project};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

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

/// Every target with a Vulkan backend.
const TARGETS: [&str; 3] = ["linux-aarch64", "linux-x86_64", "windows-x86_64"];

const GRAPHICS_STATE: &str = "_mfb_rt_canvas_graphics";
/// `GRAPHICS_OFFSET_VULKAN_READY` (`src/codegen/runtime/canvas/state_layout.rs`).
const READY: u64 = 424;
const UNTRIED: u64 = 0;
const BUILT: u64 = 1;
const FAILED: u64 = 2;
const BUILDING: u64 = 3;

const IS_READY: &str = "_mfb_rt_canvas_canvas_vulkanReady";
const BUILD: &str = "_mfb_rt_canvas_canvas_vulkanBuild";
const BUILDER_ENTRY: &str = "_mfb_rt_canvas_vulkan_builder";

/// `canvas::vulkanBuild`'s argument: where the build runs.
const ON_ITS_OWN_THREAD: u64 = 0;
const ON_THIS_THREAD: u64 = 1;

fn ncode_for(target: &'static str) -> &'static Value {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static Value>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.entry(target).or_insert_with(|| {
        let project = temp_project("canvas_vulkan_off_frame", SOURCE);
        let ncode = build_ncode_app(&project, target, "canvas_vulkan_off_frame");
        let _ = std::fs::remove_dir_all(&project);
        Box::leak(Box::new(ncode))
    })
}

/// Where an MFBASIC runtime member leaves its `Boolean` answer.
fn answer(machine: &Machine) -> u64 {
    machine.reg(match machine.abi {
        Abi::Aarch64 => "x1",
        Abi::SysV => "rsi",
        Abi::Win64 => "rdx",
        Abi::Riscv => unreachable!("canvas has no RISC-V target"),
    })
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

/// The thread library and the loader, as far as starting the build reaches them.
#[derive(Default)]
struct Platform {
    /// Whether the thread library can start a thread.
    threads_fail: bool,
    /// The start routine each created thread was given, and the ready word at that
    /// moment.
    created: Vec<(String, u64)>,
    /// Threads given back to the system (`pthread_detach` / `CloseHandle`).
    released: Vec<u64>,
    /// Loader opens (`dlopen` / `LoadLibraryExA`), each answered "no such library".
    opens: usize,
}

const THREAD: u64 = 0x7ead;

impl Externals for Platform {
    fn call(&mut self, m: &mut Machine, name: &str) -> Option<u64> {
        match name {
            "pthread_create" => {
                let routine = m.symbol_at(m.arg(2)).unwrap_or("?").to_string();
                let ready = ready_word(m);
                self.created.push((routine, ready));
                if self.threads_fail {
                    return Some(11); // EAGAIN
                }
                m.write_uint(m.arg(0), 8, THREAD);
                Some(0)
            }
            "pthread_detach" => {
                self.released.push(m.arg(0));
                Some(0)
            }
            "CreateThread" => {
                let routine = m.symbol_at(m.arg(2)).unwrap_or("?").to_string();
                let ready = ready_word(m);
                self.created.push((routine, ready));
                Some(if self.threads_fail { 0 } else { THREAD })
            }
            "CloseHandle" => {
                self.released.push(m.arg(0));
                Some(1)
            }
            "dlopen" | "LoadLibraryExA" => {
                self.opens += 1;
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
fn asking_whether_the_pipeline_is_ready_calls_nothing() {
    for target in TARGETS {
        for (state, expected) in [(UNTRIED, 0), (BUILDING, 0), (FAILED, 0), (BUILT, 1)] {
            let mut machine = Machine::new(ncode_for(target));
            set_ready_word(&mut machine, state);
            machine.call(IS_READY, &[], &mut NoPlatform);
            assert_eq!(
                answer(&machine),
                expected,
                "{target}: canvas::vulkanReady() with the ready word at {state}"
            );
            assert_eq!(
                ready_word(&mut machine),
                state,
                "{target}: the question changed the ready word from {state}"
            );
        }
    }
}

/// Starting the build claims it (so no second thread is ever started), creates one
/// thread whose start routine is the builder's entry, and hands the thread back to the
/// system: nobody joins it, because nothing may wait for a pipeline compile.
#[test]
fn the_build_starts_on_a_thread_of_its_own() {
    for target in TARGETS {
        let mut machine = Machine::new(ncode_for(target));
        let mut platform = Platform::default();
        machine.call(BUILD, &[ON_ITS_OWN_THREAD], &mut platform);
        assert_eq!(
            platform.created,
            vec![(BUILDER_ENTRY.to_string(), BUILDING)],
            "{target}: one thread, entered at the builder, created after the claim"
        );
        assert_eq!(platform.released, vec![THREAD], "{target}");
        assert_eq!(platform.opens, 0, "{target}: the caller opened the loader");
        assert_eq!(ready_word(&mut machine), BUILDING, "{target}");

        // Asked again while it runs, and again after it finished either way.
        for state in [BUILDING, BUILT, FAILED] {
            set_ready_word(&mut machine, state);
            machine.call(BUILD, &[ON_ITS_OWN_THREAD], &mut NoPlatform);
            machine.call(BUILD, &[ON_THIS_THREAD], &mut NoPlatform);
            assert_eq!(ready_word(&mut machine), state, "{target}");
        }
    }
}

/// No thread, no GPU: the frames stay software, as with no Vulkan at all.
#[test]
fn a_thread_that_cannot_be_started_is_a_failed_build() {
    for target in TARGETS {
        let mut machine = Machine::new(ncode_for(target));
        let mut platform = Platform {
            threads_fail: true,
            ..Platform::default()
        };
        machine.call(BUILD, &[ON_ITS_OWN_THREAD], &mut platform);
        assert_eq!(platform.created.len(), 1, "{target}");
        assert!(platform.released.is_empty(), "{target}");
        assert_eq!(ready_word(&mut machine), FAILED, "{target}");
        machine.call(IS_READY, &[], &mut NoPlatform);
        assert_eq!(answer(&machine), 0, "{target}");
    }
}

/// `MFB_CANVAS_GPU` set: the build runs on the caller, to its end, as it always did.
/// Here the loader has no Vulkan, so its end is "failed".
#[test]
fn asked_for_by_name_the_build_runs_on_the_calling_thread() {
    for target in TARGETS {
        let mut machine = Machine::new(ncode_for(target));
        let mut platform = Platform::default();
        machine.call(BUILD, &[ON_THIS_THREAD], &mut platform);
        assert!(
            platform.created.is_empty(),
            "{target}: a thread was started"
        );
        assert_eq!(platform.opens, 1, "{target}");
        assert_eq!(ready_word(&mut machine), FAILED, "{target}");
    }
}

/// The builder thread's whole life: build, publish the answer, return. A machine with
/// no Vulkan publishes "failed", and the thread library gets back every register it
/// keeps across the start routine.
#[test]
fn the_builder_thread_publishes_its_answer_and_returns() {
    for target in TARGETS {
        let mut machine = Machine::new(ncode_for(target));
        let kept: &[&str] = match machine.abi {
            Abi::Aarch64 => &[
                "x19", "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28",
            ],
            Abi::SysV => &["rbx", "rbp", "r12", "r13", "r14", "r15"],
            Abi::Win64 => &["rbx", "rbp", "rdi", "rsi", "r12", "r13", "r14", "r15"],
            Abi::Riscv => unreachable!("canvas has no RISC-V target"),
        };
        for (index, register) in kept.iter().enumerate() {
            machine.set_reg(register, 0xC0DE_0000 + index as u64);
        }
        // The claim was made for it by the thread that started it.
        set_ready_word(&mut machine, BUILDING);
        let mut platform = Platform::default();
        machine.call(BUILDER_ENTRY, &[0], &mut platform);
        assert_eq!(platform.opens, 1, "{target}");
        assert!(platform.created.is_empty(), "{target}");
        assert_eq!(ready_word(&mut machine), FAILED, "{target}");
        for (index, register) in kept.iter().enumerate() {
            assert_eq!(
                machine.reg(register),
                0xC0DE_0000 + index as u64,
                "{target}: the builder's entry returned with {register} changed"
            );
        }
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

/// The hand-off is one word. The builder writes every Vulkan object's word with plain
/// stores and the ready word LAST; the graphics thread reads the ready word first and
/// the objects only after it reads "built". On AArch64 that order has to be asked for:
/// the publish is a store-release and the read a load-acquire. x86-64 keeps stores in
/// order and loads in order by itself.
#[test]
fn the_ready_word_is_published_with_a_release_and_read_with_an_acquire_on_aarch64() {
    let plan = ncode_for("linux-aarch64");
    let build = instructions(plan, BUILD);
    let releases = build.iter().filter(|i| i["op"] == "stlr_u64").count();
    assert!(
        releases >= 2,
        "canvas::vulkanBuild publishes built and failed with a store-release each; found {releases}"
    );
    let ready_offset = READY.to_string();
    let plain = build
        .iter()
        .filter(|i| i["op"] == "str_u64" && i["offset"] == ready_offset.as_str())
        .filter(|i| i["base"] != "sp")
        .count();
    assert_eq!(
        plain, 0,
        "canvas::vulkanBuild writes the ready word with a plain store"
    );
    let ready = instructions(plan, IS_READY);
    assert!(
        ready.iter().any(|i| i["op"] == "ldar_u64"),
        "canvas::vulkanReady reads the ready word with a plain load"
    );
}
