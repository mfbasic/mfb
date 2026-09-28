//! bug-730: the macOS and Windows `--app` term surfaces draw a cursor while
//! `term::showCursor` is in effect, as the terminal and Linux GTK backends do.
//! Before the fix both stored the cursor-visible flag and no paint code read it.
//!
//! A painted caret is not observable headlessly, so these read the emitted plan:
//! the paint routine must read the flag after it has painted the grid. The
//! behaviour itself was verified by screenshot on this Mac and on the real
//! Windows box (bug-730 doc).

#[path = "../common/mod.rs"]
mod common;
use serde_json::Value;
use std::process::Command;

const SOURCE: &str = "IMPORT term\n\nFUNC main AS Integer\n  term::on()\n  \
term::showCursor()\n  term::hideCursor()\n  term::sync()\n  term::off()\n  RETURN 0\nEND FUNC\n";

fn plan(name: &str, target: &str) -> Value {
    let project = common::temp_project(name, SOURCE);
    let output = Command::new(common::mfb_exe())
        .args(["build", "-ncode", "--app", "-target", target])
        .arg(&project)
        .output()
        .expect("run mfb build -ncode --app");
    assert!(
        output.status.success(),
        "mfb build -ncode --app -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let text =
        std::fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode dump");
    let _ = std::fs::remove_dir_all(&project);
    serde_json::from_str(&text).expect("parse ncode json")
}

fn instructions<'a>(plan: &'a Value, symbol: &str) -> &'a Vec<Value> {
    plan["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["symbol"] == symbol)
        .unwrap_or_else(|| panic!("no function {symbol} in the plan"))["instructions"]
        .as_array()
        .expect("instructions")
}

/// The index of the first instruction calling or addressing `name`.
fn first_ref(ins: &[Value], name: &str) -> Option<usize> {
    ins.iter()
        .position(|i| i["target"] == name || i["symbol"] == name)
}

#[test]
fn macos_draw_rect_reads_the_cursor_flag_and_fills_a_caret() {
    let plan = plan("bug730_macos", "macos-aarch64");
    let ins = instructions(&plan, "_mfb_macapp_term_drawRect");
    // TV_CURSOR_VISIBLE_OFFSET (56) read off the TVSTATE pointer (x19).
    let flag = ins
        .iter()
        .position(|i| i["op"] == "ldr_u64" && i["base"] == "x19" && i["offset"] == "56")
        .expect("drawRect must read the cursor-visible flag (TVSTATE+56)");
    // ...and fill a rect after reading it (the caret bar).
    let fill_after = ins[flag..]
        .iter()
        .any(|i| i["target"] == "_NSRectFill" || i["symbol"] == "_NSRectFill");
    assert!(
        fill_after,
        "drawRect must NSRectFill the caret after reading the flag"
    );
}

#[test]
fn windows_paint_draws_a_caret_from_the_mirrored_flag() {
    let plan = plan("bug730_windows", "windows-x86_64");
    let paint = instructions(&plan, "_mfb_winapp_wndproc");
    let blit = first_ref(paint, "BitBlt").expect("WM_PAINT BitBlts the grid");
    let flag = first_ref(paint, "_mfb_winapp_tui_cursor_vis")
        .expect("WM_PAINT must read the mirrored cursor-visible flag");
    assert!(
        flag > blit,
        "the caret is painted over the blitted grid, not into it"
    );
    assert!(
        paint[flag..]
            .iter()
            .any(|i| i["target"] == "PatBlt" || i["symbol"] == "PatBlt"),
        "WM_PAINT must PatBlt the caret bar after reading the flag"
    );
    // The WndProc has no arena-state register, so the setters and term::on must
    // write the mirrored global the paint reads.
    for setter in [
        "_mfb_rt_term_term_showCursor",
        "_mfb_rt_term_term_hideCursor",
        "_mfb_rt_term_term_on",
    ] {
        assert!(
            first_ref(instructions(&plan, setter), "_mfb_winapp_tui_cursor_vis").is_some(),
            "{setter} must write _mfb_winapp_tui_cursor_vis"
        );
    }
}
