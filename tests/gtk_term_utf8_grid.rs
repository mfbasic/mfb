//! Regression tests for bug-203: the linux-gtk `term::` grid stored and rendered
//! ONE UTF-8 BYTE per cell, so a non-ASCII glyph (e.g. the box-drawing U+2500,
//! ubiquitous in TUIs) was split across cells as lone continuation bytes, drawn
//! as invalid-UTF-8 tofu, and advanced the cursor by its byte count instead of
//! one column.
//!
//! A char cell is now a `u32` holding one code point's UTF-8 bytes packed
//! little-endian, so a `str_u32` into a 5-byte buffer lays the sequence out in
//! order for `cairo_show_text`, NUL-terminated.
//!
//! These assert on the emitted code plan, not on pixels. Rendering is a Cairo
//! draw callback that needs a real display; the GTK VM available here has no
//! reachable X server (only console users may start one) and a `term::` app
//! crashes under the headless `broadwayd` backend — a PRE-EXISTING failure that
//! reproduces identically on a pre-bug-203 baseline binary. So the glyph
//! actually painting one column is NOT covered by an automated test; it needs a
//! manual `-app` run on a GTK desktop.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Writes box-drawing and accented glyphs while TUI mode is on.
const TERM_SOURCE: &str = "IMPORT term\nIMPORT io\n\n\
     SUB main()\n\
    \x20 term::on()\n\
    \x20 io::print(\"\\u{2500}\\u{253C} h\\u{E9}llo\")\n\
    \x20 term::sync()\n\
    \x20 term::off()\n\
     END SUB\n";

fn build_ncode(name: &str) -> serde_json::Value {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), TERM_SOURCE).expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg("-app")
        .args(["-target", "linux-x86_64"])
        .arg("-ncode")
        .arg(&root)
        .output()
        .expect("run mfb build -app -ncode");
    assert!(
        output.status.success(),
        "gtk term app build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let plan = fs::read_to_string(root.join(format!("{name}.ncode"))).expect("read ncode");
    let value = serde_json::from_str(&plan).expect("parse ncode json");
    let _ = fs::remove_dir_all(&root);
    value
}

fn instructions<'a>(plan: &'a serde_json::Value, symbol: &str) -> &'a Vec<serde_json::Value> {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"] == symbol)
        .unwrap_or_else(|| panic!("no {symbol} in code plan"))["instructions"]
        .as_array()
        .expect("instructions array")
}

fn ops(instructions: &[serde_json::Value]) -> Vec<&str> {
    instructions
        .iter()
        .filter_map(|i| i["op"].as_str())
        .collect()
}

/// The runtime state carries three live grids and three snapshot grids. With a
/// u32 char cell, chars costs the same as fg/bg — so `_mfb_gtkapp_state` grows by
/// exactly 2 * 160 * 48 * 3 bytes (live + snapshot) over the 1-byte layout.
/// Pinning the size catches a stride/offset constant drifting out of sync with
/// the memset/memcpy sizes, which would silently corrupt neighbouring state.
#[test]
fn gtk_state_sizes_the_char_grid_at_four_bytes_per_cell() {
    let plan = build_ncode("gtk_term_state");
    let objects = plan["data"]
        .as_array()
        .or_else(|| plan["dataObjects"].as_array())
        .expect("data objects array");
    let state = objects
        .iter()
        .find(|o| o["symbol"] == "_mfb_gtkapp_state")
        .expect("no _mfb_gtkapp_state data object");
    let size = state["size"].as_u64().expect("state size");

    const CELLS: u64 = 160 * 48;
    // handles(7) + argc/argv + mode + lineLen = 11 u64, then the 1024B line
    // buffer, 13 u64 of term geometry, then chars/fg/bg live + snapshot at 4B.
    let expected = 11 * 8 + 1024 + 13 * 8 + 6 * CELLS * 4;
    assert_eq!(
        size, expected,
        "the char grid should be 4 bytes/cell like fg/bg (bug-203); \
         got size={size}, expected {expected}"
    );
}

/// The write path must decode a whole code point per cell and advance the cursor
/// one column, instead of storing each byte into its own cell.
#[test]
fn gtk_term_write_decodes_a_code_point_per_cell() {
    let plan = build_ncode("gtk_term_write");
    let write = instructions(&plan, "_mfb_gtkapp_term_write");
    let names: Vec<&str> = write
        .iter()
        .filter_map(|i| i["name"].as_str())
        .collect();
    for label in ["u8_len_done", "u8_len_ok", "u8_pack_done"] {
        assert!(
            names.contains(&label),
            "term_write should decode UTF-8 (missing label {label}); labels: {names:?}"
        );
    }

    // The lead-byte length ladder: <0xC0 -> 1, <0xE0 -> 2, <0xF0 -> 3, else 4.
    let ladder: Vec<&str> = write
        .iter()
        .filter(|i| i["op"] == "cmp_imm")
        .filter_map(|i| i["rhs"].as_str())
        .collect();
    for boundary in ["192", "224", "240"] {
        assert!(
            ladder.contains(&boundary),
            "term_write should classify the lead byte against {boundary}; saw {ladder:?}"
        );
    }

    // The cell is written as a 32-bit glyph, never as a lone byte.
    assert!(
        ops(write).contains(&"str_u32"),
        "term_write should store a packed u32 glyph per cell"
    );
    assert!(
        !write
            .iter()
            .any(|i| i["op"] == "str_u8" && i["offset"] == "0"),
        "term_write must not store a bare byte into a grid cell (bug-203)"
    );
}

/// The draw path must hand cairo the whole cell, NUL-terminated.
#[test]
fn gtk_term_draw_renders_the_whole_cell() {
    let plan = build_ncode("gtk_term_draw");
    let draw = instructions(&plan, "_mfb_gtkapp_term_draw");
    let op_list = ops(draw);
    assert!(
        op_list.contains(&"ldr_u32"),
        "term_draw should load the full u32 cell"
    );
    assert!(
        op_list.contains(&"str_u32"),
        "term_draw should stage the cell's bytes into the glyph buffer as a u32"
    );
    // A single-byte load of a cell followed by a 2-byte NUL-terminated buffer was
    // the bug; the buffer's terminator now sits at +4.
    assert!(
        draw.iter()
            .any(|i| i["op"] == "str_u8" && i["offset"].as_str() == Some("100")),
        "term_draw should NUL-terminate the glyph buffer after 4 bytes (off_buf+4)"
    );
}
