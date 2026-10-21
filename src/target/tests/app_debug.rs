//! `--debug` app builds: the transcript readback and the canvas frame profile.
//!
//! A debug app build carries test hooks a release one does not: `MFB_TRANSCRIPT_DUMP`
//! makes the headless run print the transcript view's text before it exits, and a
//! canvas program's Metal renderer carries the frame-profile strings. None of it is
//! in a normal build, so none of it is reached by the corpus.

use super::app_window::{function, targets};
use super::plan_imports::{module_for, HELLO};
use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{code_for_nir, CodeTarget};

const CANVAS: &str = "\
IMPORT app
IMPORT canvas

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::present([])
END FUNC
";

fn app_plan(source: &str, target: CodeTarget, debug: bool) -> NativeCodePlan {
    let mode = target.app_mode().expect("an app target");
    let mut module = module_for(source, target, mode);
    module.debug.enabled = debug;
    code_for_nir(&module, target).unwrap_or_else(|err| panic!("{}: {err}", target.name()))
}

fn has_cstring(plan: &NativeCodePlan, text: &str) -> bool {
    let mut hex: String = text.bytes().map(|byte| format!("{byte:02x}")).collect();
    hex.push_str("00");
    plan.data_objects.iter().any(|object| object.value == hex)
}

/// macOS: a debug build reads `MFB_TRANSCRIPT_DUMP` at exit and hops to the main
/// thread to write the transcript view's text to stdout; a normal build has neither
/// the helper nor the variable's name.
#[test]
fn a_debug_macos_app_carries_the_transcript_readback_and_a_release_one_does_not() {
    let target = CodeTarget::MacosAarch64;
    let debug = app_plan(HELLO, target, true);
    let dump = function(&debug, "_mfb_macapp_transcript_dump");
    let reached = targets(dump);
    for symbol in ["_objc_msgSend", "_strlen", "_write"] {
        assert!(
            reached.contains(&symbol),
            "the readback must call {symbol}; it reaches {reached:?}"
        );
    }
    // string, then UTF8String: two sends before the first strlen.
    let sends_before_strlen = reached
        .iter()
        .take_while(|symbol| **symbol != "_strlen")
        .filter(|symbol| **symbol == "_objc_msgSend")
        .count();
    assert_eq!(sends_before_strlen, 2);
    let finish = targets(function(&debug, "_mfb_macapp_program_finish"));
    for symbol in ["_getenv", "_dispatch_sync_f"] {
        assert!(
            finish.contains(&symbol),
            "the exit path must call {symbol}; the finish helper reaches {finish:?}"
        );
    }
    assert!(has_cstring(&debug, "MFB_TRANSCRIPT_DUMP"));

    let release = app_plan(HELLO, target, false);
    assert!(!release
        .functions
        .iter()
        .any(|f| f.symbol == "_mfb_macapp_transcript_dump"));
    assert!(!has_cstring(&release, "MFB_TRANSCRIPT_DUMP"));
}

/// macOS: a debug canvas build's Metal renderer profiles itself (GPU start/end
/// times) and honours `MFB_CANVAS_VSYNC`; a release canvas build names none of it.
#[test]
fn a_debug_macos_canvas_app_carries_the_metal_profile_strings_and_a_release_one_does_not() {
    let target = CodeTarget::MacosAarch64;
    let debug = app_plan(CANVAS, target, true);
    for text in [
        "GPUStartTime",
        "GPUEndTime",
        "setDisplaySyncEnabled:",
        "MFB_CANVAS_VSYNC",
    ] {
        assert!(
            has_cstring(&debug, text),
            "a debug canvas build names `{text}`"
        );
    }
    let release = app_plan(CANVAS, target, false);
    for text in ["GPUStartTime", "MFB_CANVAS_VSYNC"] {
        assert!(
            !has_cstring(&release, text),
            "a release canvas build must not name `{text}`"
        );
    }
}

/// Linux (both ISAs): a debug build writes the transcript buffer's text to stdout
/// from a GLib idle callback and ends the process; a debug canvas build also reads
/// the scripted resize and pointer variables. A release build has none of it.
#[test]
fn a_debug_linux_app_carries_the_transcript_idle_readback_and_the_canvas_hooks() {
    for target in [CodeTarget::LinuxX86_64, CodeTarget::LinuxAarch64] {
        let name = target.name();
        let debug = app_plan(HELLO, target, true);
        let dump = targets(function(&debug, "_mfb_gtkapp_transcript_dump_idle"));
        for symbol in [
            "gtk_text_buffer_get_bounds",
            "gtk_text_buffer_get_text",
            "strlen",
        ] {
            assert!(
                dump.contains(&symbol),
                "{name}: the readback must call {symbol}; it reaches {dump:?}"
            );
        }
        assert!(has_cstring(&debug, "MFB_TRANSCRIPT_DUMP"), "{name}");
        let release = app_plan(HELLO, target, false);
        assert!(
            !release
                .functions
                .iter()
                .any(|f| f.symbol == "_mfb_gtkapp_transcript_dump_idle"),
            "{name}"
        );
        assert!(!has_cstring(&release, "MFB_TRANSCRIPT_DUMP"), "{name}");
    }
    let canvas = app_plan(CANVAS, CodeTarget::LinuxX86_64, true);
    for text in [
        "MFB_CANVAS_RESIZE_W",
        "MFB_CANVAS_RESIZE_H",
        "MFB_CANVAS_POINTER_X",
        "MFB_CANVAS_POINTER_Y",
    ] {
        assert!(
            has_cstring(&canvas, text),
            "a debug canvas build names `{text}`"
        );
    }
    let canvas = app_plan(CANVAS, CodeTarget::LinuxX86_64, false);
    assert!(!has_cstring(&canvas, "MFB_CANVAS_RESIZE_W"));
}
