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
//! * **WIN-04** `term::drawText` walked UTF-16 units — no combining-mark or ZWJ
//!   fold, no wide-at-the-edge drop, control units stamped, no row bound — while
//!   the same backend's `io::write` grid path already folded clusters. Asserted
//!   against THAT walk rather than against macOS: the fix emits the io::write walk
//!   a second time, so the fold must be the same instructions in both bodies.
//!
//! * **WIN-02** the surface was a compile-time 80x25: `term::terminalSize` stored
//!   the literals and every clamp said column 79 / row 24, whatever the window.
//! * **WIN-03** `term::didResize` had no Windows arm and fell through to the
//!   console body, whose arena flag nothing on Windows ever sets.
//!
//!   Both are asserted structurally here — the size is READ, every bound is read
//!   from the same place, and a resize from either the window or the scripted hook
//!   reaches it — and at run time by `scripts/test-winapp.sh` on box 2230, which
//!   drives `MFB_WINAPP_TERM_RESIZE_W`/`_H` and checks the reported size.

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
    functions_of(&app_ncode_plan(name, target, TERM_SOURCE))
}

fn functions_of(plan: &Value) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for function in plan["functions"].as_array().expect("functions array") {
        map.insert(
            function["symbol"].as_str().expect("symbol").to_string(),
            function["instructions"].clone(),
        );
    }
    map
}

/// The whole `-ncode` plan of `source` built `-app` for `target`.
fn app_ncode_plan(name: &str, target: &str, source: &str) -> Value {
    let project = temp_project(name, source);
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
    serde_json::from_str(&text).expect("ncode is JSON")
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
/// coordinate, colour or count could be confused with (the largest size immediate
/// this backend emits is the 3840-pixel backing bitmap width).
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

// ---------------------------------------------------------------------------
// bug-540 WIN-04: `term::drawText` runs the SAME cluster walk `io::write` does.
// ---------------------------------------------------------------------------

/// The io write body is the function that owns the TUI grid path — located by that
/// label rather than by a symbol spelling, so renaming the io member cannot turn
/// this into a test of nothing.
fn io_write_grid_body(functions: &serde_json::Map<String, Value>) -> &[Value] {
    functions
        .values()
        .filter_map(Value::as_array)
        .find(|instructions| {
            instructions.iter().any(|instruction| {
                instruction["op"] == "label" && instruction["name"] == "term_grid_path"
            })
        })
        .expect("an io::print in a term:: app build emits the io write body's TUI grid path")
}

fn label_index(instructions: &[Value], name: &str) -> Option<usize> {
    instructions
        .iter()
        .position(|instruction| instruction["op"] == "label" && instruction["name"] == name)
}

/// Strip a walk's specialization tag (`term_` in `io::write`, `dt_` in `drawText`)
/// from every label and branch target, so the two emissions compare instruction for
/// instruction. Nothing else is normalised: registers, immediates and slot offsets
/// must already be identical.
fn untag(instructions: &[Value]) -> Vec<Value> {
    instructions
        .iter()
        .map(|instruction| {
            let mut instruction = instruction.clone();
            for key in ["name", "target"] {
                if let Some(text) = instruction.get(key).and_then(Value::as_str) {
                    let bare = text
                        .strip_prefix("term_")
                        .or_else(|| text.strip_prefix("dt_"))
                        .unwrap_or(text)
                        .to_string();
                    instruction[key] = Value::String(bare);
                }
            }
            instruction
        })
        .collect()
}

/// `[label {tag}_extend ..= label {width_tag}_done]` — the combining-mark / ZWJ fold
/// and the display-width lookup, the two halves of "how many cells is this cluster".
fn cluster_window(instructions: &[Value], tag: &str, width_tag: &str) -> Option<Vec<Value>> {
    let start = label_index(instructions, &format!("{tag}_extend"))?;
    let end = label_index(instructions, &format!("{width_tag}_done"))?;
    (start < end).then(|| untag(&instructions[start..=end]))
}

/// bug-540 WIN-04. `term::drawText` on Windows iterated UTF-16 units: a combining
/// mark or a ZWJ-joined scalar took its own cell, where `mfb man term drawText`
/// promises one position per grapheme cluster. The same file's `io::write` grid
/// path already folds clusters, so the fix emits THAT walk for `drawText` too, and
/// this asserts the fold and the width lookup are the same instructions in both
/// bodies. Before the fix the `drawText` body had no fold at all.
#[test]
fn windows_app_mode_draw_text_shares_the_io_write_cluster_walk() {
    let windows = app_ncode_functions("win_app_term_draw_text_walk", "windows-x86_64");
    let write = cluster_window(io_write_grid_body(&windows), "term", "ww").expect(
        "the io::write grid path folds clusters and looks up a width — the oracle this \
         test compares against",
    );
    assert!(
        write.len() > 40,
        "the io::write cluster window is implausibly short ({} instructions) — this test's \
         premise is wrong",
        write.len()
    );
    let draw_text = body(&windows, "_mfb_rt_term_term_drawText");
    let shared = cluster_window(draw_text, "dt", "dt_ww").unwrap_or_else(|| {
        panic!(
            "term::drawText has no cluster fold: it walks UTF-16 units, so `e` + U+0301 and a \
             ZWJ emoji take several cells on Windows and one everywhere else (bug-540 WIN-04)"
        )
    });
    assert_eq!(
        shared, write,
        "term::drawText's cluster fold and width lookup must be the io::write walk emitted a \
         second time, not a second walk — two walks are how the two members come to disagree \
         about the same string (bug-540 WIN-04)"
    );
}

/// bug-540 WIN-04, the edge half, plus the two `drawText` rules the unit walk also
/// skipped. `mfb man term drawText`: a double-width cluster with one column left is
/// DROPPED and the run stops; control characters advance a column and stamp nothing;
/// a row off the surface draws nothing. The unit walk tested only `col >= 80`, so it
/// drew a wide unit in column 79, handed control characters to `TextOutW`, and
/// stamped any row it was given.
#[test]
fn windows_app_mode_draw_text_drops_a_wide_cluster_at_the_edge_and_skips_controls() {
    let windows = app_ncode_functions("win_app_term_draw_text_edge", "windows-x86_64");
    let draw_text = body(&windows, "_mfb_rt_term_term_drawText");
    let stamp = draw_text
        .iter()
        .position(|instruction| instruction["op"] == "bl" && instruction["target"] == "TextOutW")
        .expect("term::drawText stamps through TextOutW");
    let width_done = draw_text
        .iter()
        .rposition(|instruction| {
            instruction["op"] == "label"
                && instruction["name"]
                    .as_str()
                    .is_some_and(|name| name.ends_with("ww_done"))
        })
        .expect("term::drawText looks up a display width");
    assert!(width_done < stamp, "the width lookup precedes the stamp");
    assert!(
        draw_text[width_done..stamp].iter().any(|instruction| {
            instruction["op"]
                .as_str()
                .is_some_and(|op| op.starts_with("b."))
                && instruction["target"] == "dt_done"
        }),
        "between the width lookup and the stamp, term::drawText must be able to END the run: \
         a width-2 cluster with only one column left is dropped, never drawn half off the \
         surface (bug-540 WIN-04)"
    );
    assert!(
        draw_text[..stamp]
            .iter()
            .any(|instruction| instruction["op"] == "cmp_imm" && instruction["rhs"] == "32"),
        "term::drawText must test for a control character (< U+0020) before stamping — the \
         documented contract skips them, and TextOutW draws whatever the font maps a C0 unit to"
    );
    // `rows-1` is read from the live surface (bug-540 WIN-02) — this matched the
    // fixed surface's literal `24` until the surface stopped being 80x25.
    let row_guard = draw_text.windows(5).any(|w| {
        w[0]["op"] == "adrp"
            && w[1]["op"] == "ldr_u64"
            && w[2]["op"] == "sub_imm"
            && w[2]["imm"] == "1"
            && w[3]["op"] == "cmp"
            && w[4]["op"] == "b.gt"
            && w[4]["target"] == "dt_done"
    });
    assert!(
        row_guard,
        "term::drawText must draw nothing for a row off the surface (row > rows-1)"
    );
}

// ---------------------------------------------------------------------------
// bug-540 WIN-02 / WIN-03: the surface follows the window.
// ---------------------------------------------------------------------------

/// [`TERM_SOURCE`]'s members plus every member whose answer or bound is the
/// surface size: `terminalSize`, `didResize`, `moveTo` and an `io::write` that runs
/// the grid walk while TUI mode is on (and `isOn`, which locates the arena
/// term-state base, as [`fields_base`] does).
const RESIZE_SOURCE: &str = "IMPORT io\nIMPORT term\n\nSUB main()\n  \
     term::on()\n  \
     LET size AS term::TermSize = term::terminalSize()\n  \
     term::moveTo(size.rows - 1, size.columns - 1)\n  \
     term::drawHLine(term::LineStyle.Double, 5, 1, 12)\n  \
     term::drawVLine(term::LineStyle.HeavyDash, 6, 3, 10)\n  \
     term::drawBox(term::LineStyle.Double, 7, 2, 11, 20)\n  \
     term::fillRect(term::FillStyle.Dark, 8, 4, 10, 16)\n  \
     term::drawText(9, 6, \"app\")\n  \
     term::drawGlyph(10, 7, 9731)\n  \
     io::write(\"grid\")\n  \
     io::print(toString(term::didResize()) & toString(term::isOn()))\n  \
     term::sync()\n  \
     term::off()\nEND SUB\n";

fn resize_ncode_functions(name: &str) -> serde_json::Map<String, Value> {
    functions_of(&app_ncode_plan(name, "windows-x86_64", RESIZE_SOURCE))
}

/// The data symbols a body addresses (`adrp`), in order.
fn addressed(instructions: &[Value]) -> Vec<String> {
    instructions
        .iter()
        .filter(|instruction| instruction["op"] == "adrp")
        .filter_map(|instruction| instruction["symbol"].as_str())
        .map(str::to_string)
        .collect()
}

/// The backend-owned (`_mfb_winapp_*`) data symbols `term::terminalSize` reads its
/// answer from. Derived from the member rather than restated, like `slot_of`.
fn size_symbols(functions: &serde_json::Map<String, Value>) -> Vec<String> {
    addressed(body(functions, "_mfb_rt_term_term_terminalSize"))
        .into_iter()
        .filter(|symbol| symbol.starts_with("_mfb_winapp_"))
        .collect()
}

/// bug-540 WIN-02. `term::terminalSize` stored the literals `80` and `25` into the
/// record, so it reported 80x25 however large the window was. It must READ the
/// surface size — two backend globals, columns then rows — and store no literal.
#[test]
fn windows_app_mode_terminal_size_reads_the_live_surface_size() {
    let windows = resize_ncode_functions("win_app_term_size_live");
    let size = body(&windows, "_mfb_rt_term_term_terminalSize");
    let literals: Vec<&str> = size
        .iter()
        .filter(|instruction| instruction["op"] == "mov_imm")
        .filter_map(|instruction| instruction["value"].as_str())
        .filter(|value| matches!(*value, "80" | "25"))
        .collect();
    assert!(
        literals.is_empty(),
        "term::terminalSize must not report the compile-time 80x25 (found immediates \
         {literals:?}) — the surface follows the window (bug-540 WIN-02)"
    );
    let symbols = size_symbols(&windows);
    assert_eq!(
        symbols.len(),
        2,
        "term::terminalSize must read the columns and the rows from the backend's live \
         surface-size globals; it addresses {symbols:?} (bug-540 WIN-02)"
    );
}

/// bug-540 WIN-02, the half a partial conversion fails: every bound on the surface
/// must come from the SAME live size `term::terminalSize` reports. A clamp still
/// saying 79 while the surface is 120 wide silently truncates, and no run-time
/// check on the box can see which cell a glyph landed in — so this is where that
/// is pinned. `79`/`24` were the old last column/row; neither may survive as an
/// immediate in any body that bounds a coordinate.
#[test]
fn windows_app_mode_every_grid_bound_follows_the_live_size() {
    let windows = resize_ncode_functions("win_app_term_bounds_live");
    let symbols = size_symbols(&windows);
    assert_eq!(
        symbols.len(),
        2,
        "the size globals, from term::terminalSize"
    );
    let mut bodies: Vec<(String, &[Value])> = [
        "drawHLine",
        "drawVLine",
        "drawBox",
        "fillRect",
        "drawText",
        "drawGlyph",
        "moveTo",
    ]
    .iter()
    .map(|member| {
        let symbol = format!("_mfb_rt_term_term_{member}");
        let instructions = body(&windows, &symbol);
        (symbol, instructions)
    })
    .collect();
    bodies.push((
        "io::write grid path".to_string(),
        io_write_grid_body(&windows),
    ));
    for (name, instructions) in bodies {
        // A bound may live one call down in a backend helper (`io::write`'s scroll
        // is out of line), so the backend callees' reads count as the body's.
        let mut reads = addressed(instructions);
        for callee in instructions
            .iter()
            .filter(|instruction| instruction["op"] == "bl")
            .filter_map(|instruction| instruction["target"].as_str())
            .filter(|target| target.starts_with("_mfb_winapp_"))
        {
            if let Some(callee_body) = windows.get(callee).and_then(Value::as_array) {
                reads.extend(addressed(callee_body));
            }
        }
        for symbol in &symbols {
            assert!(
                reads.contains(symbol),
                "{name} must bound its coordinates by the live surface size ({symbol}), \
                 not a compile-time constant (bug-540 WIN-02)"
            );
        }
        let stale: Vec<&str> = instructions
            .iter()
            .filter(|instruction| matches!(instruction["op"].as_str(), Some("mov_imm")))
            .filter_map(|instruction| instruction["value"].as_str())
            .chain(
                instructions
                    .iter()
                    .filter(|instruction| instruction["op"] == "cmp_imm")
                    .filter_map(|instruction| instruction["rhs"].as_str()),
            )
            .filter(|value| matches!(*value, "79" | "24"))
            .collect();
        assert!(
            stale.is_empty(),
            "{name} still bounds against the fixed 80x25 surface's last column/row \
             ({stale:?}) — a partial conversion silently truncates (bug-540 WIN-02)"
        );
    }
}

/// bug-540 WIN-03. `term::didResize` had no Windows arm: it fell through to the
/// console body, which reads-and-clears the arena term-state flag that only the
/// console present ever latches — so it always read FALSE. The Windows arm must
/// read the resize the backend records on its own surface state (the macOS
/// `TV_DID_RESIZE` shape) and acknowledge it, never the console arena slot.
#[test]
fn windows_app_mode_did_resize_reads_the_windows_surface_record() {
    let windows = resize_ncode_functions("win_app_term_did_resize");
    let did_resize = body(&windows, "_mfb_rt_term_term_didResize");
    let arena = fields_base(&windows);
    assert!(
        !did_resize.iter().any(|instruction| {
            instruction["op"] == "ldr_u64" && instruction["base"].as_str() == Some(arena.as_str())
        }),
        "term::didResize must not read the console arena term-state flag: nothing on \
         Windows latches it, so it always reads FALSE (bug-540 WIN-03)"
    );
    let reads: Vec<String> = addressed(did_resize)
        .into_iter()
        .filter(|symbol| symbol.starts_with("_mfb_winapp_"))
        .collect();
    assert!(
        !reads.is_empty(),
        "term::didResize must read the resize the Windows backend records (bug-540 WIN-03)"
    );
    assert!(
        did_resize
            .iter()
            .any(|instruction| instruction["op"] == "str_u64"),
        "term::didResize must acknowledge what it reports — reading it clears it"
    );
}

/// The UTF-16LE, NUL-terminated string a `-ncode` data object holds, if it is one.
fn utf16_value(object: &Value) -> Option<String> {
    let hex = object["value"].as_str()?;
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect::<Option<_>>()?;
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|pair| u16::from_le_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16(&units).ok()
}

/// bug-540 WIN-02/WIN-03, the resize SOURCES. A window resize arrives as `WM_SIZE`
/// on the UI thread, and headless box runs have no window, so the scripted hook
/// (`MFB_WINAPP_TERM_RESIZE_W`/`_H`, the twin of `MFB_CANVAS_RESIZE_W`/`_H`) must
/// drive the SAME handler from `_main` — otherwise the run on box 2230 would prove
/// a stand-in rather than the production path. And that handler must be what
/// updates the size `term::terminalSize` reads.
#[test]
fn windows_app_mode_window_and_scripted_resize_share_one_term_handler() {
    let plan = app_ncode_plan("win_app_term_resize_paths", "windows-x86_64", RESIZE_SOURCE);
    let windows = functions_of(&plan);
    let calls = |symbol: &str| -> Vec<String> {
        body(&windows, symbol)
            .iter()
            .filter(|instruction| instruction["op"] == "bl")
            .filter_map(|instruction| instruction["target"].as_str())
            .map(str::to_string)
            .collect()
    };
    let wndproc_calls = calls("_mfb_winapp_wndproc");
    let main_calls = calls("_main");
    let shared: Vec<&String> = wndproc_calls
        .iter()
        .filter(|target| target.starts_with("_mfb_winapp_") && main_calls.contains(target))
        .collect();
    assert!(
        !shared.is_empty(),
        "WM_SIZE (in the window procedure) and the headless scripted resize (in _main) \
         must call one shared term:: resize handler (bug-540 WIN-02)"
    );
    let size = size_symbols(&windows);
    assert!(
        shared.iter().any(|handler| {
            windows.contains_key(handler.as_str()) && {
                let reads = addressed(body(&windows, handler));
                size.iter().all(|symbol| reads.contains(symbol))
            }
        }),
        "the shared resize handler must update the size term::terminalSize reports \
         ({size:?}) (bug-540 WIN-02)"
    );
    // The scripted hook's two variable names are data the bootstrap reads.
    let names: Vec<(String, String)> = plan["dataObjects"]
        .as_array()
        .expect("dataObjects")
        .iter()
        .filter_map(|object| Some((object["symbol"].as_str()?.to_string(), utf16_value(object)?)))
        .collect();
    let main_reads = addressed(body(&windows, "_main"));
    for variable in ["MFB_WINAPP_TERM_RESIZE_W", "MFB_WINAPP_TERM_RESIZE_H"] {
        let symbol = names
            .iter()
            .find(|(_, value)| value == variable)
            .map(|(symbol, _)| symbol.clone())
            .unwrap_or_else(|| {
                panic!(
                    "no data object names {variable}: the scripted term:: resize hook \
                     does not exist (bug-540 WIN-02 prerequisite)"
                )
            });
        assert!(
            main_reads.contains(&symbol),
            "_main must read {variable} on the headless path"
        );
    }
}
