//! bug-540: the Windows `--app` `term::` backend against the macOS one.
//!
//! The Windows GDI backend draws, but it drew a *reduced* surface. Two of the
//! four sub-issues are closed here and both are asserted the same way — by
//! building the SAME program for `macos-aarch64` and for `windows-x86_64` and
//! comparing the two bodies. The macOS app backend is the behavioural oracle
//! (`mfb spec app term-backend`), so a cross-backend comparison is worth more
//! than restating a constant table in this file: a table restated here can drift
//! from the emitters and keep passing, while a comparison against a backend that
//! reads the real table cannot.
//!
//! * **WIN-01** `term::LineStyle`/`term::FillStyle` were ignored.
//!   `emit_term_draw_line` wrote `9472`/`9474` as literals, `emit_term_draw_box`
//!   hard-coded the four Light corners, and `emit_term_fill_rect` stamped a space
//!   and let the background colour show — so `Filled`, `Light`, `Medium`, `Dark`,
//!   `Checker` and `CheckerAlt` were indistinguishable and every rule was `Light`.
//! * **WIN-05** (found while fixing bug-541) `term::on` reset only `active`, `fg`
//!   and `bg`, where `mfb man term` promises it "resets all `term::` state to its
//!   defaults" and the console, macOS and GTK bodies all reset six fields plus the
//!   pending-resize flag. Measured on box 2230 before the fix: `term::setBold`,
//!   `term::off`, `term::on`, `term::getBold` answered `TRUE` on Windows and
//!   `FALSE` everywhere else.
//!
//! WIN-02 (fixed 80x25 surface), WIN-03 (`didResize` always `FALSE`) and WIN-04
//! (UTF-16 units rather than grapheme clusters) are NOT closed here; see the bug.

#[path = "../common/mod.rs"]
mod common;
use common::temp_project;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Every positioned member, each with a style the `Light` literal cannot fake:
/// `Double` and `HeavyDash` are different glyphs in every table, and `Dark` is a
/// different glyph from a space.
const TERM_SOURCE: &str = "IMPORT io\nIMPORT term\nIMPORT color\n\nSUB main()\n  \
     term::on()\n  \
     term::setBold(TRUE)\n  \
     term::setUnderline(TRUE)\n  \
     term::showCursor()\n  \
     term::setForeground(color::rgb(0, 255, 0))\n  \
     term::setBackground(color::rgb(0, 0, 0))\n  \
     term::drawHLine(term::LineStyle.Double, 5, 1, 12)\n  \
     term::drawVLine(term::LineStyle.HeavyDash, 6, 3, 10)\n  \
     term::drawBox(term::LineStyle.Double, 7, 2, 11, 20)\n  \
     term::fillRect(term::FillStyle.Dark, 8, 4, 10, 16)\n  \
     term::drawText(9, 6, \"app\")\n  \
     term::drawGlyph(10, 7, 9731)\n  \
     term::sync()\n  \
     LET seen AS Boolean = term::getBold()\n  \
     io::print(toString(seen) & toString(term::isOn()) & toString(term::didResize()))\n  \
     term::off()\nEND SUB\n";

fn app_ncode_functions(name: &str, target: &str) -> serde_json::Map<String, Value> {
    let project = temp_project(name, TERM_SOURCE);
    let mut command = Command::new(common::mfb_exe());
    // `-target` on BOTH sides, always. Omitting it for the macOS oracle asked for
    // the HOST backend, which is macOS only on a developer Mac: on the Linux CI
    // rows the "macOS oracle" was the GTK backend, which emits no box-drawing
    // immediate at all, and the premise assertion below fired instead of the
    // comparison. Measured with the release compiler: `-app -target
    // macos-aarch64 -ncode` yields 7/7/26/6 glyphs for drawHLine/drawVLine/
    // drawBox/fillRect from any host, `-target linux-x86_64` yields 0/0/0/0.
    command.arg("build").arg("-app").args(["-target", target]);
    let output = command
        .arg("-ncode")
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build -app {target} -ncode failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(Path::new(&project).join(format!("{name}.ncode")))
        .expect("read the native code plan");
    let plan: Value = serde_json::from_str(&text).expect("ncode is JSON");
    let mut map = serde_json::Map::new();
    for function in plan["functions"].as_array().expect("functions array") {
        map.insert(
            function["symbol"].as_str().expect("symbol").to_string(),
            function["instructions"].clone(),
        );
    }
    map
}

fn body<'a>(functions: &'a serde_json::Map<String, Value>, symbol: &str) -> &'a [Value] {
    functions
        .get(symbol)
        .unwrap_or_else(|| panic!("{symbol} is emitted by a term:: app build"))
        .as_array()
        .expect("instructions")
}

/// Every immediate in a body that falls in the box-drawing / block-element range
/// `U+2500..U+259F` — i.e. exactly the `TERM_*_CODEPOINTS` entries, and nothing a
/// coordinate, colour or count could be confused with (the largest grid bound this
/// backend emits is 79).
fn glyph_immediates(instructions: &[Value]) -> std::collections::BTreeSet<u32> {
    instructions
        .iter()
        .filter(|instruction| instruction["op"] == "mov_imm")
        .filter_map(|instruction| instruction["value"].as_str())
        .filter_map(|value| value.parse::<u32>().ok())
        .filter(|value| (0x2500..=0x259F).contains(value))
        .collect()
}

/// bug-540 WIN-01. The Windows bodies must select their glyphs from the same
/// `TERM_*_CODEPOINTS` tables the macOS bodies index, so the two backends emit the
/// same glyph set for the same member. Before the fix Windows emitted `9472`
/// (`0x2500`) and `9474` (`0x2502`) as the only rule glyphs, the four Light
/// corners, and `32` for every fill — sets of size 1, 6 and 0 against macOS's 7,
/// 26 and 6.
#[test]
fn windows_app_mode_selects_line_and_fill_glyphs_from_the_shared_tables() {
    let windows = app_ncode_functions("win_app_term_styles_win", "windows-x86_64");
    let macos = app_ncode_functions("win_app_term_styles_mac", "macos-aarch64");
    for member in ["drawHLine", "drawVLine", "drawBox", "fillRect"] {
        let symbol = format!("_mfb_rt_term_term_{member}");
        let want = glyph_immediates(body(&macos, &symbol));
        let got = glyph_immediates(body(&windows, &symbol));
        assert!(
            !want.is_empty(),
            "the macOS oracle for term::{member} emits no box-drawing glyph at all — \
             this test's premise is wrong, not the Windows backend"
        );
        assert_eq!(
            got, want,
            "term::{member}: the Windows app body must select from the same \
             TERM_*_CODEPOINTS tables the macOS body indexes.\n  macOS: {want:04X?}\n\
             \x20 win  : {got:04X?}",
        );
    }
}

/// The positive half of WIN-01, and the half a wrong ordinal chain passes without:
/// the style still has to reach the SURFACE. Each body must still stamp through
/// `TextOutW`, and must read the ordinal it was handed rather than a constant.
#[test]
fn windows_app_mode_still_stamps_the_selected_glyph_through_textoutw() {
    let windows = app_ncode_functions("win_app_term_styles_stamp", "windows-x86_64");
    for member in ["drawHLine", "drawVLine", "drawBox", "fillRect"] {
        let symbol = format!("_mfb_rt_term_term_{member}");
        let instructions = body(&windows, &symbol);
        assert!(
            instructions.iter().any(|instruction| {
                instruction["op"] == "bl" && instruction["target"] == serde_json::json!("TextOutW")
            }),
            "term::{member} must still stamp its glyph with TextOutW — selecting the \
             right code point and never drawing it is the same bug from the other side"
        );
        // The ordinal arrives in ARG[0] (`rcx` on Win64) and this backend used to
        // never read it — its first store was of ARG[1]. Parking the ordinal has to
        // be the body's FIRST store, because the clipping helpers below use
        // ARG[0..2] as scratch and would otherwise destroy it.
        let first_store = instructions
            .iter()
            .find(|instruction| instruction["op"] == "str_u64")
            .expect("the body parks its incoming arguments");
        assert_eq!(
            first_store["src"],
            serde_json::json!("rcx"),
            "term::{member} must park the incoming style ordinal FIRST — it arrives in \
             ARG[0], the clipping helpers clobber ARG[0..2], and this backend ignored \
             it entirely (bug-540 WIN-01)"
        );
    }
}

/// The `(base, offset)` a single-slot `term::` reader or writer touches, used to
/// derive each term-state field from the member that owns it rather than
/// hard-coding an arena layout that moves.
fn slot_of(functions: &serde_json::Map<String, Value>, symbol: &str, op: &str) -> (String, String) {
    let instructions = body(functions, symbol);
    let instruction = instructions
        .iter()
        .rev()
        .find(|instruction| instruction["op"] == op)
        .unwrap_or_else(|| panic!("{symbol} performs a {op} on its term-state field"));
    (
        instruction["base"].as_str().expect("base").to_string(),
        instruction["offset"].as_str().expect("offset").to_string(),
    )
}

/// bug-540 WIN-05. `term::on` resets every `term::` setting to its default, on
/// every backend. The Windows body reset three fields; the other three backends
/// reset six plus the pending-resize flag, and `mfb man term` documents the reset
/// as covering "white foreground, black background, bold and underline off,
/// cursor visible".
#[test]
fn windows_app_mode_term_on_resets_every_term_state_field() {
    let windows = app_ncode_functions("win_app_term_on_reset", "windows-x86_64");
    // Each field is located from the member that owns it, so nothing here restates
    // the arena term-state layout.
    let fields = [
        (
            "active",
            slot_of(&windows, "_mfb_rt_term_term_isOn", "ldr_u64"),
        ),
        (
            "foreground",
            slot_of(&windows, "_mfb_rt_term_term_setForeground", "str_u64"),
        ),
        (
            "background",
            slot_of(&windows, "_mfb_rt_term_term_setBackground", "str_u64"),
        ),
        (
            "bold",
            slot_of(&windows, "_mfb_rt_term_term_setBold", "str_u64"),
        ),
        (
            "underline",
            slot_of(&windows, "_mfb_rt_term_term_setUnderline", "str_u64"),
        ),
        (
            "cursorVisible",
            slot_of(&windows, "_mfb_rt_term_term_showCursor", "str_u64"),
        ),
        (
            "didResize",
            slot_of(&windows, "_mfb_rt_term_term_didResize", "str_u64"),
        ),
    ];
    let on = body(&windows, "_mfb_rt_term_term_on");
    for (name, (base, offset)) in fields {
        assert!(
            on.iter().any(|instruction| {
                instruction["op"] == "str_u64"
                    && instruction["base"].as_str() == Some(base.as_str())
                    && instruction["offset"].as_str() == Some(offset.as_str())
            }),
            "term::on must reset the shared term-state '{name}' field ({base}+{offset}): \
             entering TUI mode starts from the documented defaults on every backend, and \
             a Windows-only leftover made `setBold`/`off`/`on`/`getBold` answer TRUE \
             there and FALSE everywhere else (bug-540 WIN-05)"
        );
    }
    // And the reset is a reset: `term::on` must not READ any of those fields first.
    assert!(
        !on.iter().any(|instruction| {
            instruction["op"] == "ldr_u64"
                && instruction["base"].as_str() == Some(fields_base(&windows).as_str())
        }),
        "term::on must not read the shared term-state it is resetting"
    );
}

fn fields_base(functions: &serde_json::Map<String, Value>) -> String {
    slot_of(functions, "_mfb_rt_term_term_isOn", "ldr_u64").0
}
