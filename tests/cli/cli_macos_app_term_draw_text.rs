//! bug-594: the macOS `--app` `term::drawText` IMP (`TermView mfbDrawText:`,
//! `_mfb_macapp_term_drawText`) against the documented contract.
//!
//! `mfb man term drawText` says control characters (below U+0020) "advance one
//! column but stamp nothing", `mfb spec app term-backend` says "control bytes taking
//! a column without stamping", and the console and Linux GTK backends do exactly
//! that. The macOS IMP moved the UTF-16 index past a control character and left the
//! running column where it was, so `drawText(row, 0, "a\tb")` stamped `b` one column
//! further left on macOS than everywhere else.
//!
//! The `NSView` cannot be read back headlessly, and adding a readback hook is
//! product surface nobody has agreed to, so these pins read the EMITTED code plan
//! (`-app -target macos-aarch64 -ncode`). They prove which instructions the control
//! branch runs; they prove nothing about pixels.
//!
//! Every register, label and slot is derived from the body's own structure (the
//! loop back-edge, the loop-top exit test, the right-edge clip), so a renamed label
//! or a moved register fails a premise assertion rather than passing vacuously.

#[path = "../common/mod.rs"]
mod common;
use common::temp_project;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

const DRAW_TEXT_SYMBOL: &str = "_mfb_macapp_term_drawText";
const WRITE_STRING_SYMBOL: &str = "_mfb_macapp_term_writeString";

/// A `term::` program: the IMP is built with the utf8proc width table (`uses_term`).
const TERM_SOURCE: &str = "IMPORT io\nIMPORT term\n\nSUB main()\n  \
     term::on()\n  \
     term::drawText(0, 0, \"ab\")\n  \
     term::sync()\n  \
     term::off()\nEND SUB\n";

/// An app with no `term::` import: the IMP is still emitted, without the width table.
const PLAIN_SOURCE: &str = "IMPORT io\n\nSUB main()\n  io::print(\"ab\")\nEND SUB\n";

fn app_ncode_functions(name: &str, source: &str) -> HashMap<String, Vec<Value>> {
    let project = temp_project(name, source);
    // `-target` always: without it `-app` asks for the HOST backend, which is GTK on
    // the Linux CI rows.
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .args(["-target", "macos-aarch64"])
        .arg("-ncode")
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build -app -target macos-aarch64 -ncode failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(Path::new(&project).join(format!("{name}.ncode")))
        .expect("read the native code plan");
    let plan: Value = serde_json::from_str(&text).expect("ncode is JSON");
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .map(|function| {
            (
                function["symbol"].as_str().expect("symbol").to_string(),
                function["instructions"]
                    .as_array()
                    .expect("instructions")
                    .clone(),
            )
        })
        .collect()
}

fn op(instruction: &Value) -> &str {
    instruction["op"].as_str().unwrap_or("")
}

fn field<'a>(instruction: &'a Value, name: &str) -> &'a str {
    instruction[name].as_str().unwrap_or("")
}

fn is_conditional_branch(instruction: &Value) -> bool {
    op(instruction).starts_with("b.")
}

/// The structure of the `mfbDrawText:` loop, derived from the body alone.
struct Walk<'a> {
    body: &'a [Value],
    labels: HashMap<String, usize>,
    /// Index of the unconditional back-edge `b <loop>`.
    back_edge: usize,
    /// Index of the loop label.
    loop_top: usize,
    /// The label every "stop drawing" branch goes to.
    exit: String,
    /// UTF-16 index register (the loop-top `cmp i, n`).
    index: String,
    /// Running column register (the right-edge clip `cmp col, cols; b.ge exit`).
    column: String,
    /// Index of the `cmp_imm cp, #32` that selects the control branch.
    control_cmp: usize,
    /// Target label of the control branch.
    control_target: String,
    /// Index of the single `add col, col, width`.
    column_advance: usize,
}

fn walk_of(body: &[Value]) -> Walk<'_> {
    let labels: HashMap<String, usize> = body
        .iter()
        .enumerate()
        .filter(|(_, instruction)| op(instruction) == "label")
        .map(|(index, instruction)| (field(instruction, "name").to_string(), index))
        .collect();
    // The loop: a backward `b` whose label is followed by `cmp i, n; b.ge exit`.
    let loops: Vec<(usize, usize)> = body
        .iter()
        .enumerate()
        .filter(|(_, instruction)| op(instruction) == "b")
        .filter_map(|(index, instruction)| {
            let target = *labels.get(field(instruction, "target"))?;
            (target < index && op(&body[target + 1]) == "cmp" && op(&body[target + 2]) == "b.ge")
                .then_some((index, target))
        })
        .collect();
    assert_eq!(
        loops.len(),
        1,
        "premise: {DRAW_TEXT_SYMBOL} has exactly one `cmp i, n; b.ge exit` loop"
    );
    let (back_edge, loop_top) = loops[0];
    let index = field(&body[loop_top + 1], "lhs").to_string();
    let exit = field(&body[loop_top + 2], "target").to_string();
    let in_loop = loop_top + 3..back_edge;

    let clips: Vec<(usize, &str)> = in_loop
        .clone()
        .filter(|&k| {
            op(&body[k]) == "cmp"
                && op(&body[k + 1]) == "b.ge"
                && field(&body[k + 1], "target") == exit
                && field(&body[k], "lhs") != index
        })
        .map(|k| (k, field(&body[k], "lhs")))
        .collect();
    assert!(
        !clips.is_empty(),
        "premise: {DRAW_TEXT_SYMBOL} clips the run at the right edge (`cmp col, cols; b.ge exit`)"
    );
    // The FIRST exit test after the loop top is the run's right-edge clip; later
    // ones belong to the stamp (the wide-at-edge `col + 1` test, the wide-pair
    // clear), which compare scratch registers derived from the column.
    let column = clips[0].1.to_string();

    let controls: Vec<usize> = in_loop
        .clone()
        .filter(|&k| {
            op(&body[k]) == "cmp_imm"
                && field(&body[k], "rhs") == "32"
                && op(&body[k + 1]) == "b.lt"
        })
        .collect();
    assert_eq!(
        controls.len(),
        1,
        "premise: {DRAW_TEXT_SYMBOL} has one `cmp_imm cp, #32; b.lt` control-character test"
    );
    let control_cmp = controls[0];
    let control_target = field(&body[control_cmp + 1], "target").to_string();

    let advances: Vec<usize> = in_loop
        .filter(|&k| {
            op(&body[k]) == "add"
                && field(&body[k], "dst") == column
                && field(&body[k], "lhs") == column
        })
        .collect();
    assert_eq!(
        advances.len(),
        1,
        "premise: {DRAW_TEXT_SYMBOL} advances its running column ({column}) in one place"
    );
    Walk {
        body,
        labels,
        back_edge,
        loop_top,
        exit,
        index,
        column,
        control_cmp,
        control_target,
        column_advance: advances[0],
    }
}

impl Walk<'_> {
    fn label(&self, name: &str) -> usize {
        *self
            .labels
            .get(name)
            .unwrap_or_else(|| panic!("label {name} is defined in {DRAW_TEXT_SYMBOL}"))
    }

    /// Execute the control branch from its target to the loop back-edge, following
    /// unconditional branches. It must be linear: a control character makes no
    /// further decision.
    fn control_path(&self) -> Vec<usize> {
        let mut path = Vec::new();
        let mut k = self.label(&self.control_target);
        while k != self.back_edge {
            assert!(
                path.len() < 256,
                "the control branch of {DRAW_TEXT_SYMBOL} never reaches the loop back-edge"
            );
            let instruction = &self.body[k];
            assert!(
                !is_conditional_branch(instruction) && op(instruction) != "ret",
                "the control branch of {DRAW_TEXT_SYMBOL} must be linear, found {instruction} \
                 at {k}"
            );
            path.push(k);
            k = if op(instruction) == "b" {
                self.label(field(instruction, "target"))
            } else {
                k + 1
            };
        }
        path
    }

    /// The constant a register holds at `at` along `path` — `mov_imm` into a
    /// register and `str_u64`/`ldr_u64` through a stack slot, nothing else.
    fn constant_at(&self, path: &[usize], at: usize, register: &str) -> Option<i64> {
        let mut registers: HashMap<&str, Option<i64>> = HashMap::new();
        let mut slots: HashMap<&str, Option<i64>> = HashMap::new();
        for &k in path {
            if k == at {
                return registers.get(register).copied().flatten();
            }
            let instruction = &self.body[k];
            match op(instruction) {
                "mov_imm" => {
                    registers.insert(
                        field(instruction, "dst"),
                        field(instruction, "value").parse().ok(),
                    );
                }
                "str_u64" if field(instruction, "base") == "sp" => {
                    let value = registers.get(field(instruction, "src")).copied().flatten();
                    slots.insert(field(instruction, "offset"), value);
                }
                "ldr_u64" if field(instruction, "base") == "sp" => {
                    let value = slots.get(field(instruction, "offset")).copied().flatten();
                    registers.insert(field(instruction, "dst"), value);
                }
                _ => {
                    if let Some(dst) = instruction["dst"].as_str() {
                        registers.insert(dst, None);
                    }
                }
            }
        }
        None
    }
}

fn is_store_off_stack(instruction: &Value) -> bool {
    op(instruction).starts_with("str_") && field(instruction, "base") != "sp"
}

/// bug-594. A control character takes one column and stamps nothing: the control
/// branch must run the stamp path's own `add col, col, width` with `width == 1`,
/// store nothing outside the frame, call nothing, and still advance the UTF-16
/// index. Before the fix it jumped straight to the index advance and the column
/// never moved.
#[test]
fn macos_app_draw_text_control_character_advances_one_column_without_stamping() {
    for (name, source) in [
        ("macos_dt_ctrl_term", TERM_SOURCE),
        ("macos_dt_ctrl_plain", PLAIN_SOURCE),
    ] {
        let functions = app_ncode_functions(name, source);
        let body = functions
            .get(DRAW_TEXT_SYMBOL)
            .unwrap_or_else(|| panic!("{DRAW_TEXT_SYMBOL} is emitted by a macOS app build"));
        let walk = walk_of(body);
        let path = walk.control_path();

        assert!(
            path.contains(&walk.column_advance),
            "{name}: a control character in term::drawText must advance the running \
             column ({col}) — `mfb man term drawText`: \"they advance one column but \
             stamp nothing\" — but the control branch (`b.lt {target}`) never reaches the \
             column advance at instruction {adv} ({advance}). Control path: {path:?}",
            col = walk.column,
            target = walk.control_target,
            adv = walk.column_advance,
            advance = body[walk.column_advance],
        );
        let width = field(&body[walk.column_advance], "rhs");
        assert_eq!(
            walk.constant_at(&path, walk.column_advance, width),
            Some(1),
            "{name}: the control branch must advance the column by exactly ONE ({width} at \
             the advance)"
        );
        for &k in &path {
            assert!(
                !is_store_off_stack(&body[k]) && op(&body[k]) != "bl",
                "{name}: a control character must stamp nothing, but its branch runs {} at {k}",
                body[k]
            );
        }
        assert!(
            path.iter().any(|&k| {
                op(&body[k]) == "add"
                    && field(&body[k], "dst") == walk.index
                    && field(&body[k], "lhs") == walk.index
            }),
            "{name}: the control branch must still advance the UTF-16 index ({})",
            walk.index
        );
    }
}

/// The positive half: only the control step moved. The printable path still clips
/// at the right edge BEFORE it stamps, still drops a wide cluster that would
/// straddle the edge, still takes its width from the utf8proc table in a `term::`
/// build (a constant 1 without it), and still advances the column by that width.
#[test]
fn macos_app_draw_text_printable_path_still_clips_and_advances_by_display_width() {
    for (name, source, uses_term) in [
        ("macos_dt_print_term", TERM_SOURCE, true),
        ("macos_dt_print_plain", PLAIN_SOURCE, false),
    ] {
        let functions = app_ncode_functions(name, source);
        let body = functions
            .get(DRAW_TEXT_SYMBOL)
            .unwrap_or_else(|| panic!("{DRAW_TEXT_SYMBOL} is emitted by a macOS app build"));
        let walk = walk_of(body);
        let exit = walk.exit.as_str();
        let column = walk.column.as_str();

        // The first write into the grid after the control test.
        let first_stamp = (walk.control_cmp..walk.back_edge)
            .find(|&k| is_store_off_stack(&body[k]))
            .unwrap_or_else(|| panic!("{name}: the printable path stamps a cell"));
        assert!(
            first_stamp < walk.column_advance,
            "{name}: the column advances after the stamp, not before"
        );
        // Right-edge clip ahead of the stamp.
        assert!(
            (walk.control_cmp..first_stamp).any(|k| {
                op(&body[k]) == "cmp"
                    && field(&body[k], "lhs") == column
                    && op(&body[k + 1]) == "b.ge"
                    && field(&body[k + 1], "target") == exit
            }),
            "{name}: the printable path must end the run at the right edge before stamping"
        );
        // Wide-at-the-edge drop: `cmp_imm w, #2; b.ne narrow; add_imm t, col, #1;
        // cmp t, cols; b.ge exit`.
        assert!(
            (walk.control_cmp..first_stamp).any(|k| {
                op(&body[k]) == "cmp_imm"
                    && field(&body[k], "rhs") == "2"
                    && op(&body[k + 1]) == "b.ne"
                    && op(&body[k + 2]) == "add_imm"
                    && field(&body[k + 2], "src") == column
                    && op(&body[k + 3]) == "cmp"
                    && field(&body[k + 3], "lhs") == field(&body[k + 2], "dst")
                    && op(&body[k + 4]) == "b.ge"
                    && field(&body[k + 4], "target") == exit
            }),
            "{name}: a wide cluster whose trailing cell is off the grid must still be \
             dropped and end the run"
        );
        // The advance reads the width slot the printable path filled.
        let width = field(&body[walk.column_advance], "rhs");
        let reload = &body[walk.column_advance - 1];
        assert!(
            op(reload) == "ldr_u64"
                && field(reload, "base") == "sp"
                && field(reload, "dst") == width,
            "{name}: the column advance reads the display width back from its frame slot, \
             got {reload}"
        );
        let slot = field(reload, "offset");
        let fill = (walk.control_cmp + 2..first_stamp)
            .find(|&k| {
                op(&body[k]) == "str_u64"
                    && field(&body[k], "base") == "sp"
                    && field(&body[k], "offset") == slot
            })
            .unwrap_or_else(|| panic!("{name}: the printable path fills the width slot {slot}"));
        let looked_up = (walk.control_cmp + 2..fill).any(|k| {
            op(&body[k]) == "adrp" && field(&body[k], "symbol").starts_with("_mfb_unicode_")
        });
        assert_eq!(
            looked_up, uses_term,
            "{name}: the printable width comes from the utf8proc table exactly when the \
             program uses term:: (uses_term = {uses_term})"
        );
        // The loop itself is intact: its exit test is the first thing it does.
        assert_eq!(op(&body[walk.loop_top]), "label");
    }
}

/// `io::write`'s grid walk (`mfbWriteString:`) is a different IMP with a different
/// control contract ('\n' starts a new row). The drawText fix must not have leaked
/// into it: it shares no label with the drawText IMP and keeps its own newline test.
#[test]
fn macos_app_write_string_keeps_its_own_walk() {
    let functions = app_ncode_functions("macos_dt_write_string", TERM_SOURCE);
    let write = functions
        .get(WRITE_STRING_SYMBOL)
        .unwrap_or_else(|| panic!("{WRITE_STRING_SYMBOL} is emitted by a term:: app build"));
    assert!(
        !write
            .iter()
            .any(|instruction| field(instruction, "name").starts_with("dt_")
                || field(instruction, "target").starts_with("dt_")),
        "{WRITE_STRING_SYMBOL} must not share a label with the drawText IMP"
    );
    assert!(
        write
            .iter()
            .any(|instruction| op(instruction) == "cmp_imm" && field(instruction, "rhs") == "10"),
        "{WRITE_STRING_SYMBOL} must still test for '\\n'"
    );
}
