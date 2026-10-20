//! plan-176-O: lowering a V128 run with the RVV vector extension. The slot/vreg maps and the shared helpers live in
//! `riscv64/v128/mod.rs`.

use super::*;

/// plan-32-C: load the process-global `_mfb_rt_has_rvv` flag byte into `dst`
/// (`adrp`/`add_pageoff` the symbol, then a byte load).
fn load_has_rvv_flag(dst: &str) -> Vec<CodeInstruction> {
    let sym = crate::codegen::error::constants::HAS_RVV_GLOBAL_SYMBOL;
    vec![
        ci("adrp", &[("dst", dst), ("symbol", sym)]),
        ci(
            "add_pageoff",
            &[("dst", dst), ("src", dst), ("symbol", sym)],
        ),
        ci("ldr_u8", &[("dst", dst), ("base", dst), ("offset", "0")]),
    ]
}

/// plan-32-C: the native-RVV realization of one `v128` op, reading its operands
/// from and writing its result to the **same** per-thread slot region the scalar
/// arm uses (so the two arms reconcile at the slot). Returns `None` for an op this
/// pass does not yet vector-lower (compares, `BslV`/`BitV`, min/max, the
/// rounding/ties-away conversions, wide shifts) — the caller then emits the scalar
/// arm only, which is correctness-preserving. Only ops whose RVV per-lane result
/// is **bit-identical** to the scalar `f64`/`i64` op are lowered here: the RVV
/// arms run at the default RNE rounding, exactly like the scalar `fadd_d`/… they
/// mirror, and the integer/bitwise/mem ops are exact.
pub(super) fn rvv_arm(
    op: CodeOp,
    fields: &[(&'static str, Operand)],
    slots: &HashMap<String, usize>,
    vregs: &HashMap<String, u8>,
) -> Option<Vec<CodeInstruction>> {
    use CodeOp::*;
    // The caller emits the shared prologue (slot base into `t2`, then `vsetivli`)
    // once per run; this returns just this op's loads + compute + stores.
    let mut out = Vec::new();

    let vname = |value: &str| -> String {
        format!(
            "v{}",
            vregs
                .get(value)
                .copied()
                .unwrap_or_else(|| panic!("rv64 v128 rvv arm: no vreg for '{value}'"))
        )
    };
    let slot_off = |value: &str| -> String { (slots[value] * 16).to_string() };
    // Load `value`'s 16-byte (2×e64) slot into its assigned vector register.
    let load = |out: &mut Vec<CodeInstruction>, value: &str| {
        out.push(ci(
            "add_imm",
            &[("dst", T0), ("src", T2), ("imm", &slot_off(value))],
        ));
        out.push(ci(
            "rv.vop",
            &[("vop", "vle64.v"), ("dst", &vname(value)), ("base", T0)],
        ));
    };
    let store = |out: &mut Vec<CodeInstruction>, value: &str| {
        out.push(ci(
            "add_imm",
            &[("dst", T0), ("src", T2), ("imm", &slot_off(value))],
        ));
        out.push(ci(
            "rv.vop",
            &[("vop", "vse64.v"), ("src", &vname(value)), ("base", T0)],
        ));
    };

    match op {
        FAddV | FSubV | FMulV | FDivV => {
            let mn = match op {
                FAddV => "vfadd.vv",
                FSubV => "vfsub.vv",
                FMulV => "vfmul.vv",
                _ => "vfdiv.vv",
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        // Fused multiply-add/subtract (single rounding), matching the scalar
        // fmadd_d/fnmsub_d: vfmacc = vd += lhs*rhs, vfnmsac = vd -= lhs*rhs.
        FMlaV | FMlsV => {
            let mn = if op == FMlaV {
                "vfmacc.vv"
            } else {
                "vfnmsac.vv"
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &d); // the accumulator lane
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        // abs/neg via sign-injection (bit ops, exact); sqrt at RNE (exact).
        FAbsV | FNegV => {
            let mn = if op == FAbsV {
                "vfsgnjx.vv"
            } else {
                "vfsgnjn.vv"
            };
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&s)),
                    ("rhs", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        FSqrtV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vfsqrt.v"),
                    ("dst", &vname(&d)),
                    ("src", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        // f64→i64 toward zero and i64→f64: same RISC-V converters the scalar arm
        // uses (fcvtzs = fcvt.l.d RTZ; scvtf = fcvt.d.l), so bit-identical.
        FCvtzsV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vfcvt.rtz.x.f.v"),
                    ("dst", &vname(&d)),
                    ("src", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        ScvtfV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vfcvt.f.x.v"),
                    ("dst", &vname(&d)),
                    ("src", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        AddV | SubV | AndV | OrrV | EorV => {
            let mn = match op {
                AddV => "vadd.vv",
                SubV => "vsub.vv",
                AndV => "vand.vv",
                OrrV => "vor.vv",
                _ => "vxor.vv",
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        NegV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            // vrsub.vx vd, vs, x0 = 0 - vs.
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vrsub.vx"),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&s)),
                    ("gpr", ZERO),
                ],
            ));
            store(&mut out, &d);
        }
        // Immediate lane shifts, only for amounts the `.vi` uimm5 can encode
        // (0..=31); the scalar arm's shift-of-64 special case and any amount ≥32
        // fall back to scalar.
        ShlV | SshrV | UshrV => {
            let (d, s, sh) = (f(fields, "dst"), f(fields, "src"), f(fields, "shift"));
            match sh.parse::<u8>() {
                Ok(n) if n < 32 => {}
                _ => return None,
            }
            let mn = match op {
                ShlV => "vsll.vi",
                SshrV => "vsra.vi",
                _ => "vsrl.vi",
            };
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&s)),
                    ("imm", &sh),
                ],
            ));
            store(&mut out, &d);
        }
        DupVFromX => {
            let (d, s) = (f(fields, "dst"), f(fields, "src")); // s is a GPR
            out.push(ci(
                "rv.vop",
                &[("vop", "vmv.v.x"), ("dst", &vname(&d)), ("gpr", &s)],
            ));
            store(&mut out, &d);
        }
        UmovXFromV => {
            let (d, s, idx) = (f(fields, "dst"), f(fields, "src"), f(fields, "index")); // d is a GPR
            load(&mut out, &s);
            if idx == "0" {
                out.push(ci(
                    "rv.vop",
                    &[("vop", "vmv.x.s"), ("dst", &d), ("src", &vname(&s))],
                ));
            } else {
                // Reach lane 1 via the reserved v31 scratch, then extract element 0.
                out.push(ci(
                    "rv.vop",
                    &[
                        ("vop", "vslidedown.vi"),
                        ("dst", "v31"),
                        ("lhs", &vname(&s)),
                        ("imm", "1"),
                    ],
                ));
                out.push(ci(
                    "rv.vop",
                    &[("vop", "vmv.x.s"), ("dst", &d), ("src", "v31")],
                ));
            }
        }
        // 128-bit unit-stride load/store from an arbitrary address (base+offset).
        LdrQ => {
            let (d, base, o) = (f(fields, "dst"), f(fields, "base"), f(fields, "offset"));
            out.push(ci("add_imm", &[("dst", T0), ("src", &base), ("imm", &o)]));
            out.push(ci(
                "rv.vop",
                &[("vop", "vle64.v"), ("dst", &vname(&d)), ("base", T0)],
            ));
            store(&mut out, &d);
        }
        StrQ => {
            let (s, base, o) = (f(fields, "src"), f(fields, "base"), f(fields, "offset"));
            load(&mut out, &s);
            out.push(ci("add_imm", &[("dst", T0), ("src", &base), ("imm", &o)]));
            out.push(ci(
                "rv.vop",
                &[("vop", "vse64.v"), ("src", &vname(&s)), ("base", T0)],
            ));
        }
        // min/max: RVV vfmin/vfmax follow the same RISC-V minimumNumber/-0<+0
        // semantics as the scalar fminnm_d/fmaxnm_d, so they are bit-identical.
        FMinV | FMaxV => {
            let mn = if op == FMinV { "vfmin.vv" } else { "vfmax.vv" };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        // --- the mask bridge (plan-32-C Phase 3) ------------------------------
        // A compare writes a 1-bit-per-lane mask into v0, which is then
        // materialized into the NEON all-ones/all-zeros lane vector the scalar arm
        // produces (`vmv.v.i vd,0; vmerge.vim vd,vd,-1,v0`), stored to the slot.
        // `BslV`/`BitV` are then the same bit algebra as the scalar arm, so the
        // results match by construction.
        FCmGtV | FCmGeV | FCmEqV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            // gt: rhs<lhs ; ge: rhs<=lhs ; eq: lhs==rhs (all ordered → NaN false).
            let (mn, x, y) = match op {
                FCmGtV => ("vmflt.vv", &b, &a),
                FCmGeV => ("vmfle.vv", &b, &a),
                _ => ("vmfeq.vv", &a, &b),
            };
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", "v0"),
                    ("lhs", &vname(x)),
                    ("rhs", &vname(y)),
                ],
            ));
            lanes_from_mask(&mut out, &vname(&d));
            store(&mut out, &d);
        }
        FCmGtZeroV | FCmGeZeroV | FCmEqZeroV | FCmLtZeroV | FCmLeZeroV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            // v31 = +0.0 (integer 0 bits) to compare against.
            out.push(ci(
                "rv.vop",
                &[("vop", "vmv.v.i"), ("dst", "v31"), ("imm", "0")],
            ));
            let z = "v31".to_string();
            let sr = vname(&s);
            let (mn, x, y) = match op {
                FCmGtZeroV => ("vmflt.vv", &z, &sr), // 0 < a
                FCmGeZeroV => ("vmfle.vv", &z, &sr), // 0 <= a
                FCmEqZeroV => ("vmfeq.vv", &sr, &z), // a == 0
                FCmLtZeroV => ("vmflt.vv", &sr, &z), // a < 0
                _ => ("vmfle.vv", &sr, &z),          // a <= 0
            };
            out.push(ci(
                "rv.vop",
                &[("vop", mn), ("dst", "v0"), ("lhs", x), ("rhs", y)],
            ));
            lanes_from_mask(&mut out, &vname(&d));
            store(&mut out, &d);
        }
        CmGtV | CmGeV | CmEqV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            // gt: b<a ; ge: b<=a (signed) ; eq: a==b.
            let (mn, x, y) = match op {
                CmGtV => ("vmslt.vv", &b, &a),
                CmGeV => ("vmsle.vv", &b, &a),
                _ => ("vmseq.vv", &a, &b),
            };
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", "v0"),
                    ("lhs", &vname(x)),
                    ("rhs", &vname(y)),
                ],
            ));
            lanes_from_mask(&mut out, &vname(&d));
            store(&mut out, &d);
        }
        // bit-select: result = rhs ^ (mask & (lhs ^ rhs)); the mask is in dst.
        BslV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &d); // the incoming lane mask
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", "v31"),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            )); // a^b
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vand.vv"),
                    ("dst", "v31"),
                    ("lhs", "v31"),
                    ("rhs", &vname(&d)),
                ],
            )); // & mask
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&b)),
                    ("rhs", "v31"),
                ],
            )); // rhs ^ ...
            store(&mut out, &d);
        }
        // bit-insert-if-true: result = dst ^ ((dst ^ lhs) & mask); mask in rhs.
        BitV => {
            let (d, a, m) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &d);
            load(&mut out, &a);
            load(&mut out, &m);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", "v31"),
                    ("lhs", &vname(&d)),
                    ("rhs", &vname(&a)),
                ],
            )); // dst^lhs
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vand.vv"),
                    ("dst", "v31"),
                    ("lhs", "v31"),
                    ("rhs", &vname(&m)),
                ],
            )); // & mask
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&d)),
                    ("rhs", "v31"),
                ],
            )); // dst ^ ...
            store(&mut out, &d);
        }
        // Everything else (FRint*, FCvtasV, wide integer shifts, AbsV) is left
        // to the scalar arm. `SshlV`/`UshlV` are recognized `is_v128` ops with
        // no `scalarize_v128` arm, so they fail loud there rather than being
        // scalarized (pinned by `unhandled_v128_op_panics`).
        _ => return None,
    }
    Some(out)
}

/// plan-32-C Phase 3: turn the 1-bit-per-lane mask in `v0` into the NEON
/// all-ones/all-zeros lane vector in `vd` (`vmv.v.i vd,0; vmerge.vim vd,vd,-1,v0`),
/// so downstream `BslV`/`BitV`/`AndV` are the same bit algebra as the scalar arm.
/// `imm` 31 is the 5-bit `-1` the merge sign-extends to an all-ones lane.
fn lanes_from_mask(out: &mut Vec<CodeInstruction>, vd: &str) {
    out.push(ci(
        "rv.vop",
        &[("vop", "vmv.v.i"), ("dst", vd), ("imm", "0")],
    ));
    out.push(ci(
        "rv.vop",
        &[
            ("vop", "vmerge.vim"),
            ("dst", vd),
            ("src", vd),
            ("imm", "31"),
        ],
    ));
}

/// The shared RVV-arm prologue: the per-thread slot base into `t2`, then
/// `vsetivli x0, 2, e64, m1, ta, ma`. Emitted once per run (plan-32-D).
fn rvv_prologue(out: &mut Vec<CodeInstruction>) {
    out.push(ci(
        "add_imm",
        &[
            ("dst", T2),
            ("src", crate::arch::riscv64::regmodel::ARENA_BASE_REGISTER),
            (
                "imm",
                &crate::codegen::error::constants::ARENA_V128_SLOTS_OFFSET.to_string(),
            ),
        ],
    ));
    out.push(ci(
        "rv.vop",
        &[("vop", "vsetivli"), ("dst", "zero"), ("avl", "2")],
    ));
}

/// Whether `op` (with these `fields`) is realized on the native-RVV arm. A quick
/// predicate mirroring [`rvv_arm`]'s coverage, so the run builder can decide
/// membership without emitting; ops it rejects always scalarize.
pub(crate) fn rvv_lowerable(op: CodeOp, fields: &[(&'static str, Operand)]) -> bool {
    use CodeOp::*;
    match op {
        FAddV | FSubV | FMulV | FDivV | FMlaV | FMlsV | FAbsV | FNegV | FSqrtV | FCvtzsV
        | ScvtfV | AddV | SubV | AndV | OrrV | EorV | NegV | DupVFromX | UmovXFromV | LdrQ
        | StrQ | FMinV | FMaxV | FCmGtV | FCmGeV | FCmEqV | FCmGtZeroV | FCmGeZeroV
        | FCmEqZeroV | FCmLtZeroV | FCmLeZeroV | CmGtV | CmGeV | CmEqV | BslV | BitV => true,
        // Immediate lane shifts only when the `.vi` uimm5 can encode the amount.
        ShlV | SshrV | UshrV => matches!(f(fields, "shift").parse::<u8>(), Ok(n) if n < 32),
        _ => false,
    }
}

/// plan-32-D: within one RVV-arm body, drop a reload of a slot into a register
/// that already holds that slot's value. Each `v128` value owns its slot and its
/// register exclusively across a run (disjoint live ranges never share either),
/// so once a register has been loaded from — or stored to — its slot, a later
/// `add_imm t0,t2,off; vle64 vX,(t0)` for the *same* register+offset is redundant
/// until `vX` is overwritten by a compute. This is the register-residency that
/// turns a per-op memory round-trip into a resident chain; stores are kept (a
/// conservative choice that cannot miss a live-out spill).
fn drop_redundant_reloads(body: Vec<CodeInstruction>) -> Vec<CodeInstruction> {
    // reg → the slot offset it currently, provably, holds.
    let mut holds: HashMap<String, String> = HashMap::new();
    let mut keep = vec![true; body.len()];
    let mut i = 0;
    while i < body.len() {
        let inst = &body[i];
        // A slot access is `add_imm t0,t2,off` immediately followed by a
        // vle64/vse64 through t0.
        let is_slot_addr = inst.op.mnemonic() == "add_imm"
            && inst.get("dst").as_deref() == Some(T0)
            && inst.get("src").as_deref() == Some(T2);
        if is_slot_addr && i + 1 < body.len() {
            let off = inst.get("imm").unwrap_or_default();
            let next = &body[i + 1];
            if next.op.mnemonic() == "rv.vop" && next.get("base").as_deref() == Some(T0) {
                match next.get("vop").as_deref() {
                    Some("vle64.v") => {
                        let reg = next.get("dst").unwrap_or_default();
                        if holds.get(&reg) == Some(&off) {
                            // Register already holds this slot → drop the reload.
                            keep[i] = false;
                            keep[i + 1] = false;
                        } else {
                            holds.insert(reg, off);
                        }
                        i += 2;
                        continue;
                    }
                    Some("vse64.v") => {
                        let reg = next.get("src").unwrap_or_default();
                        holds.insert(reg, off);
                        i += 2;
                        continue;
                    }
                    _ => {}
                }
            }
        }
        // Any rv.vop that writes a vector register invalidates that register's
        // known slot value (it now holds a computed result). `vmv.x.s` writes a
        // GPR, not a vector reg, so it does not invalidate.
        if inst.op.mnemonic() == "rv.vop" && inst.get("vop").as_deref() != Some("vmv.x.s") {
            if let Some(dst) = inst.get("dst") {
                if dst.starts_with('v') {
                    holds.remove(&dst);
                }
            }
        }
        i += 1;
    }
    body.into_iter()
        .zip(keep)
        .filter_map(|(inst, k)| k.then_some(inst))
        .collect()
}

/// plan-32-C/-D: lower a maximal run of consecutive native-RVV-lowerable `v128`
/// ops to one runtime dual path — a single guard on `_mfb_rt_has_rvv`, then either
/// the RVV arm (one `vsetivli`, register-resident chain) or the scalar arm (the
/// unchanged per-op `scalarize_v128`), reconciled at the shared slot region.
/// `seq` makes the two arm labels unique within the function.
pub(crate) fn lower_v128_run(
    run: &[(CodeOp, Vec<(&'static str, Operand)>)],
    slots: &HashMap<String, usize>,
    vregs: &HashMap<String, u8>,
    seq: usize,
) -> Vec<CodeInstruction> {
    // Build the RVV arm: shared prologue + each op's body, then residency-optimize.
    let mut rvv_body = Vec::new();
    rvv_prologue(&mut rvv_body);
    for (op, fields) in run {
        rvv_body.extend(rvv_arm(*op, fields, slots, vregs).expect("run holds only lowerable ops"));
    }
    let rvv_body = drop_redundant_reloads(rvv_body);

    let scalar_label = format!("v128_scalar_{seq}");
    let done_label = format!("v128_done_{seq}");
    let mut out = load_has_rvv_flag(T0);
    // beqz t0, .scalar — the flag is clear on non-V hardware.
    out.push(ci(
        "rv.br",
        &[
            ("lhs", T0),
            ("rhs", ZERO),
            ("cond", "eq"),
            ("target", &scalar_label),
        ],
    ));
    out.extend(rvv_body);
    out.push(ci("b", &[("target", &done_label)]));
    out.push(ci("label", &[("name", &scalar_label)]));
    for (op, fields) in run {
        out.extend(scalarize_v128(*op, fields, slots));
    }
    out.push(ci("label", &[("name", &done_label)]));
    out
}
