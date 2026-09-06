//! bug-541: the "does nothing while TUI mode is off" gate, per app backend.
//!
//! `term::` has one module-wide rule (`mfb man term`, `mfb spec app term-backend`
//! §4.2.1): while TUI mode is off, every member except `term::on`, `term::isOn`
//! and `term::didResize` short-circuits — the setters and drawing calls do
//! nothing, and `term::terminalSize` raises `ErrUnsupported`. The console
//! backend enforces it with `emit_gate_inactive`, macOS with
//! `emit_term_active_gate`, and both read the SHARED arena term-state `active`
//! slot that all four backends already maintain correctly.
//!
//! Two app backends did not:
//!
//! * **Windows** gated its drawing bodies on the live `TUI_MEMDC` handle, whose
//!   lifetime is not TUI mode's (`term::off` never clears it), and gated
//!   `moveTo`, the setters, `sync` and `terminalSize` on nothing at all — so
//!   `term::terminalSize` answered `80x25` before any `term::on` instead of
//!   raising, which is the one difference a correct program can observe.
//! * **Linux GTK** gated every member except `term::off` itself, which scheduled
//!   its present and hide idles unconditionally, so a redundant `term::off`
//!   still asked the window to restore itself.
//!
//! These are codegen-inspection tests because that is what actually covers the
//! change: the byte-identity `.app.ncodesum` goldens say a hash MOVED, not what
//! moved, and the runtime effect of a gate is "nothing happened", which a
//! headless box cannot photograph. Each assertion below names the instruction
//! shape the gate has on that backend, so a body that loses its gate fails here
//! rather than in a screenshot nobody takes.

mod common;
use common::temp_project;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Every gated `term::` member, plus the reader the offset probe uses. Every
/// positioned member carries a non-`Light` style so a build cannot satisfy the
/// gate assertions with a body the compiler folded away.
const TERM_SOURCE: &str = "IMPORT io\nIMPORT term\nIMPORT color\n\nSUB main()\n  \
     term::on()\n  \
     term::setForeground(color::rgb(0, 255, 0))\n  \
     term::setBackground(color::rgb(0, 0, 0))\n  \
     term::setBold(TRUE)\n  \
     term::setUnderline(FALSE)\n  \
     term::showCursor()\n  \
     term::hideCursor()\n  \
     term::clear()\n  \
     term::moveTo(2, 4)\n  \
     term::drawHLine(term::LineStyle.Double, 5, 1, 12)\n  \
     term::drawVLine(term::LineStyle.HeavyDash, 6, 3, 10)\n  \
     term::drawBox(term::LineStyle.Double, 7, 2, 11, 20)\n  \
     term::fillRect(term::FillStyle.Dark, 8, 4, 10, 16)\n  \
     term::drawText(9, 6, \"app\")\n  \
     term::drawGlyph(10, 7, 9731)\n  \
     term::sync()\n  \
     LET size AS term::TermSize = term::terminalSize()\n  \
     io::print(toString(size.columns) & toString(term::isOn()) & toString(term::didResize()))\n  \
     term::off()\nEND SUB\n";

fn app_ncode_functions(name: &str, target: &str) -> serde_json::Map<String, Value> {
    let project = temp_project(name, TERM_SOURCE);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .args(["-app", "-target", target, "-ncode"])
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build -app -target {target} -ncode failed:\n{}\n{}",
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

/// The `(base, offset)` of the shared arena term-state `active` slot, read out of
/// `term::isOn` — the one member whose whole body is a load of that slot. Deriving
/// it rather than hard-coding it is deliberate: the term-state offset moves with
/// the arena layout, and a hard-coded number turns a layout change into a test
/// that passes while asserting nothing (`.ai/testing-gates.md`, the drifting
/// codegen-constant trap).
fn active_slot(functions: &serde_json::Map<String, Value>) -> (String, String) {
    let is_on = body(functions, "_mfb_rt_term_term_isOn");
    let load = is_on
        .iter()
        .rev()
        .find(|instruction| instruction["op"] == "ldr_u64")
        .expect("term::isOn loads the active flag");
    (
        load["base"].as_str().expect("base").to_string(),
        load["offset"].as_str().expect("offset").to_string(),
    )
}

/// Index of the first instruction that loads the shared `active` slot and the
/// index of the conditional branch that consumes it. `None` when the body never
/// tests the slot at all.
fn gate_position(instructions: &[Value], slot: &(String, String)) -> Option<usize> {
    let load = instructions.iter().position(|instruction| {
        instruction["op"] == "ldr_u64"
            && instruction["base"].as_str() == Some(slot.0.as_str())
            && instruction["offset"].as_str() == Some(slot.1.as_str())
    })?;
    // The branch that acts on it: the first conditional branch after the load.
    instructions
        .iter()
        .skip(load)
        .position(|instruction| {
            let op = instruction["op"].as_str().unwrap_or_default();
            op.starts_with("b.")
        })
        .map(|offset| load + offset)
}

/// The nine Windows members whose whole body must be skipped while TUI mode is
/// off, and the Win32 call each one makes when it is NOT skipped — the second
/// half is what makes this a gate test rather than a "body is empty" test.
const WINDOWS_GATED: &[(&str, &str)] = &[
    ("clear", "PatBlt"),
    ("sync", "InvalidateRect"),
    ("drawHLine", "TextOutW"),
    ("drawVLine", "TextOutW"),
    ("drawBox", "TextOutW"),
    ("fillRect", "TextOutW"),
    ("drawText", "TextOutW"),
    ("drawGlyph", "TextOutW"),
];

/// The Windows members with no Win32 call at all — pure term-state writers. They
/// were the least visible half of the gap: `term::moveTo` had no gate of any
/// kind, and the colour/attribute/cursor setters mutated the shared term-state
/// slots after `term::off`.
const WINDOWS_GATED_LEAVES: &[&str] = &[
    "moveTo",
    "setForeground",
    "setBackground",
    "setBold",
    "setUnderline",
    "showCursor",
    "hideCursor",
];

#[test]
fn windows_app_mode_term_bodies_gate_on_the_shared_active_slot() {
    let functions = app_ncode_functions("win_app_term_gate", "windows-x86_64");
    let slot = active_slot(&functions);
    for (member, call) in WINDOWS_GATED {
        let symbol = format!("_mfb_rt_term_term_{member}");
        let instructions = body(&functions, &symbol);
        let gate = gate_position(instructions, &slot).unwrap_or_else(|| {
            panic!(
                "term::{member} must test the shared active slot ({}+{}) — gating on the \
                 live TUI_MEMDC handle is not the same gate, because `term::off` never \
                 clears that handle (bug-541 GATE-02)",
                slot.0, slot.1
            )
        });
        // The gate is the FIRST thing the body does: a gate placed after the work
        // gates nothing.
        let first_call = instructions
            .iter()
            .position(|instruction| instruction["op"] == "bl")
            .unwrap_or(instructions.len());
        assert!(
            gate < first_call,
            "term::{member}: the active gate is at {gate}, after the body's first call \
             at {first_call} — the gate must precede the work it suppresses"
        );
        assert!(
            instructions.iter().any(|instruction| {
                instruction["op"] == "bl" && instruction["target"] == serde_json::json!(*call)
            }),
            "term::{member} must still call {call} on the active path — a gate that also \
             removed the work would satisfy the assertion above and draw nothing"
        );
    }
    for member in WINDOWS_GATED_LEAVES {
        let symbol = format!("_mfb_rt_term_term_{member}");
        let instructions = body(&functions, &symbol);
        assert!(
            gate_position(instructions, &slot).is_some(),
            "term::{member} must test the shared active slot ({}+{}) before writing \
             term state — it had no gate of any kind (bug-541 GATE-02)",
            slot.0,
            slot.1
        );
    }
}

/// bug-541 GATE-01, the one a correct program can observe: `term::terminalSize`
/// on Windows app mode returned a `TermSize` of the compile-time 80x25 grid
/// whether or not TUI mode was ever entered, so `TRY term::terminalSize()` never
/// took its fallback there and a program using the raise to detect "TUI not
/// entered" got a plausible-looking answer instead.
#[test]
fn windows_app_mode_terminal_size_raises_while_tui_mode_is_off() {
    let functions = app_ncode_functions("win_app_term_size", "windows-x86_64");
    let slot = active_slot(&functions);
    let instructions = body(&functions, "_mfb_rt_term_term_terminalSize");
    let gate = gate_position(instructions, &slot).expect(
        "term::terminalSize must test the shared active slot before answering (bug-541 GATE-01)",
    );
    let alloc = instructions
        .iter()
        .position(|instruction| {
            instruction["op"] == "bl"
                && instruction["target"] == serde_json::json!("_mfb_arena_alloc")
        })
        .expect("term::terminalSize allocates its TermSize record on the active path");
    assert!(
        gate < alloc,
        "term::terminalSize gates at {gate}, after allocating the record at {alloc}"
    );
    // The inactive branch raises rather than returning an inert size: the ERR tag
    // (1) and the `ErrUnsupported` code both appear as immediates.
    let (code, message) = mfb_error_unsupported();
    assert!(
        instructions.iter().any(|instruction| {
            instruction["op"] == "mov_imm" && instruction["value"] == serde_json::json!(code)
        }),
        "term::terminalSize must load the ErrUnsupported code ({code}) on its inactive \
         branch; body was:\n{instructions:#?}"
    );
    assert!(
        instructions
            .iter()
            .any(|instruction| { instruction["symbol"] == serde_json::json!(message) }),
        "term::terminalSize must load the ErrUnsupported message symbol ({message}) on \
         its inactive branch"
    );
}

/// `ErrUnsupported`'s code and message symbol. The emitter reads them through
/// `crate::codegen::registry::runtime_error_emission`, which an integration test
/// cannot call, so they are restated here — and cross-checked against the GTK
/// backend, which has raised this exact pair since bug-111
/// (`src/target/linux_gtk/app_io.rs:emit_app_term_terminal_size`). A rename that
/// moved only one of the two would fail that cross-check.
fn mfb_error_unsupported() -> (&'static str, &'static str) {
    ("77050007", "_mfb_str_error_unsupported")
}

/// bug-541 GATE-03: `term::off` on the GTK backend had no gate of its own, so a
/// redundant `term::off` — TUI mode already off — still scheduled the final
/// present and the hide idle. Every one of its siblings in the same file opens
/// with `emit_gtk_term_active_gate`; the console and macOS `term::off` bodies
/// both open with the same test.
#[test]
fn linux_app_mode_term_off_is_a_no_op_while_tui_mode_is_off() {
    let functions = app_ncode_functions("linux_app_term_off", "linux-aarch64");
    let instructions = body(&functions, "_mfb_rt_term_term_off");
    let first_idle = instructions
        .iter()
        .position(|instruction| {
            instruction["op"] == "bl" && instruction["target"] == serde_json::json!("g_idle_add")
        })
        .expect("GTK term::off schedules its present and hide idles with g_idle_add");
    let first_branch = instructions.iter().position(|instruction| {
        instruction["op"]
            .as_str()
            .unwrap_or_default()
            .starts_with("b.")
    });
    let first_branch = first_branch.unwrap_or_else(|| {
        panic!(
            "GTK term::off has no conditional branch at all — a redundant term::off \
             still schedules the present and hide idles (bug-541 GATE-03); body was:\n\
             {instructions:#?}"
        )
    });
    assert!(
        first_branch < first_idle,
        "GTK term::off tests nothing before scheduling its idles (gate at {first_branch}, \
         first g_idle_add at {first_idle})"
    );
    // The positive half: the gated body still does its work on the active path —
    // both idles, and the arena/app active flags cleared.
    assert_eq!(
        instructions
            .iter()
            .filter(|instruction| {
                instruction["op"] == "bl"
                    && instruction["target"] == serde_json::json!("g_idle_add")
            })
            .count(),
        2,
        "GTK term::off must still schedule BOTH the final present and the hide idle"
    );
}

/// The positive pin the whole change turns on: `term::on`, `term::isOn` and
/// `term::didResize` are the three members that answer either way, and adding a
/// module-wide gate must not reach them. `term::didResize` in particular must
/// keep reading `FALSE` before any `term::on` rather than no-oping into an
/// undefined value.
#[test]
fn the_three_ungated_term_members_stay_ungated_on_every_app_backend() {
    for target in ["windows-x86_64", "linux-aarch64", "macos-aarch64"] {
        let name = format!("app_term_ungated_{}", target.replace('-', "_"));
        let functions = app_ncode_functions(&name, target);
        let slot = active_slot(&functions);

        // `term::on` is the gate: it must not test the flag it is about to set.
        let on = body(&functions, "_mfb_rt_term_term_on");
        assert!(
            gate_position(on, &slot).is_none(),
            "{target}: term::on must not be gated on the active slot — it is the call \
             that sets it"
        );

        // `term::isOn` READS the slot, so it cannot be recognised by "does not touch
        // it". What it must not do is branch: it answers with the flag's value.
        let is_on = body(&functions, "_mfb_rt_term_term_isOn");
        assert!(
            !is_on.iter().any(|instruction| {
                instruction["op"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("b.")
            }),
            "{target}: term::isOn must answer either way, with no gate branch"
        );

        // `term::didResize` reads and clears its own flag, never the active flag.
        let did_resize = body(&functions, "_mfb_rt_term_term_didResize");
        assert!(
            gate_position(did_resize, &slot).is_none(),
            "{target}: term::didResize must answer either way — reading FALSE before any \
             term::on is the contract, not raising and not no-oping"
        );
    }
}
