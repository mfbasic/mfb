//! plan-32-C/-D: `select_riscv64` must emit the **native-RVV arm** of the v128
//! dual-path lowering for a function whose SIMD run fits the `v1`–`v30` vector
//! register pool, not just the scalar fallback. The transcendental math kernels
//! overflow that pool (so they always scalarize), which left the RVV arm
//! (`v128::rvv_arm` / `lower_v128_run` / the mask bridge / `drop_redundant_reloads`)
//! reachable only through a low-register-pressure riscv64 cross-build — a path no
//! test previously exercised, so the whole native-RVV emitter was uncovered in
//! the non-`cfg(test)` build even though its unit tests pass.
//!
//! Each builtin below lowers to a short run of *rvv-lowerable* v128 ops, so
//! `build_vreg_map` succeeds and the dual path is emitted. The `.ncode` dump is
//! inspected directly (no riscv64 host needed): we assert the RVV mnemonics that
//! only the native arm produces are present, which fails loudly if the dual path
//! ever silently degrades to scalar-only.

mod common;
use common::build_ncode;
use serde_json::Value;
use std::fs;

/// A single function's worth of low-pressure, rvv-lowerable SIMD, spanning the
/// float three-same (`min`/`max`), two-reg-misc (`abs`/`sqrt`), float→int
/// convert (`floor`), and the integer compare + mask-bridge + bit-select path
/// (`min`/`max` on `List OF Integer`). Together they drive most of `rvv_arm`.
const SOURCE: &str = "IMPORT io\n\
IMPORT math\n\
\n\
FUNC main AS Integer\n\
  LET af AS List OF Float = [1.0, 2.0, 3.0, 4.0]\n\
  LET bf AS List OF Float = [4.0, 3.0, 2.0, 1.0]\n\
  LET mn AS List OF Float = math::min(af, bf)\n\
  LET mx AS List OF Float = math::max(af, bf)\n\
  LET ab AS List OF Float = math::abs(af)\n\
  LET sq AS List OF Float = math::sqrt(af)\n\
  LET fl AS List OF Integer = math::floor(af)\n\
  LET ai AS List OF Integer = [1, 2, 3, 4]\n\
  LET bi AS List OF Integer = [4, 3, 2, 1]\n\
  LET imn AS List OF Integer = math::min(ai, bi)\n\
  LET imx AS List OF Integer = math::max(ai, bi)\n\
  io::print(toString(len(mn) + len(mx) + len(ab) + len(sq) + len(fl) + len(imn) + len(imx)))\n\
  RETURN 0\n\
END FUNC\n";

/// Every `vop` mnemonic across every function in an `.ncode` dump.
fn vops(ncode: &Value) -> Vec<String> {
    ncode["functions"]
        .as_array()
        .expect("ncode has a functions array")
        .iter()
        .flat_map(|func| {
            func["instructions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|inst| inst.get("vop").and_then(Value::as_str))
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn riscv64_low_pressure_simd_emits_the_native_rvv_arm() {
    let name = "v128_rvv_codegen";
    let project = common::temp_project(name, SOURCE);
    let ncode = build_ncode(&project, "linux-riscv64", name);
    let vops = vops(&ncode);
    let has = |mn: &str| vops.iter().any(|v| v == mn);

    // The RVV arm is entered at all: its per-run `vsetivli` prologue and the
    // slot<->register load/store the scalar arm never emits.
    assert!(
        has("vsetivli"),
        "no vsetivli — the native-RVV arm was not emitted"
    );
    assert!(
        has("vle64.v") && has("vse64.v"),
        "missing RVV slot load/store"
    );

    // Float arms: three-same min/max, two-reg-misc abs (sign-inject) and sqrt,
    // and the float→int convert (toward zero) that `math::floor`'s lowering uses.
    for mn in [
        "vfmin.vv",
        "vfmax.vv",
        "vfsgnjx.vv",
        "vfsqrt.v",
        "vfcvt.rtz.x.f.v",
    ] {
        assert!(has(mn), "missing float RVV op {mn}");
    }

    // The mask bridge + bit-select + integer compare path (`math::min`/`max` on
    // a `List OF Integer`): a `vmslt` mask into v0, materialized to lane vectors
    // via `vmv.v.i`/`vmerge.vim`, then `vand`/`vor`/`vxor` bit algebra.
    for mn in [
        "vmslt.vv",
        "vmv.v.i",
        "vmerge.vim",
        "vand.vv",
        "vor.vv",
        "vxor.vv",
    ] {
        assert!(has(mn), "missing integer/mask RVV op {mn}");
    }

    // Lane broadcast (`vmv.v.x`) and lane-1 extraction (`vslidedown.vi` +
    // `vmv.x.s`), emitted by the list constructors / element reads.
    assert!(has("vmv.v.x"), "missing dup broadcast vmv.v.x");
    assert!(
        has("vslidedown.vi") && has("vmv.x.s"),
        "missing lane extract"
    );

    let _ = fs::remove_dir_all(&project);
}
