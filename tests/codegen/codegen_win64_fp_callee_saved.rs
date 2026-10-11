//! bug-994: on `windows-x86_64` every function saves each callee-saved `xmm`
//! register its final instruction stream writes.
//!
//! Win64 makes `xmm6`–`xmm15` callee-saved, and the register allocator relies on
//! it: a caller keeps a `Float` in `xmm6`–`xmm14` across a call. The math and SIMD
//! kernels stage lanes in the fixed tokens `FP_SCRATCH[6..7]`/`VEC_SCRATCH[6..7]`,
//! which realize to `xmm6`/`xmm7` on x86-64, and the frame's save set used to
//! cover only the registers the allocator chose. So `math::sin`, `math::pow` or a
//! `Fixed`→`Float` conversion inside a callee silently replaced the caller's
//! `Float` (`10.00 20.00` printed as `10.00 -0.09`).
//!
//! `xmm15` is not checked: it is the encoder's private SSE scratch, written inside
//! instruction encodings and invisible in the `-ncode` dump, and the allocator
//! never homes a value there (bug-994 Blast Radius).

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, build_ncode_app, temp_project};
use serde_json::Value;
use std::collections::BTreeSet;

/// bug-994's reduced reproduction: three callees that inline a sin kernel, a pow
/// kernel and a Fixed→Float conversion, and a caller holding four Floats across
/// calls to them.
const REPRO: &str = "\
IMPORT io\n\
IMPORT math\n\
IMPORT os\n\
FUNC viaSin(x AS Float, n AS Integer) AS Float\n\
  MUT s AS Float = 0.0\n\
  FOR i = 0 TO n - 1\n\
    s = s + math::sin(x + toFloat(i))\n\
  NEXT\n\
  RETURN s\n\
END FUNC\n\
FUNC viaPow(x AS Float, n AS Integer) AS Float\n\
  MUT s AS Float = 0.0\n\
  FOR i = 0 TO n - 1\n\
    s = s + math::pow(x + 1.5, toFloat(i) + 0.5)\n\
  NEXT\n\
  RETURN s\n\
END FUNC\n\
FUNC viaFixed(x AS Fixed, n AS Integer) AS Float\n\
  MUT s AS Float = 0.0\n\
  FOR i = 0 TO n - 1\n\
    s = s + toFloat(x) * 3.0\n\
  NEXT\n\
  RETURN s\n\
END FUNC\n\
FUNC probe(which AS Integer) AS String\n\
  LET k AS Float = toFloat(len(os::args()))\n\
  MUT a AS Float = 0.0\n\
  MUT b AS Float = 0.0\n\
  MUT c AS Float = 0.0\n\
  MUT d AS Float = 0.0\n\
  MUT w AS Float = 0.0\n\
  MUT i AS Integer = 0\n\
  DO WHILE i < 2\n\
    IF i = 0 THEN\n\
      a = k + 10.0\n\
      b = k + 20.0\n\
      c = k + 30.0\n\
      d = k + 40.0\n\
    ELSEIF which = 1 THEN\n\
      w = viaSin(k, 3)\n\
    ELSEIF which = 2 THEN\n\
      w = viaPow(k, 3)\n\
    ELSE\n\
      w = viaFixed(toFixed(k + 0.25), 3)\n\
    END IF\n\
    i = i + 1\n\
  LOOP\n\
  RETURN toString(which) & \": \" & toString(a) & \" \" & toString(b) & \" \" & toString(c) & \" \" & toString(d)\n\
END FUNC\n\
FUNC main AS Integer\n\
  FOR m = 1 TO 3\n\
    io::print(probe(m))\n\
  NEXT\n\
  RETURN 0\n\
END FUNC\n";

/// A canvas app program: its Vulkan helpers (`_mfb_rt_canvas_canvas_vulkan*`)
/// are hand-built `finalize_vreg_body_with_locals` bodies that stage floats in
/// `FP_SCRATCH[7]`, so the fix must reach that path as well as builder bodies.
const CANVAS: &str = "\
IMPORT app\n\
IMPORT canvas\n\
IMPORT color\n\
IMPORT vector\n\
FUNC main() AS Nothing\n\
  app::setMode(app::Mode.Canvas)\n\
  canvas::present([canvas::Circle[id := \"\", position := vector::Float2[x := 60.0, y := 40.0], radius := 6.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])\n\
END FUNC\n";

fn win64_allocatable_callee_saved_xmm() -> BTreeSet<String> {
    (6..=14).map(|i| format!("xmm{i}")).collect()
}

/// Every function that writes a callee-saved xmm register lists it in its frame
/// save set. Returns the offenders as `symbol: [registers]`.
fn unsaved_xmm_writes(ncode: &Value) -> Vec<String> {
    let watched = win64_allocatable_callee_saved_xmm();
    let mut offenders = Vec::new();
    for function in ncode["functions"].as_array().expect("functions") {
        let symbol = function["symbol"].as_str().unwrap_or("?");
        let saved: BTreeSet<&str> = function["frame"]["calleeSaved"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut missing = BTreeSet::new();
        for instruction in function["instructions"].as_array().expect("instructions") {
            if let Some(register) = instruction.get("dst").and_then(Value::as_str) {
                if watched.contains(register) && !saved.contains(register) {
                    missing.insert(register.to_string());
                }
            }
        }
        if !missing.is_empty() {
            offenders.push(format!("{symbol}: {missing:?}"));
        }
    }
    offenders
}

fn saves(ncode: &Value, symbol: &str, register: &str) -> bool {
    ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("no `{symbol}` in the dump"))["frame"]["calleeSaved"]
        .as_array()
        .is_some_and(|a| a.iter().any(|r| r.as_str() == Some(register)))
}

#[test]
fn every_win64_function_saves_the_callee_saved_xmm_it_writes() {
    let project = temp_project("win64_fp_callee_saved", REPRO);
    let ncode = build_ncode(&project, "windows-x86_64", "win64_fp_callee_saved");
    let _ = std::fs::remove_dir_all(&project);
    let offenders = unsaved_xmm_writes(&ncode);
    assert!(
        offenders.is_empty(),
        "functions write a callee-saved xmm register their frame does not save:\n  {}",
        offenders.join("\n  ")
    );
    // The two the reproduction observed: a sin kernel and a Fixed→Float
    // conversion each write `xmm7` through `FP_SCRATCH[7]`.
    for symbol in ["_mfb_fn_viaSin", "_mfb_fn_viaFixed"] {
        assert!(
            saves(&ncode, symbol, "xmm7"),
            "`{symbol}` does not save xmm7"
        );
    }
}

#[test]
fn win64_hand_built_helpers_save_the_callee_saved_xmm_they_write() {
    let project = common::temp_project("win64_fp_callee_saved_canvas", CANVAS);
    let ncode = build_ncode_app(&project, "windows-x86_64", "win64_fp_callee_saved_canvas");
    let _ = std::fs::remove_dir_all(&project);
    let offenders = unsaved_xmm_writes(&ncode);
    assert!(
        offenders.is_empty(),
        "functions write a callee-saved xmm register their frame does not save:\n  {}",
        offenders.join("\n  ")
    );
    assert!(saves(
        &ncode,
        "_mfb_rt_canvas_canvas_vulkanDrawScene",
        "xmm7"
    ));
}

/// The fix must not reach any other target: their scratch realizations are
/// caller-saved, so no frame there may save an FP register it did not save
/// before. Checked as "no xmm in a SysV frame".
#[test]
fn sysv_frames_save_no_xmm() {
    let project = temp_project("sysv_fp_callee_saved", REPRO);
    let ncode = build_ncode(&project, "linux-x86_64", "sysv_fp_callee_saved");
    let _ = std::fs::remove_dir_all(&project);
    for function in ncode["functions"].as_array().expect("functions") {
        let saved = function["frame"]["calleeSaved"].as_array();
        assert!(
            !saved.is_some_and(|a| a
                .iter()
                .any(|r| r.as_str().is_some_and(|r| r.starts_with("xmm")))),
            "`{}` saves an xmm register under SysV, where every xmm is caller-saved",
            function["symbol"]
        );
    }
}
