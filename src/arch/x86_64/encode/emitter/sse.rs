//! plan-176-O: the x86-64 encoder's SSE/AVX encoders. `Encoder`, the REX/ModRM/SIB helpers and
//! the `InstructionEncoder` impl live in `encode/emitter/mod.rs`.

use super::*;

/// SSE reg-reg: `[prefix] [REX] 0F opcode modrm(11,reg,rm)` on xmm indices.
/// REX.R extends `reg_x`, REX.B extends `rm_x` (needed for xmm8-15); a REX byte
/// is only emitted when one of them is high.
pub(super) fn enc_sse_rr(prefix: Option<u8>, opcode: u8, reg_x: u8, rm_x: u8) -> Vec<u8> {
    let mut b = Vec::new();
    if let Some(p) = prefix {
        b.push(p);
    }
    if reg_x >= 8 || rm_x >= 8 {
        b.push(rex(false, reg_x >= 8, false, rm_x >= 8));
    }
    b.push(0x0f);
    b.push(opcode);
    b.push(modrm(0b11, reg_x, rm_x));
    b
}

/// `movq xmm, r64` — 66 REX.W 0F 6E /r (reg = xmm, rm = r64).
pub(super) fn enc_movq_xmm_r64(dst_x: u8, src_r: u8) -> Vec<u8> {
    vec![
        0x66,
        rex(true, dst_x >= 8, false, src_r >= 8),
        0x0f,
        0x6e,
        modrm(0b11, dst_x, src_r),
    ]
}

/// `movq r64, xmm` — 66 REX.W 0F 7E /r, MR form (reg = xmm src, rm = r64 dst).
pub(super) fn enc_movq_r64_xmm(dst_r: u8, src_x: u8) -> Vec<u8> {
    vec![
        0x66,
        rex(true, src_x >= 8, false, dst_r >= 8),
        0x0f,
        0x7e,
        modrm(0b11, src_x, dst_r),
    ]
}

/// `roundsd dst_x, src_x, mode` — SSE4.1 directed rounding: 66 [REX] 0F 3A 0B /r
/// ib. `mode` low 2 bits select the rounding (0=nearest-even, 1=−∞, 2=+∞,
/// 3=trunc); bit 2 clear means the immediate mode is used (not MXCSR).
pub(super) fn enc_roundsd(dst_x: u8, src_x: u8, mode: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if dst_x >= 8 || src_x >= 8 {
        b.push(rex(false, dst_x >= 8, false, src_x >= 8));
    }
    b.extend_from_slice(&[0x0F, 0x3A, 0x0B, modrm(0b11, dst_x, src_x), mode]);
    b
}

/// `shift r64, imm8` — REX.W C1 /digit ib (digit: 4=SHL, 5=SHR, 7=SAR).
pub(super) fn enc_shift_imm_reg(digit: u8, reg_n: u8, imm: u8) -> Vec<u8> {
    vec![
        rex(true, false, false, reg_n >= 8),
        0xC1,
        modrm(0b11, digit, reg_n),
        imm,
    ]
}

/// SSE convert with REX.W: cvtsi2sd (dst xmm, src r64) / cvttsd2si (dst r64, src
/// xmm). Both are Intel RM form (reg = dst, rm = src), so REX.R extends dst and
/// REX.B extends src regardless of which class each is.
pub(super) fn enc_sse_cvt(prefix: u8, opcode: u8, _dst_is_xmm: bool, dst: u8, src: u8) -> Vec<u8> {
    vec![
        prefix,
        rex(true, dst >= 8, false, src >= 8),
        0x0f,
        opcode,
        modrm(0b11, dst, src),
    ]
}

/// `psllq/psrlq xmm, imm8` — 66 [REX.B] 0F 73 /`ext` ib (ext = 6 shifts left, 2
/// shifts right the whole 64-bit lane). Used to build |x| without a mask.
pub(super) fn enc_psxlq(ext: u8, xmm: u8, imm: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if xmm >= 8 {
        b.push(rex(false, false, false, true));
    }
    b.push(0x0f);
    b.push(0x73);
    b.push(modrm(0b11, ext, xmm));
    b.push(imm);
    b
}

/// `movsd xmm, [base+disp]` (opcode 0x10) or `movsd [base+disp], xmm` (0x11) —
/// F2 [REX] 0F op with a `[base+disp32]` memory operand.
pub(super) fn enc_movsd_mem(opcode: u8, xmm: u8, base: u8, disp: i32) -> Vec<u8> {
    let mut b = vec![0xf2];
    if xmm >= 8 || base >= 8 {
        b.push(rex(false, xmm >= 8, false, base >= 8));
    }
    b.push(0x0f);
    b.push(opcode);
    b.extend_from_slice(&mem_disp32(xmm, base, disp));
    b
}

/// 128-bit unaligned load/store `movups` (0F 10 load / 11 store) — no prefix.
pub(super) fn enc_movups_mem(opcode: u8, xmm: u8, base: u8, disp: i32) -> Vec<u8> {
    let mut b = Vec::new();
    if xmm >= 8 || base >= 8 {
        b.push(rex(false, xmm >= 8, false, base >= 8));
    }
    b.push(0x0f);
    b.push(opcode);
    b.extend_from_slice(&mem_disp32(xmm, base, disp));
    b
}

/// `movaps dst, src` (0F 28 /r) — a 128-bit register copy.
pub(super) fn enc_movaps(dst: u8, src: u8) -> Vec<u8> {
    let mut b = Vec::new();
    if dst >= 8 || src >= 8 {
        b.push(rex(false, dst >= 8, false, src >= 8));
    }
    b.extend_from_slice(&[0x0f, 0x28, modrm(0b11, dst, src)]);
    b
}

/// Three-byte SSE (66 0F 38 opcode /r) — pcmpgtq (0x37), pcmpeqq (0x29).
pub(super) fn enc_sse38_rr(opcode: u8, reg_x: u8, rm_x: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if reg_x >= 8 || rm_x >= 8 {
        b.push(rex(false, reg_x >= 8, false, rm_x >= 8));
    }
    b.extend_from_slice(&[0x0f, 0x38, opcode, modrm(0b11, reg_x, rm_x)]);
    b
}

/// `cmppd dst, src, pred` (66 0F C2 /r ib) — packed f64 compare to a lane mask.
pub(super) fn enc_cmppd(dst: u8, src: u8, pred: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if dst >= 8 || src >= 8 {
        b.push(rex(false, dst >= 8, false, src >= 8));
    }
    b.extend_from_slice(&[0x0f, 0xc2, modrm(0b11, dst, src), pred]);
    b
}

/// `pshufd dst, src, imm` (66 0F 70 /r ib) — shuffle 32-bit lanes. `imm=0xEE`
/// copies the high 64 bits (dwords 2,3) into the low 64 (a "high lane → lane 0").
pub(super) fn enc_pshufd(dst: u8, src: u8, imm: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if dst >= 8 || src >= 8 {
        b.push(rex(false, dst >= 8, false, src >= 8));
    }
    b.extend_from_slice(&[0x0f, 0x70, modrm(0b11, dst, src), imm]);
    b
}

/// FMA3 packed-double 231-form `dst = ±(lhs*rhs) + dst` via 3-byte VEX:
/// `VEX.128.66.0F38.W1 opcode /r` with reg=dst, vvvv=lhs, rm=rhs. `opcode` is
/// 0xB8 (vfmadd231pd) or 0xBC (vfnmadd231pd = dst − lhs*rhs). Requires FMA3.
pub(super) fn enc_vfma231pd(opcode: u8, dst: u8, lhs: u8, rhs: u8) -> Vec<u8> {
    // P0: [~R][~X=1][~B][mmmmm=00010 (0F38)]; ~R extends reg(dst), ~B extends rm(rhs).
    let r_bar = if dst >= 8 { 0 } else { 1 };
    let b_bar = if rhs >= 8 { 0 } else { 1 };
    let p0 = (r_bar << 7) | (1 << 6) | (b_bar << 5) | 0b00010;
    // P1: [W=1][vvvv=~lhs][L=0][pp=01 (66)].
    let vvvv = (!(lhs as u32)) & 0xF;
    let p1 = (1u8 << 7) | ((vvvv as u8) << 3) | 0b01;
    vec![0xC4, p0, p1, opcode, modrm(0b11, dst, rhs)]
}

/// Scalar-double 231-form FMA. The VEX prefix is identical to the packed-double
/// form (`VEX.128.66.0F38.W1`): the scalar/packed distinction is only the opcode
/// LSB (odd = `sd`), and the `L` bit is ignored for scalar, so the encoded bytes
/// match `enc_vfma231pd` for the same opcode. `dst = ±(lhs*rhs) ± dst`.
pub(super) fn enc_vfma231sd(opcode: u8, dst: u8, lhs: u8, rhs: u8) -> Vec<u8> {
    enc_vfma231pd(opcode, dst, lhs, rhs)
}

/// `roundpd dst, src, mode` (SSE4.1 66 0F 3A 09 /r ib).
pub(super) fn enc_roundpd(dst: u8, src: u8, mode: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if dst >= 8 || src >= 8 {
        b.push(rex(false, dst >= 8, false, src >= 8));
    }
    b.extend_from_slice(&[0x0f, 0x3a, 0x09, modrm(0b11, dst, src), mode]);
    b
}

/// Parse the `dst`/`lhs`/`rhs` fields of a three-operand SIMD op as xmm indices.
pub(super) fn three_fp(instruction: &CodeInstruction) -> Result<(u8, u8, u8), String> {
    Ok((
        fp_reg(field(instruction, "dst")?)?,
        fp_reg(field(instruction, "lhs")?)?,
        fp_reg(field(instruction, "rhs")?)?,
    ))
}

/// Packed 3-operand SSE op `dst = lhs OP rhs` (prefix 66). Handles the in-place
/// forms: `dst==lhs` → op in place; `dst==rhs` commutative → swap; `dst==rhs`
/// non-commutative → stage `rhs` in xmm15; else copy `lhs` first.
fn vec3(prefix: u8, opcode: u8, commutative: bool, dst: u8, lhs: u8, rhs: u8) -> Vec<u8> {
    if dst == lhs {
        enc_sse_rr(Some(prefix), opcode, dst, rhs)
    } else if dst == rhs && commutative {
        enc_sse_rr(Some(prefix), opcode, dst, lhs)
    } else if dst == rhs {
        let mut b = enc_movaps(15, rhs);
        b.extend(enc_movaps(dst, lhs));
        b.extend(enc_sse_rr(Some(prefix), opcode, dst, 15));
        b
    } else {
        let mut b = enc_movaps(dst, lhs);
        b.extend(enc_sse_rr(Some(prefix), opcode, dst, rhs));
        b
    }
}

pub(super) fn vec3_op(
    instruction: &CodeInstruction,
    opcode: u8,
    commutative: bool,
) -> Result<Encoded, String> {
    let (dst, lhs, rhs) = three_fp(instruction)?;
    Ok(Encoded::plain(vec3(
        0x66,
        opcode,
        commutative,
        dst,
        lhs,
        rhs,
    )))
}

/// Commutative three-byte SSE op (pcmpeqq) with in-place dst handling.
pub(super) fn vec_sse38_commutative(opcode: u8, dst: u8, lhs: u8, rhs: u8) -> Vec<u8> {
    if dst == lhs {
        enc_sse38_rr(opcode, dst, rhs)
    } else if dst == rhs {
        enc_sse38_rr(opcode, dst, lhs)
    } else {
        let mut b = enc_movaps(dst, lhs);
        b.extend(enc_sse38_rr(opcode, dst, rhs));
        b
    }
}

/// `dst = rhs CMP lhs` via cmppd — the swapped-operand form that realizes
/// `fcmgt`/`fcmge` (`lhs>rhs` = `rhs<lhs`, false on NaN) from LT/LE predicates.
pub(super) fn vec_cmppd_swapped(dst: u8, lhs: u8, rhs: u8, pred: u8) -> Vec<u8> {
    if dst == rhs {
        enc_cmppd(dst, lhs, pred)
    } else if dst == lhs {
        let mut b = enc_movaps(15, lhs);
        b.extend(enc_movaps(dst, rhs));
        b.extend(enc_cmppd(dst, 15, pred));
        b
    } else {
        let mut b = enc_movaps(dst, rhs);
        b.extend(enc_cmppd(dst, lhs, pred));
        b
    }
}

/// `dst = lhs OP rhs` for a scalar-double SSE arithmetic op (addsd/subsd/mulsd/
/// divsd), which is 2-operand (`dst OP= src`). Handles the `dst == rhs` aliasing:
/// commutative ops reorder; non-commutative ones stage in the reserved xmm15.
pub(super) fn sse_arith(
    instruction: &CodeInstruction,
    opcode: u8,
    commutative: bool,
) -> Result<Encoded, String> {
    let dst = fp_reg(field(instruction, "dst")?)?;
    let lhs = fp_reg(field(instruction, "lhs")?)?;
    let rhs = fp_reg(field(instruction, "rhs")?)?;
    if dst == lhs {
        return Ok(Encoded::plain(enc_sse_rr(Some(0xf2), opcode, dst, rhs)));
    }
    if dst == rhs {
        if commutative {
            return Ok(Encoded::plain(enc_sse_rr(Some(0xf2), opcode, dst, lhs)));
        }
        // sub/div with dst aliasing rhs: xmm15 = lhs; xmm15 OP= rhs; dst = xmm15.
        let mut b = enc_sse_rr(Some(0xf2), 0x10, 15, lhs);
        b.extend(enc_sse_rr(Some(0xf2), opcode, 15, rhs));
        b.extend(enc_sse_rr(Some(0xf2), 0x10, dst, 15));
        return Ok(Encoded::plain(b));
    }
    let mut b = enc_sse_rr(Some(0xf2), 0x10, dst, lhs);
    b.extend(enc_sse_rr(Some(0xf2), opcode, dst, rhs));
    Ok(Encoded::plain(b))
}
