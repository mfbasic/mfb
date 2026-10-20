//! plan-176-J: the x86-64 encoder's float_v128_and_ranges tests. The shared `fresh_encoder`/`bytes`/\n//! `enc_err`/`enc` helpers live in `encode/tests/mod.rs`.

use super::*;

#[test]
fn float_scalar_moves_and_arith() {
    // fmov_d_from_x xmm0, rax : movq xmm0, rax → 66 48 0F 6E C0
    assert_eq!(
        bytes("fmov_d_from_x", &[("dst", "xmm0"), ("src", "rax")]),
        [0x66, 0x48, 0x0F, 0x6E, 0xC0]
    );
    // fmov_x_from_d rax, xmm0 : movq rax, xmm0 → 66 48 0F 7E C0
    assert_eq!(
        bytes("fmov_x_from_d", &[("dst", "rax"), ("src", "xmm0")]),
        [0x66, 0x48, 0x0F, 0x7E, 0xC0]
    );
    // fmov_d_from_d xmm1, xmm2 : movaps → 0F 28 CA
    assert_eq!(
        bytes("fmov_d_from_d", &[("dst", "xmm1"), ("src", "xmm2")]),
        [0x0F, 0x28, 0xCA]
    );
    // addsd dst==lhs: fadd_d xmm0, xmm0, xmm1 → F2 0F 58 C1
    assert_eq!(
        bytes(
            "fadd_d",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0xF2, 0x0F, 0x58, 0xC1]
    );
    // fadd_d dst==rhs commutative: xmm0, xmm1, xmm0 → addsd xmm0, xmm1
    assert_eq!(
        bytes(
            "fadd_d",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0xF2, 0x0F, 0x58, 0xC1]
    );
    // fadd_d disjoint: xmm0, xmm1, xmm2 → movsd xmm0,xmm1 ; addsd xmm0,xmm2
    assert_eq!(
        bytes(
            "fadd_d",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0xF2, 0x0F, 0x10, 0xC1, 0xF2, 0x0F, 0x58, 0xC2]
    );
    // fsub_d dst==rhs non-commutative: stages through xmm15.
    // movsd xmm15,xmm1 ; subsd xmm15,xmm0 ; movsd xmm0,xmm15 (xmm1-xmm0).
    assert_eq!(
        bytes(
            "fsub_d",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [
            0xF2, 0x44, 0x0F, 0x10, 0xF9, 0xF2, 0x44, 0x0F, 0x5C, 0xF8, 0xF2, 0x41, 0x0F, 0x10,
            0xC7
        ]
    );
    // fsub_d dst==lhs in place: subsd xmm0,xmm1.
    assert_eq!(
        bytes(
            "fsub_d",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0xF2, 0x0F, 0x5C, 0xC1]
    );
    // fdiv_d disjoint: movsd xmm0,xmm1 ; divsd xmm0,xmm2.
    assert_eq!(
        bytes(
            "fdiv_d",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0xF2, 0x0F, 0x10, 0xC1, 0xF2, 0x0F, 0x5E, 0xC2]
    );
    // fmul_d dst==lhs in place: mulsd xmm0,xmm1.
    assert_eq!(
        bytes(
            "fmul_d",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0xF2, 0x0F, 0x59, 0xC1]
    );
}

#[test]
fn float_sqrt_compare_neg_abs() {
    // sqrtsd xmm0, xmm1 : F2 0F 51 C1
    assert_eq!(
        bytes("fsqrt_d", &[("dst", "xmm0"), ("src", "xmm1")]),
        [0xF2, 0x0F, 0x51, 0xC1]
    );
    // ucomisd xmm0, xmm1 : 66 0F 2E C1
    assert_eq!(
        bytes("fcmp_d", &[("lhs", "xmm0"), ("rhs", "xmm1")]),
        [0x66, 0x0F, 0x2E, 0xC1]
    );
    // fcmp_zero_d src=xmm0 : xorps xmm15,xmm15 ; ucomisd xmm0,xmm15.
    assert_eq!(
        bytes("fcmp_zero_d", &[("src", "xmm0")]),
        [0x45, 0x0F, 0x57, 0xFF, 0x66, 0x41, 0x0F, 0x2E, 0xC7]
    );
    // fneg_d in place: pcmpeqd xmm15 ; psllq xmm15,63 (sign mask) ; xorpd xmm0,xmm15.
    assert_eq!(
        bytes("fneg_d", &[("dst", "xmm0"), ("src", "xmm0")]),
        [
            0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7, 0x3F, 0x66, 0x41, 0x0F,
            0x57, 0xC7
        ]
    );
    // fneg_d copy-first: movsd xmm0,xmm1 then the sign-flip above.
    assert_eq!(
        bytes("fneg_d", &[("dst", "xmm0"), ("src", "xmm1")]),
        [
            0xF2, 0x0F, 0x10, 0xC1, 0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7,
            0x3F, 0x66, 0x41, 0x0F, 0x57, 0xC7
        ]
    );
    // fabs_d in place: psllq xmm0,1 ; psrlq xmm0,1 (clear the sign bit).
    assert_eq!(
        bytes("fabs_d", &[("dst", "xmm0"), ("src", "xmm0")]),
        [0x66, 0x0F, 0x73, 0xF0, 0x01, 0x66, 0x0F, 0x73, 0xD0, 0x01]
    );
    // fabs_d copy-first: movsd xmm0,xmm1 ; psllq xmm0,1 ; psrlq xmm0,1.
    assert_eq!(
        bytes("fabs_d", &[("dst", "xmm0"), ("src", "xmm1")]),
        [0xF2, 0x0F, 0x10, 0xC1, 0x66, 0x0F, 0x73, 0xF0, 0x01, 0x66, 0x0F, 0x73, 0xD0, 0x01]
    );
}

#[test]
fn float_int_conversions() {
    // cvtsi2sd xmm0, rax : F2 48 0F 2A C0
    assert_eq!(
        bytes("scvtf_d_from_x", &[("dst", "xmm0"), ("src", "rax")]),
        [0xF2, 0x48, 0x0F, 0x2A, 0xC0]
    );
    // cvttsd2si rax, xmm0 : F2 48 0F 2C C0
    assert_eq!(
        bytes("fcvtzs_x_from_d", &[("dst", "rax"), ("src", "xmm0")]),
        [0xF2, 0x48, 0x0F, 0x2C, 0xC0]
    );
    // Directed-rounding floor/ceil and ties-away drive the roundsd sequences.
    // fcvtms (floor): roundsd xmm15,xmm0,1 ; cvttsd2si rax,xmm15.
    assert_eq!(
        bytes("fcvtms_x_from_d", &[("dst", "rax"), ("src", "xmm0")]),
        [0x66, 0x44, 0x0F, 0x3A, 0x0B, 0xF8, 0x01, 0xF2, 0x49, 0x0F, 0x2C, 0xC7]
    );
    // fcvtps (ceil): roundsd xmm15,xmm0,2 ; cvttsd2si rax,xmm15.
    assert_eq!(
        bytes("fcvtps_x_from_d", &[("dst", "rax"), ("src", "xmm0")]),
        [0x66, 0x44, 0x0F, 0x3A, 0x0B, 0xF8, 0x02, 0xF2, 0x49, 0x0F, 0x2C, 0xC7]
    );
    // fcvtas (ties-away): push rcx ; cvttsd2si rax,xmm0 ; cvtsi2sd xmm15,rax ;
    // subsd xmm15,xmm0 ; addsd xmm15,xmm15 ; roundsd xmm15,xmm15,3 ;
    // cvttsd2si rcx,xmm15 ; sub rax,rcx ; pop rcx.
    assert_eq!(
        bytes("fcvtas_x_from_d", &[("dst", "rax"), ("src", "xmm0")]),
        [
            0x51, 0xF2, 0x48, 0x0F, 0x2C, 0xC0, 0xF2, 0x4C, 0x0F, 0x2A, 0xF8, 0xF2, 0x44, 0x0F,
            0x5C, 0xF8, 0xF2, 0x45, 0x0F, 0x58, 0xFF, 0x66, 0x45, 0x0F, 0x3A, 0x0B, 0xFF, 0x03,
            0xF2, 0x49, 0x0F, 0x2C, 0xCF, 0x48, 0x29, 0xC8, 0x59
        ]
    );
}

#[test]
fn f2i_nearest_never_clobbers_its_own_dst() {
    // bug-17: the ties-away sequence staged `bits(0.5)` with `movabs rax`, the one
    // GPR-shuttling float op that neither preserved nor avoided rax. With
    // `dst == rax` (a legal encoding: rax is the ABI return register) the movabs
    // destroyed the sign bit the previous two shifts had just computed, so every
    // negative input rounded toward zero instead of away.
    //
    // bug-295 replaced the whole copysign-materialization approach — the sequence
    // no longer builds `bits(0.5)` at all, because computing the fraction exactly
    // is what stops the double rounding. This test therefore asserts bug-17's
    // INVARIANT (the sequence must not corrupt its own `dst`) rather than the
    // mechanism that used to satisfy it: the previous version also required a
    // literal `0x3FE` immediate to be present, which pinned an implementation
    // detail rather than the property that matters.
    for dst in ["rax", "rcx", "rbx", "r10"] {
        let b = bytes("fcvtas_x_from_d", &[("dst", dst), ("src", "xmm0")]);
        // No `movabs r64, imm64` (REX.W + B8+rd) anywhere in the sequence — the
        // specific instruction bug-17 removed.
        assert!(
            !b.windows(2)
                .any(|w| w[0] & 0xF8 == 0x48 && w[1] & 0xF8 == 0xB8),
            "{dst}: movabs must stay gone: {b:02x?}"
        );
    }
    // The GPR commandeered for the correction term is pushed first and popped last,
    // and is never `dst` — so whichever register the caller asked the result in,
    // the sequence cannot destroy it. (0x50+rd = push, 0x58+rd = pop.)
    for (dst, dst_num) in [("rax", 0u8), ("rcx", 1), ("rbx", 3), ("r10", 10)] {
        let b = bytes("fcvtas_x_from_d", &[("dst", dst), ("src", "xmm0")]);
        let pushed = b[0] - 0x50;
        let popped = b[b.len() - 1] - 0x58;
        assert_eq!(
            pushed, popped,
            "{dst}: must restore the register it commandeered"
        );
        assert_ne!(
            pushed, dst_num,
            "{dst}: the commandeered scratch must not be dst itself: {b:02x?}"
        );
    }
}

#[test]
fn float_scalar_mem() {
    // movsd xmm0, [rbx+8] load; movsd [rbx+8], xmm0 store.
    assert_eq!(
        bytes(
            "ldr_d",
            &[("dst", "xmm0"), ("base", "rbx"), ("offset", "8")]
        ),
        [0xF2, 0x0F, 0x10, 0x83, 0x08, 0x00, 0x00, 0x00]
    );
    assert_eq!(
        bytes(
            "str_d",
            &[("src", "xmm0"), ("base", "rbx"), ("offset", "8")]
        ),
        [0xF2, 0x0F, 0x11, 0x83, 0x08, 0x00, 0x00, 0x00]
    );
    // Extended register + rsp base drives the REX + SIB paths:
    // movsd xmm8, [rsp] (REX.R + SIB, disp32=0).
    assert_eq!(
        bytes(
            "ldr_d",
            &[("dst", "xmm8"), ("base", "rsp"), ("offset", "0")]
        ),
        [0xF2, 0x44, 0x0F, 0x10, 0x84, 0x24, 0x00, 0x00, 0x00, 0x00]
    );
}

#[test]
fn float_mem_bad_offset_errors() {
    let ins = CodeInstruction::new("ldr_d")
        .physical_field("dst", "xmm0")
        .physical_field("base", "rbx")
        .field("offset", "nope");
    assert!(enc_err(&ins).contains("ldr_d"));
    let ins = CodeInstruction::new("str_d")
        .physical_field("src", "xmm0")
        .physical_field("base", "rbx")
        .field("offset", "nope");
    assert!(enc_err(&ins).contains("str_d"));
}

#[test]
fn v128_load_store_and_bad_offset() {
    // movups xmm0, [rbx+16] load; movups [rbx+16], xmm0 store.
    assert_eq!(
        bytes(
            "ldr_q",
            &[("dst", "xmm0"), ("base", "rbx"), ("offset", "16")]
        ),
        [0x0F, 0x10, 0x83, 0x10, 0x00, 0x00, 0x00]
    );
    assert_eq!(
        bytes(
            "str_q",
            &[("src", "xmm0"), ("base", "rbx"), ("offset", "16")]
        ),
        [0x0F, 0x11, 0x83, 0x10, 0x00, 0x00, 0x00]
    );
    // Extended register drives REX: movups xmm8, [r9] (REX.R.B, disp32=0).
    assert_eq!(
        bytes("ldr_q", &[("dst", "xmm8"), ("base", "r9"), ("offset", "0")]),
        [0x45, 0x0F, 0x10, 0x81, 0x00, 0x00, 0x00, 0x00]
    );
    let ins = CodeInstruction::new("ldr_q")
        .physical_field("dst", "xmm0")
        .physical_field("base", "rbx")
        .field("offset", "x");
    assert!(enc_err(&ins).contains("ldr_q"));
    let ins = CodeInstruction::new("str_q")
        .physical_field("src", "xmm0")
        .physical_field("base", "rbx")
        .field("offset", "x");
    assert!(enc_err(&ins).contains("str_q"));
}

#[test]
fn v128_three_operand_arith() {
    // Drive every vec3_op mnemonic through the disjoint arm: each is
    // `movaps xmm0,xmm1` (0F 28 C1) then the packed op `<pfx> 0F <opc> C2`.
    for (op, want) in [
        ("fadd_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x58, 0xC2]), // addpd
        ("fmul_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x59, 0xC2]), // mulpd
        ("fsub_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x5C, 0xC2]), // subpd
        ("fdiv_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x5E, 0xC2]), // divpd
        ("fmin_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x5D, 0xC2]), // minpd
        ("fmax_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x5F, 0xC2]), // maxpd
        ("add_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0xD4, 0xC2]),  // paddq
        ("sub_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0xFB, 0xC2]),  // psubq
        ("and_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0xDB, 0xC2]),  // pand
        ("orr_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0xEB, 0xC2]),  // por
        ("eor_v", [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0xEF, 0xC2]),  // pxor
    ] {
        assert_eq!(
            bytes(op, &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]),
            want,
            "vec3 disjoint {op}"
        );
    }
    // Commutative dst==rhs: fadd_v xmm0,xmm1,xmm0 → addpd xmm0,xmm1 in place.
    assert_eq!(
        bytes(
            "fadd_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0x66, 0x0F, 0x58, 0xC1]
    );
    // Non-commutative dst==rhs staged: movaps xmm15,xmm0 ; movaps xmm0,xmm1 ;
    // subpd xmm0,xmm15 (= xmm1 - xmm0).
    assert_eq!(
        bytes(
            "fsub_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0x44, 0x0F, 0x28, 0xF8, 0x0F, 0x28, 0xC1, 0x66, 0x41, 0x0F, 0x5C, 0xC7]
    );
    // dst==lhs in place: fadd_v xmm0,xmm0,xmm1 → addpd xmm0,xmm1.
    assert_eq!(
        bytes(
            "fadd_v",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0x66, 0x0F, 0x58, 0xC1]
    );
}

#[test]
fn v128_unary_and_neg_abs() {
    // fsqrt_v xmm0, xmm1 : sqrtpd xmm0,xmm1.
    assert_eq!(
        bytes("fsqrt_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [0x66, 0x0F, 0x51, 0xC1]
    );
    // fneg_v in place: pcmpeqd xmm15 ; psllq xmm15,63 ; xorpd xmm0,xmm15.
    assert_eq!(
        bytes("fneg_v", &[("dst", "xmm0"), ("src", "xmm0")]),
        [
            0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7, 0x3F, 0x66, 0x41, 0x0F,
            0x57, 0xC7
        ]
    );
    // fneg_v copy-first inserts movaps xmm0,xmm1 before the xorpd.
    assert_eq!(
        bytes("fneg_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [
            0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xF7, 0x3F, 0x0F, 0x28, 0xC1,
            0x66, 0x41, 0x0F, 0x57, 0xC7
        ]
    );
    // fabs_v in place: pcmpeqd xmm15 ; psrlq xmm15,1 ; andpd xmm0,xmm15.
    assert_eq!(
        bytes("fabs_v", &[("dst", "xmm0"), ("src", "xmm0")]),
        [
            0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xD7, 0x01, 0x66, 0x41, 0x0F,
            0x54, 0xC7
        ]
    );
    // fabs_v copy-first inserts movaps xmm0,xmm1 before the andpd.
    assert_eq!(
        bytes("fabs_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [
            0x66, 0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0x73, 0xD7, 0x01, 0x0F, 0x28, 0xC1,
            0x66, 0x41, 0x0F, 0x54, 0xC7
        ]
    );
    // neg_v xmm0, xmm1 : pxor xmm15,xmm15 ; psubq xmm15,xmm1 ; movaps xmm0,xmm15.
    assert_eq!(
        bytes("neg_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0xFB, 0xF9, 0x41, 0x0F, 0x28, 0xC7]
    );
    // abs_v aliasing: pxor xmm15 ; pcmpgtq xmm15,xmm0 (sign mask) ;
    // pxor xmm0,xmm15 ; psubq xmm0,xmm15 (two's-complement abs).
    assert_eq!(
        bytes("abs_v", &[("dst", "xmm0"), ("src", "xmm0")]),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF8, 0x66, 0x41, 0x0F,
            0xEF, 0xC7, 0x66, 0x41, 0x0F, 0xFB, 0xC7
        ]
    );
    // abs_v disjoint inserts movaps xmm0,xmm1 after the mask.
    assert_eq!(
        bytes("abs_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF9, 0x0F, 0x28, 0xC1,
            0x66, 0x41, 0x0F, 0xEF, 0xC7, 0x66, 0x41, 0x0F, 0xFB, 0xC7
        ]
    );
}

#[test]
fn v128_integer_compares() {
    // cmgt_v dst==rhs && dst!=lhs staged: movaps xmm15,xmm0 ; movaps xmm0,xmm1 ;
    // pcmpgtq xmm0,xmm15 (= xmm1 > xmm0).
    assert_eq!(
        bytes(
            "cmgt_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0x44, 0x0F, 0x28, 0xF8, 0x0F, 0x28, 0xC1, 0x66, 0x41, 0x0F, 0x38, 0x37, 0xC7]
    );
    // cmgt_v dst==lhs in place: pcmpgtq xmm0,xmm1.
    assert_eq!(
        bytes(
            "cmgt_v",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0x66, 0x0F, 0x38, 0x37, 0xC1]
    );
    // cmgt_v disjoint: movaps xmm0,xmm1 ; pcmpgtq xmm0,xmm2.
    assert_eq!(
        bytes(
            "cmgt_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x38, 0x37, 0xC2]
    );
    // cmeq_v commutative in-place (dst==lhs): pcmpeqq xmm0,xmm1.
    assert_eq!(
        bytes(
            "cmeq_v",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0x66, 0x0F, 0x38, 0x29, 0xC1]
    );
    // cmeq_v dst==rhs commutes to the same pcmpeqq xmm0,xmm1.
    assert_eq!(
        bytes(
            "cmeq_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0x66, 0x0F, 0x38, 0x29, 0xC1]
    );
    // cmeq_v disjoint: movaps xmm0,xmm1 ; pcmpeqq xmm0,xmm2.
    assert_eq!(
        bytes(
            "cmeq_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x38, 0x29, 0xC2]
    );
    // cmge_v dst==rhs: movaps xmm15,xmm1 ; pcmpgtq xmm0,xmm15 ; pcmpeqd xmm15 ;
    // pxor xmm0,xmm15 (= NOT(rhs>lhs) = lhs>=rhs).
    assert_eq!(
        bytes(
            "cmge_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [
            0x44, 0x0F, 0x28, 0xF9, 0x66, 0x41, 0x0F, 0x38, 0x37, 0xC7, 0x66, 0x45, 0x0F, 0x76,
            0xFF, 0x66, 0x41, 0x0F, 0xEF, 0xC7
        ]
    );
    // cmge_v disjoint inserts movaps xmm0,xmm2 before the pcmpgtq.
    assert_eq!(
        bytes(
            "cmge_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [
            0x44, 0x0F, 0x28, 0xF9, 0x0F, 0x28, 0xC2, 0x66, 0x41, 0x0F, 0x38, 0x37, 0xC7, 0x66,
            0x45, 0x0F, 0x76, 0xFF, 0x66, 0x41, 0x0F, 0xEF, 0xC7
        ]
    );
}

#[test]
fn v128_float_compares() {
    // fcmgt_v dst==rhs: fcmgt(lhs,rhs)=rhs<lhs → cmpltpd xmm0,xmm1 (pred 1).
    assert_eq!(
        bytes(
            "fcmgt_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0x66, 0x0F, 0xC2, 0xC1, 0x01]
    );
    // fcmgt_v dst==lhs staged: movaps xmm15,xmm0 ; movaps xmm0,xmm1 ;
    // cmpltpd xmm0,xmm15.
    assert_eq!(
        bytes(
            "fcmgt_v",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0x44, 0x0F, 0x28, 0xF8, 0x0F, 0x28, 0xC1, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x01]
    );
    // fcmge_v disjoint: movaps xmm0,xmm2 ; cmplepd xmm0,xmm1 (pred 2).
    assert_eq!(
        bytes(
            "fcmge_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0x0F, 0x28, 0xC2, 0x66, 0x0F, 0xC2, 0xC1, 0x02]
    );
    // fcmeq_v dst==rhs staged: movaps xmm15,xmm0 ; movaps xmm0,xmm1 ;
    // cmpeqpd xmm0,xmm15 (pred 0).
    assert_eq!(
        bytes(
            "fcmeq_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
        ),
        [0x44, 0x0F, 0x28, 0xF8, 0x0F, 0x28, 0xC1, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x00]
    );
    // fcmeq_v dst==lhs in place (commutative): cmpeqpd xmm0,xmm1.
    assert_eq!(
        bytes(
            "fcmeq_v",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0x66, 0x0F, 0xC2, 0xC1, 0x00]
    );
    // fcmeq_v disjoint: movaps xmm0,xmm1 ; cmpeqpd xmm0,xmm2.
    assert_eq!(
        bytes(
            "fcmeq_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0xC2, 0xC2, 0x00]
    );
}

#[test]
fn v128_zero_compares() {
    // Each op zeroes xmm15 (pxor) and compares against it. gt/ge move the zero
    // into dst then cmplt/cmple dst,src; lt/le/eq compare dst(=src) against
    // xmm15. `same` is dst==src==xmm0, `diff` copies src=xmm1 in first.
    for (op, same, diff) in [
        (
            "fcmgt_zero_v", // pxor xmm15 ; movaps xmm0,xmm15 ; cmpltpd xmm0,{xmm0|xmm1}
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xC7, 0x66, 0x0F, 0xC2, 0xC0, 0x01,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xC7, 0x66, 0x0F, 0xC2, 0xC1, 0x01,
            ][..],
        ),
        (
            "fcmge_zero_v", // ... cmplepd (pred 2)
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xC7, 0x66, 0x0F, 0xC2, 0xC0, 0x02,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x41, 0x0F, 0x28, 0xC7, 0x66, 0x0F, 0xC2, 0xC1, 0x02,
            ][..],
        ),
        (
            "fcmlt_zero_v", // pxor xmm15 ; [movaps xmm0,xmm1;] cmpltpd xmm0,xmm15
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x01,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x0F, 0x28, 0xC1, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x01,
            ][..],
        ),
        (
            "fcmle_zero_v", // ... cmplepd xmm0,xmm15 (pred 2)
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x02,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x0F, 0x28, 0xC1, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x02,
            ][..],
        ),
        (
            "fcmeq_zero_v", // ... cmpeqpd xmm0,xmm15 (pred 0)
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x00,
            ][..],
            &[
                0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x0F, 0x28, 0xC1, 0x66, 0x41, 0x0F, 0xC2, 0xC7, 0x00,
            ][..],
        ),
    ] {
        assert_eq!(
            bytes(op, &[("dst", "xmm0"), ("src", "xmm0")]),
            same,
            "{op} same"
        );
        assert_eq!(
            bytes(op, &[("dst", "xmm0"), ("src", "xmm1")]),
            diff,
            "{op} diff"
        );
    }
}

#[test]
fn v128_rounding() {
    // roundpd xmm0,xmm1,mode — ceil(2)/floor(1)/trunc(3)/nearest(0).
    for (op, want) in [
        ("frintp_v", [0x66, 0x0F, 0x3A, 0x09, 0xC1, 0x02]), // ceil
        ("frintm_v", [0x66, 0x0F, 0x3A, 0x09, 0xC1, 0x01]), // floor
        ("frintz_v", [0x66, 0x0F, 0x3A, 0x09, 0xC1, 0x03]), // trunc
        ("frintn_v", [0x66, 0x0F, 0x3A, 0x09, 0xC1, 0x00]), // nearest-even
    ] {
        assert_eq!(bytes(op, &[("dst", "xmm0"), ("src", "xmm1")]), want, "{op}");
    }
    // frinta_v (ties-away): roundpd xmm0,xmm1,3 ; movapd xmm15,xmm0 ;
    // subpd xmm15,xmm1 ; addpd xmm15,xmm15 ; roundpd xmm15,xmm15,3 ;
    // subpd xmm0,xmm15.
    assert_eq!(
        bytes("frinta_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [
            0x66, 0x0F, 0x3A, 0x09, 0xC1, 0x03, 0x66, 0x44, 0x0F, 0x28, 0xF8, 0x66, 0x44, 0x0F,
            0x5C, 0xF9, 0x66, 0x45, 0x0F, 0x58, 0xFF, 0x66, 0x45, 0x0F, 0x3A, 0x09, 0xFF, 0x03,
            0x66, 0x41, 0x0F, 0x5C, 0xC7
        ]
    );
}

#[test]
fn v128_shifts_dup_extract() {
    // shl_v in place: psllq xmm0,3.
    assert_eq!(
        bytes("shl_v", &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "3")]),
        [0x66, 0x0F, 0x73, 0xF0, 0x03]
    );
    // shl_v copy-first: movaps xmm0,xmm1 ; psllq xmm0,3.
    assert_eq!(
        bytes("shl_v", &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "3")]),
        [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x73, 0xF0, 0x03]
    );
    // ushr_v copy-first: movaps xmm0,xmm1 ; psrlq xmm0,3.
    assert_eq!(
        bytes(
            "ushr_v",
            &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "3")]
        ),
        [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x73, 0xD0, 0x03]
    );
    // Bad shift errors.
    let ins = CodeInstruction::new("shl_v")
        .physical_field("dst", "xmm0")
        .physical_field("src", "xmm0")
        .field("shift", "z");
    assert!(encode_instruction(&ins).is_err());
    // dup_v_from_x xmm0, rax : movq xmm0,rax ; punpcklqdq xmm0,xmm0 (broadcast).
    assert_eq!(
        bytes("dup_v_from_x", &[("dst", "xmm0"), ("src", "rax")]),
        [0x66, 0x48, 0x0F, 0x6E, 0xC0, 0x66, 0x0F, 0x6C, 0xC0]
    );
    // umov_x_from_v lane 0 (movq rax,xmm0).
    assert_eq!(
        bytes(
            "umov_x_from_v",
            &[("dst", "rax"), ("src", "xmm0"), ("index", "0")]
        ),
        [0x66, 0x48, 0x0F, 0x7E, 0xC0]
    );
    // umov_x_from_v lane 1 (pextrq rax,xmm0,1).
    assert_eq!(
        bytes(
            "umov_x_from_v",
            &[("dst", "rax"), ("src", "xmm0"), ("index", "1")]
        ),
        [0x66, 0x48, 0x0F, 0x3A, 0x16, 0xC0, 0x01]
    );
    let ins = CodeInstruction::new("umov_x_from_v")
        .physical_field("dst", "rax")
        .physical_field("src", "xmm0")
        .field("index", "z");
    assert!(encode_instruction(&ins).is_err());
}

#[test]
fn v128_bit_select_fma_convert() {
    // bsl_v: movaps xmm15,xmm0 ; pand xmm0,xmm1 ; pandn xmm15,xmm2 ; por xmm0,xmm15.
    assert_eq!(
        bytes(
            "bsl_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [
            0x44, 0x0F, 0x28, 0xF8, 0x66, 0x0F, 0xDB, 0xC1, 0x66, 0x44, 0x0F, 0xDF, 0xFA, 0x66,
            0x41, 0x0F, 0xEB, 0xC7
        ]
    );
    // bit_v: movaps xmm15,xmm0 ; pxor xmm15,xmm1 ; pand xmm15,xmm2 ; pxor xmm0,xmm15.
    assert_eq!(
        bytes(
            "bit_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [
            0x44, 0x0F, 0x28, 0xF8, 0x66, 0x44, 0x0F, 0xEF, 0xF9, 0x66, 0x44, 0x0F, 0xDB, 0xFA,
            0x66, 0x41, 0x0F, 0xEF, 0xC7
        ]
    );
    // fmla_v xmm0,xmm1,xmm2 : vfmadd231pd xmm0,xmm1,xmm2.
    assert_eq!(
        bytes(
            "fmla_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0xC4, 0xE2, 0xF1, 0xB8, 0xC2]
    );
    // fmls_v xmm0,xmm1,xmm2 : vfnmadd231pd xmm0,xmm1,xmm2.
    assert_eq!(
        bytes(
            "fmls_v",
            &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
        ),
        [0xC4, 0xE2, 0xF1, 0xBC, 0xC2]
    );
    // Extended-register FMA drives the VEX ~R/~B bits: vfmadd231pd xmm8,xmm1,xmm9.
    assert_eq!(
        bytes(
            "fmla_v",
            &[("dst", "xmm8"), ("lhs", "xmm1"), ("rhs", "xmm9")]
        ),
        [0xC4, 0x42, 0xF1, 0xB8, 0xC1]
    );
    // Lane-serial fcvtzs_v: push rax/rdx ; cvttsd2si both lanes (lane 1 via
    // pshufd) ; reassemble via movq/punpcklqdq ; pop rdx/rax.
    assert_eq!(
        bytes("fcvtzs_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [
            0x50, 0x52, 0xF2, 0x48, 0x0F, 0x2C, 0xC1, 0x66, 0x44, 0x0F, 0x70, 0xF9, 0xEE, 0xF2,
            0x49, 0x0F, 0x2C, 0xD7, 0x66, 0x48, 0x0F, 0x6E, 0xC0, 0x66, 0x4C, 0x0F, 0x6E, 0xFA,
            0x66, 0x41, 0x0F, 0x6C, 0xC7, 0x5A, 0x58
        ]
    );
    // Lane-serial scvtf_v: symmetric, movq out then cvtsi2sd both lanes,
    // reassemble via unpcklpd.
    assert_eq!(
        bytes("scvtf_v", &[("dst", "xmm0"), ("src", "xmm1")]),
        [
            0x50, 0x52, 0x66, 0x48, 0x0F, 0x7E, 0xC8, 0x66, 0x44, 0x0F, 0x70, 0xF9, 0xEE, 0x66,
            0x4C, 0x0F, 0x7E, 0xFA, 0xF2, 0x48, 0x0F, 0x2A, 0xC0, 0xF2, 0x4C, 0x0F, 0x2A, 0xFA,
            0x66, 0x41, 0x0F, 0x14, 0xC7, 0x5A, 0x58
        ]
    );
    // sshr_v k=20 copy-first: build sign mask (pxor/pcmpgtq/psllq 44) ;
    // movaps xmm0,xmm1 ; psrlq xmm0,20 ; por xmm0,mask.
    assert_eq!(
        bytes(
            "sshr_v",
            &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "20")]
        ),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF9, 0x66, 0x41, 0x0F,
            0x73, 0xF7, 0x2C, 0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x73, 0xD0, 0x14, 0x66, 0x41, 0x0F,
            0xEB, 0xC7
        ]
    );
    // sshr_v k=0 copy-first: the mask is re-zeroed (pxor twice) so por is a no-op.
    assert_eq!(
        bytes(
            "sshr_v",
            &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "0")]
        ),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF9, 0x66, 0x45, 0x0F,
            0xEF, 0xFF, 0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x73, 0xD0, 0x00, 0x66, 0x41, 0x0F, 0xEB,
            0xC7
        ]
    );
    // sshr_v k=20 in place (dst==src) skips the movaps copy.
    assert_eq!(
        bytes(
            "sshr_v",
            &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "20")]
        ),
        [
            0x66, 0x45, 0x0F, 0xEF, 0xFF, 0x66, 0x44, 0x0F, 0x38, 0x37, 0xF8, 0x66, 0x41, 0x0F,
            0x73, 0xF7, 0x2C, 0x66, 0x0F, 0x73, 0xD0, 0x14, 0x66, 0x41, 0x0F, 0xEB, 0xC7
        ]
    );
    let ins = CodeInstruction::new("sshr_v")
        .physical_field("dst", "xmm0")
        .physical_field("src", "xmm1")
        .field("shift", "z");
    assert!(encode_instruction(&ins).is_err());

    // bug-16: `sshr .2d, #64` sign-fills the lane on AArch64. The emulation must
    // keep the `pcmpgtq` sign mask (unshifted) and let `psrlq dst, 64` zero the
    // lane, so the `por` leaves the mask. Clearing the mask returned 0 instead of
    // -1 for a negative lane. `pxor` (0F EF) therefore appears exactly ONCE for
    // k=64, and twice for the k=0 no-op that must clear the mask.
    let pxors = |b: &[u8]| b.windows(2).filter(|w| w == &[0x0F, 0xEF]).count();
    let k64 = bytes(
        "sshr_v",
        &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "64")],
    );
    assert_eq!(pxors(&k64), 1, "k=64 keeps the sign mask: {k64:02x?}");
    let k0 = bytes(
        "sshr_v",
        &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "0")],
    );
    assert_eq!(pxors(&k0), 2, "k=0 clears the sign mask");
    // The lane is shifted out by a `psrlq` (0F 73 /2, modrm reg field = 2) whose
    // immediate is 64 — an x86 count > 63 zeroes the lane by definition.
    assert!(
        k64.windows(4)
            .any(|w| w[0] == 0x0F && w[1] == 0x73 && (w[2] >> 3) & 0x7 == 0x02 && w[3] == 64),
        "psrlq dst,64 present: {k64:02x?}"
    );
    // ushr/shl by 64 zero the lane on both ISAs (an x86 count > 63 is defined to).
    // movaps xmm0,xmm1 ; psrlq xmm0,64.
    assert_eq!(
        bytes(
            "ushr_v",
            &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "64")]
        ),
        [0x0F, 0x28, 0xC1, 0x66, 0x0F, 0x73, 0xD0, 0x40]
    );
    // Past 64 the immediate is malformed and must be rejected, not truncated.
    for op in ["sshr_v", "ushr_v", "shl_v"] {
        let ins = CodeInstruction::new(op)
            .physical_field("dst", "xmm0")
            .physical_field("src", "xmm1")
            .field("shift", "65");
        assert!(
            encode_instruction(&ins).is_err(),
            "{op} must reject a shift of 65"
        );
    }
}

#[test]
fn div_aliasing_and_preservation() {
    // udiv whose divisor aliases rdx stages it in a stack slot:
    // sub rsp,8 ; mov [rsp],rdx ; mov rax,rsi ; xor rdx,rdx ; div [rsp] ;
    // mov rbx,rax ; add rsp,8.
    assert_eq!(
        bytes("udiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdx")]),
        [
            0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x14, 0x24, 0x48, 0x89, 0xF0,
            0x48, 0x31, 0xD2, 0x48, 0xF7, 0x34, 0x24, 0x48, 0x89, 0xC3, 0x48, 0x81, 0xC4, 0x08,
            0x00, 0x00, 0x00
        ]
    );
    // sdiv with rax dividend and non-rax dst preserves rax across the divide:
    // sub rsp,8 ; mov [rsp],rax ; mov rax,rax ; cqo ; idiv rsi ; mov rbx,rax ;
    // mov rax,[rsp] ; add rsp,8.
    assert_eq!(
        bytes("sdiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rsi")]),
        [
            0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x89, 0xC0,
            0x48, 0x99, 0x48, 0xF7, 0xFE, 0x48, 0x89, 0xC3, 0x48, 0x8B, 0x04, 0x24, 0x48, 0x81,
            0xC4, 0x08, 0x00, 0x00, 0x00
        ]
    );
    // Both aliasing + preservation at once (lhs==rhs==rax): two stack slots.
    assert_eq!(
        bytes("udiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rax")]),
        [
            0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x81, 0xEC,
            0x08, 0x00, 0x00, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x89, 0xC0, 0x48, 0x31, 0xD2,
            0x48, 0xF7, 0x34, 0x24, 0x48, 0x89, 0xC3, 0x48, 0x81, 0xC4, 0x08, 0x00, 0x00, 0x00,
            0x48, 0x8B, 0x04, 0x24, 0x48, 0x81, 0xC4, 0x08, 0x00, 0x00, 0x00
        ]
    );
}

#[test]
fn msub_extended_registers() {
    // Cover the extended-register REX arms of enc_mov/enc_imul in msub:
    // mov rax,r9 ; imul rax,r10 ; mov r8,r11 ; sub r8,rax.
    assert_eq!(
        bytes(
            "msub",
            &[
                ("dst", "r8"),
                ("lhs", "r9"),
                ("rhs", "r10"),
                ("minuend", "r11"),
            ]
        ),
        [0x4C, 0x89, 0xC8, 0x49, 0x0F, 0xAF, 0xC2, 0x4D, 0x89, 0xD8, 0x49, 0x29, 0xC0]
    );
}

#[test]
fn add_carry_extended_and_sub_borrow() {
    // add_carry with an extended carry_out drives the REX byte in enc_setcc_to:
    // add rbx,rdi ; setb r8b ; movzx r8,r8b.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "r8"),
                ("lhs", "rbx"),
                ("rhs", "rdi"),
                ("carry_in", "xzr"),
            ]
        ),
        [0x48, 0x01, 0xFB, 0x41, 0x0F, 0x92, 0xC0, 0x4D, 0x0F, 0xB6, 0xC0]
    );
    // sub_borrow with a borrow-in register drives the sbb path:
    // bt r10,0 ; sbb rbx,rdi ; setb sil ; movzx rsi,sil.
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "rsi"),
                ("lhs", "rbx"),
                ("rhs", "rdi"),
                ("borrow_in", "r10"),
            ]
        ),
        [
            0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x19, 0xFB, 0x40, 0x0F, 0x92, 0xC6, 0x48, 0x0F,
            0xB6, 0xF6
        ]
    );
    // add_carry where dst != lhs drives the mov-in arm:
    // mov rbx,rcx ; add rbx,rdi ; setb sil ; movzx rsi,sil.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "rsi"),
                ("lhs", "rcx"),
                ("rhs", "rdi"),
                ("carry_in", "xzr"),
            ]
        ),
        [0x48, 0x89, 0xCB, 0x48, 0x01, 0xFB, 0x40, 0x0F, 0x92, 0xC6, 0x48, 0x0F, 0xB6, 0xF6]
    );
    // sub_borrow dst != lhs, no borrow-in: mov rbx,rcx ; sub rbx,rdi ; setb sil ;
    // movzx rsi,sil.
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "rsi"),
                ("lhs", "rcx"),
                ("rhs", "rdi"),
                ("borrow_in", "xzr"),
            ]
        ),
        [0x48, 0x89, 0xCB, 0x48, 0x29, 0xFB, 0x40, 0x0F, 0x92, 0xC6, 0x48, 0x0F, 0xB6, 0xF6]
    );
}

#[test]
fn x86_float_conditional_branches() {
    for op in [
        "x86.jae", "x86.jp", "x86.jnp", "x86.ja", "x86.jb", "x86.jbe", "x86.je", "x86.jne",
    ] {
        let br = bytes(op, &[("target", "L")]);
        assert_eq!(br.len(), 6, "{op} should be a 6-byte jcc rel32");
        assert_eq!(br[0], 0x0F);
    }
    // b.vc / b.vs (jno / jo) and b.ls (jbe) integer-side branches.
    assert_eq!(bytes("b.vc", &[("target", "L")]), [0x0F, 0x81, 0, 0, 0, 0]);
}

#[test]
fn immediate_true_false_and_shift_range() {
    // mov_imm accepts the boolean tokens.
    assert_eq!(
        bytes(
            "mov_imm",
            &[("dst", "rax"), ("type", "Bool"), ("value", "true")]
        ),
        [0x48, 0xB8, 1, 0, 0, 0, 0, 0, 0, 0]
    );
    // A shift of 64 is out of range.
    let ins = CodeInstruction::new("lsl_imm")
        .physical_field("dst", "rax")
        .physical_field("src", "rax")
        .field("shift", "64");
    assert!(encode_instruction(&ins).is_err());
    // An invalid immediate is rejected.
    let ins = CodeInstruction::new("mov_imm")
        .physical_field("dst", "rax")
        .field("type", "Integer")
        .field("value", "notanumber");
    assert!(encode_instruction(&ins).is_err());
}

#[test]
fn imm32_overflow_and_disp32_overflow() {
    // add_imm with u64::MAX, which sign-extends to -1 → fits imm32 as a mask:
    // add rax, -1 (48 81 C0 FF FF FF FF).
    let mask = u64::MAX.to_string();
    assert_eq!(
        bytes("add_imm", &[("dst", "rax"), ("src", "rax"), ("imm", &mask)]),
        [0x48, 0x81, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    // A truly out-of-range immediate errors.
    let huge = (u64::MAX / 2).to_string();
    let ins = CodeInstruction::new("add_imm")
        .physical_field("dst", "rax")
        .physical_field("src", "rax")
        .field("imm", &huge);
    assert!(enc_err(&ins).contains("imm32"));
    // A memory offset exceeding disp32 errors.
    let ins = CodeInstruction::new("ldr_u64")
        .physical_field("dst", "rax")
        .physical_field("base", "rbx")
        .field("offset", &(u64::from(u32::MAX) + 1).to_string());
    assert!(enc_err(&ins).contains("disp32"));
}

#[test]
fn blr_and_unknown_register() {
    // blr through the low-register arm (no REX).
    assert_eq!(bytes("blr", &[("register", "rcx")]), [0xFF, 0xD1]);
    // An unknown register name errors.
    let ins = CodeInstruction::new("mov")
        .physical_field("dst", "notareg")
        .physical_field("src", "rax");
    assert!(enc_err(&ins).contains("unknown"));
    // An unknown xmm name errors.
    let ins = CodeInstruction::new("fmov_d_from_d")
        .physical_field("dst", "xmm99")
        .physical_field("src", "xmm0");
    assert!(enc_err(&ins).contains("xmm"));
}
