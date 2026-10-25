//! plan-176-J: the x86-64 encoder's integer tests. The shared `fresh_encoder`/`bytes`/\n//! `enc_err`/`enc` helpers live in `encode/tests/mod.rs`.

use super::*;

#[test]
fn mov_reg_reg() {
    // mov rax, rbx : REX.W 89 /r, rm=rax(0) reg=rbx(3) → 48 89 D8
    assert_eq!(
        bytes("mov", &[("dst", "rax"), ("src", "rbx")]),
        [0x48, 0x89, 0xD8]
    );
    // mov r8, r15 : REX.W.R.B 89 /r → 4D 89 F8
    assert_eq!(
        bytes("mov", &[("dst", "r8"), ("src", "r15")]),
        [0x4D, 0x89, 0xF8]
    );
}

#[test]
fn mov_imm64() {
    // mov rax, 1 : 48 B8 + 8-byte imm
    assert_eq!(
        bytes(
            "mov_imm",
            &[("dst", "rax"), ("type", "Integer"), ("value", "1")]
        ),
        [0x48, 0xB8, 0x01, 0, 0, 0, 0, 0, 0, 0]
    );
    // mov r15, 0 : REX.W.B 49 BF + 8 zero bytes
    assert_eq!(
        bytes(
            "mov_imm",
            &[("dst", "r15"), ("type", "Integer"), ("value", "0")]
        ),
        [0x49, 0xBF, 0, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn add_sub_and_or_eor() {
    // add rax, rax, rcx : dst==lhs → add rax,rcx = 48 01 C8
    assert_eq!(
        bytes("add", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rcx")]),
        [0x48, 0x01, 0xC8]
    );
    // sub rdx, rdx, rbx : 48 29 DA
    assert_eq!(
        bytes("sub", &[("dst", "rdx"), ("lhs", "rdx"), ("rhs", "rbx")]),
        [0x48, 0x29, 0xDA]
    );
    // and rsi, rsi, rdi : 48 21 FE
    assert_eq!(
        bytes("and", &[("dst", "rsi"), ("lhs", "rsi"), ("rhs", "rdi")]),
        [0x48, 0x21, 0xFE]
    );
    // orr rax, rax, rbx : 48 09 D8
    assert_eq!(
        bytes("orr", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x09, 0xD8]
    );
    // eor rax, rax, rax : 48 31 C0
    assert_eq!(
        bytes("eor", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rax")]),
        [0x48, 0x31, 0xC0]
    );
}

#[test]
fn add_with_move() {
    // add rdx, rax, rcx : dst != lhs → mov rdx,rax ; add rdx,rcx
    // mov rdx,rax = 48 89 C2 ; add rdx,rcx = 48 01 CA
    assert_eq!(
        bytes("add", &[("dst", "rdx"), ("lhs", "rax"), ("rhs", "rcx")]),
        [0x48, 0x89, 0xC2, 0x48, 0x01, 0xCA]
    );
}

#[test]
fn mvn_from_zero_token_materializes_minus_one() {
    // `mvn dst, xzr` is the portable "-1" idiom (net/io poll timeouts, ALSA
    // "infinite", process::waitFor's WIFSIGNALED result). x86 has no zero
    // register, and `reg` maps the token to sentinel 16, which enc_mov would
    // encode as `r8` — yielding `~r8` (whatever the caller last left there)
    // instead of -1. That is why `process::waitFor` returned -3 on a CI runner
    // where r8 held 2, while a box whose r8 held 0 saw the correct -1: the
    // runtime symptom depends on leftover register state, so pin the ENCODING.
    //
    // xor rax,rax (48 31 C0) ; not rax (48 F7 D0).
    assert_eq!(
        bytes("mvn", &[("dst", "rax"), ("src", "xzr")]),
        [0x48, 0x31, 0xC0, 0x48, 0xF7, 0xD0]
    );
    // r8 needs REX.R+REX.B on the xor (4D 31 C0) and REX.B on the not (49 F7 D0).
    assert_eq!(
        bytes("mvn", &[("dst", "r8"), ("src", "xzr")]),
        [0x4D, 0x31, 0xC0, 0x49, 0xF7, 0xD0]
    );
    // A real source register is unaffected: mov rax,rcx (48 89 C8) ; not rax.
    assert_eq!(
        bytes("mvn", &[("dst", "rax"), ("src", "rcx")]),
        [0x48, 0x89, 0xC8, 0x48, 0xF7, 0xD0]
    );
}

#[test]
fn mov_from_zero_token_materializes_zero() {
    // `mov dst, xzr` means "materialize 0" — reached by the Win64 audio backend's
    // NULL WASAPI arguments. Encoding `r8` there would pass garbage, not NULL.
    // xor rcx,rcx (48 31 C9).
    assert_eq!(
        bytes("mov", &[("dst", "rcx"), ("src", "xzr")]),
        [0x48, 0x31, 0xC9]
    );
    // A real source register still emits a plain move: mov rax,rcx (48 89 C8).
    assert_eq!(
        bytes("mov", &[("dst", "rax"), ("src", "rcx")]),
        [0x48, 0x89, 0xC8]
    );
}

#[test]
fn sub_with_zero_lhs_negates() {
    // sub rax, xzr, rax : dst==rhs → neg rax (48 F7 D8), NOT `sub rax,rax`=0.
    assert_eq!(
        bytes("sub", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rax")]),
        [0x48, 0xF7, 0xD8]
    );
    // sub rdx, xzr, rax : dst!=rhs → mov rdx,rax (48 89 C2) ; neg rdx (48 F7 DA).
    assert_eq!(
        bytes("sub", &[("dst", "rdx"), ("lhs", "xzr"), ("rhs", "rax")]),
        [0x48, 0x89, 0xC2, 0x48, 0xF7, 0xDA]
    );
    // sub r8, xzr, r8 : neg r8 needs REX.B (49 F7 D8).
    assert_eq!(
        bytes("sub", &[("dst", "r8"), ("lhs", "xzr"), ("rhs", "r8")]),
        [0x49, 0xF7, 0xD8]
    );
    // add rax, xzr, rcx : 0 + rcx = rcx → mov rax,rcx (48 89 C8).
    assert_eq!(
        bytes("add", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rcx")]),
        [0x48, 0x89, 0xC8]
    );
}

#[test]
fn alu3_dst_equals_rhs_aliasing() {
    // add rax, rcx, rax : dst==rhs → imul-free commute, add rax,rcx (48 01 C8),
    // NOT `mov rax,rcx; add rax,rax`.
    assert_eq!(
        bytes("add", &[("dst", "rax"), ("lhs", "rcx"), ("rhs", "rax")]),
        [0x48, 0x01, 0xC8]
    );
    // sub rax, rcx, rax : dst==rhs, non-commutative → neg rax (48 F7 D8) ;
    // add rax,rcx (48 01 C8) = rcx - rax.
    assert_eq!(
        bytes("sub", &[("dst", "rax"), ("lhs", "rcx"), ("rhs", "rax")]),
        [0x48, 0xF7, 0xD8, 0x48, 0x01, 0xC8]
    );
}

#[test]
fn mul_aliasing() {
    // mul rax, rax, rcx : dst==lhs → imul rax,rcx (48 0F AF C1).
    assert_eq!(
        bytes("mul", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rcx")]),
        [0x48, 0x0F, 0xAF, 0xC1]
    );
    // mul rax, rcx, rax : dst==rhs → imul rax,rcx (commutative), NOT rcx*rcx.
    assert_eq!(
        bytes("mul", &[("dst", "rax"), ("lhs", "rcx"), ("rhs", "rax")]),
        [0x48, 0x0F, 0xAF, 0xC1]
    );
    // mul rdx, rax, rcx : disjoint → mov rdx,rax (48 89 C2) ; imul rdx,rcx (48 0F AF D1).
    assert_eq!(
        bytes("mul", &[("dst", "rdx"), ("lhs", "rax"), ("rhs", "rcx")]),
        [0x48, 0x89, 0xC2, 0x48, 0x0F, 0xAF, 0xD1]
    );
}

#[test]
fn mvn() {
    // mvn rax, rbx : mov rax,rbx (48 89 D8) ; not rax (48 F7 D0)
    assert_eq!(
        bytes("mvn", &[("dst", "rax"), ("src", "rbx")]),
        [0x48, 0x89, 0xD8, 0x48, 0xF7, 0xD0]
    );
}

#[test]
fn mul_low() {
    // mul rax, rax, rbx : dst==lhs → imul rax,rbx in place (48 0F AF C3), no
    // redundant `mov rax,rax`.
    assert_eq!(
        bytes("mul", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x0F, 0xAF, 0xC3]
    );
}

#[test]
fn umulh() {
    // umulh rbx, rsi, rdi : mov rax,rsi (48 89 F0) ; mul rdi (48 F7 E7) ;
    // mov rbx,rdx (48 89 D3)
    assert_eq!(
        bytes("umulh", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdi")]),
        // push rax ; push rdx ; mov rax,rsi ; mul rdi ; mov rbx,rdx ; pop rdx ;
        // pop rax — rax/rdx are saved so an ABI-staged/residual operand there
        // survives (bug-125).
        [0x50, 0x52, 0x48, 0x89, 0xF0, 0x48, 0xF7, 0xE7, 0x48, 0x89, 0xD3, 0x5A, 0x58]
    );
}

#[test]
fn udiv_sdiv() {
    // udiv rbx, rsi, rdi : mov rax,rsi (48 89 F0) ; xor rdx,rdx (48 31 D2) ;
    // div rdi (48 F7 F7) ; mov rbx,rax (48 89 C3)
    assert_eq!(
        bytes("udiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdi")]),
        [0x48, 0x89, 0xF0, 0x48, 0x31, 0xD2, 0x48, 0xF7, 0xF7, 0x48, 0x89, 0xC3]
    );
    // sdiv rbx, rsi, rdi : mov rax,rsi ; cqo (48 99) ; idiv rdi (48 F7 FF) ;
    // mov rbx,rax
    assert_eq!(
        bytes("sdiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdi")]),
        [0x48, 0x89, 0xF0, 0x48, 0x99, 0x48, 0xF7, 0xFF, 0x48, 0x89, 0xC3]
    );
}

#[test]
fn msub() {
    // dst = minuend - lhs*rhs. When the minuend is rax (which the product goes
    // through), it is captured into dst FIRST, before rax is clobbered:
    // mov rbx,rax (48 89 C3) ; mov rax,rsi (48 89 F0) ; imul rax,rdi (48 0F AF C7) ;
    // sub rbx,rax (48 29 C3)
    assert_eq!(
        bytes(
            "msub",
            &[
                ("dst", "rbx"),
                ("lhs", "rsi"),
                ("rhs", "rdi"),
                ("minuend", "rax")
            ]
        ),
        [0x48, 0x89, 0xC3, 0x48, 0x89, 0xF0, 0x48, 0x0F, 0xAF, 0xC7, 0x48, 0x29, 0xC3]
    );
    // Non-rax minuend keeps the product-first order:
    // mov rax,rsi (48 89 F0) ; imul rax,rdi (48 0F AF C7) ; mov rbx,rcx (48 89 CB) ;
    // sub rbx,rax (48 29 C3)
    assert_eq!(
        bytes(
            "msub",
            &[
                ("dst", "rbx"),
                ("lhs", "rsi"),
                ("rhs", "rdi"),
                ("minuend", "rcx")
            ]
        ),
        [0x48, 0x89, 0xF0, 0x48, 0x0F, 0xAF, 0xC7, 0x48, 0x89, 0xCB, 0x48, 0x29, 0xC3]
    );
}

#[test]
fn add_imm_sub_imm() {
    // add_imm rax, rax, 16 : dst==src → add rax, imm32 = 48 81 C0 10 00 00 00
    assert_eq!(
        bytes("add_imm", &[("dst", "rax"), ("src", "rax"), ("imm", "16")]),
        [0x48, 0x81, 0xC0, 0x10, 0, 0, 0]
    );
    // sub_imm rbx, rbx, 1 : 48 81 EB 01 00 00 00
    assert_eq!(
        bytes("sub_imm", &[("dst", "rbx"), ("src", "rbx"), ("imm", "1")]),
        [0x48, 0x81, 0xEB, 0x01, 0, 0, 0]
    );
}

#[test]
fn add_sp_sub_sp() {
    // add rsp, 32 : 48 81 C4 20 00 00 00
    assert_eq!(
        bytes("add_sp", &[("imm", "32")]),
        [0x48, 0x81, 0xC4, 0x20, 0, 0, 0]
    );
    // sub rsp, 32 : 48 81 EC 20 00 00 00
    assert_eq!(
        bytes("sub_sp", &[("imm", "32")]),
        [0x48, 0x81, 0xEC, 0x20, 0, 0, 0]
    );
}

#[test]
fn cmp_cmp_imm() {
    // cmp rax, rbx : 39 /r rm=rax reg=rbx → 48 39 D8
    assert_eq!(
        bytes("cmp", &[("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x39, 0xD8]
    );
    // cmp rax, 0 : 48 81 F8 00 00 00 00
    assert_eq!(
        bytes("cmp_imm", &[("lhs", "rax"), ("rhs", "0")]),
        [0x48, 0x81, 0xF8, 0, 0, 0, 0]
    );
}

#[test]
fn shifts_imm() {
    // lsl_imm rax, rax, 3 : dst==src → shl rax,3 = 48 C1 E0 03
    assert_eq!(
        bytes("lsl_imm", &[("dst", "rax"), ("src", "rax"), ("shift", "3")]),
        [0x48, 0xC1, 0xE0, 0x03]
    );
    // lsr_imm rax, rax, 1 : shr rax,1 = 48 C1 E8 01
    assert_eq!(
        bytes("lsr_imm", &[("dst", "rax"), ("src", "rax"), ("shift", "1")]),
        [0x48, 0xC1, 0xE8, 0x01]
    );
    // asr_imm rax, rax, 63 : sar rax,63 = 48 C1 F8 3F
    assert_eq!(
        bytes(
            "asr_imm",
            &[("dst", "rax"), ("src", "rax"), ("shift", "63")]
        ),
        [0x48, 0xC1, 0xF8, 0x3F]
    );
}

#[test]
fn shifts_var() {
    // lslv rax, rax, rbx : push rcx ; mov rcx,rbx ; shl rax,cl ; pop rcx (bug-125).
    assert_eq!(
        bytes("lslv", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x51, 0x48, 0x89, 0xD9, 0x48, 0xD3, 0xE0, 0x59]
    );
    // rorv rax, rax, rbx : push rcx ; mov rcx,rbx ; ror rax,cl ; pop rcx.
    assert_eq!(
        bytes("rorv", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x51, 0x48, 0x89, 0xD9, 0x48, 0xD3, 0xC8, 0x59]
    );
    // asrv rax, rax, rbx : push rcx ; mov rcx,rbx ; sar rax,cl ; pop rcx.
    assert_eq!(
        bytes("asrv", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x51, 0x48, 0x89, 0xD9, 0x48, 0xD3, 0xF8, 0x59]
    );
}

#[test]
fn loads() {
    // ldr_u64 rdi, [rsp+16] : 48 8B BC 24 10 00 00 00
    assert_eq!(
        bytes(
            "ldr_u64",
            &[("dst", "rdi"), ("base", "rsp"), ("offset", "16")]
        ),
        [0x48, 0x8B, 0xBC, 0x24, 0x10, 0, 0, 0]
    );
    // ldr_u64 rax, [rbx+8] : 48 8B 83 08 00 00 00  (rbx base, no SIB)
    assert_eq!(
        bytes(
            "ldr_u64",
            &[("dst", "rax"), ("base", "rbx"), ("offset", "8")]
        ),
        [0x48, 0x8B, 0x83, 0x08, 0, 0, 0]
    );
    // ldr_u32 rax, [rbx+0] : 8B 83 00 00 00 00  (no REX.W, zero-extends)
    assert_eq!(
        bytes(
            "ldr_u32",
            &[("dst", "rax"), ("base", "rbx"), ("offset", "0")]
        ),
        [0x8B, 0x83, 0, 0, 0, 0]
    );
    // ldr_u8 rax, [rbx+4] : movzx 48 0F B6 83 04 00 00 00
    assert_eq!(
        bytes(
            "ldr_u8",
            &[("dst", "rax"), ("base", "rbx"), ("offset", "4")]
        ),
        [0x48, 0x0F, 0xB6, 0x83, 0x04, 0, 0, 0]
    );
    // ldr_u16 rax, [rbx+2] : movzx 48 0F B7 83 02 00 00 00
    assert_eq!(
        bytes(
            "ldr_u16",
            &[("dst", "rax"), ("base", "rbx"), ("offset", "2")]
        ),
        [0x48, 0x0F, 0xB7, 0x83, 0x02, 0, 0, 0]
    );
}

#[test]
fn stores() {
    // str_u64 rax, [rbx+8] : 48 89 83 08 00 00 00
    assert_eq!(
        bytes(
            "str_u64",
            &[("src", "rax"), ("base", "rbx"), ("offset", "8")]
        ),
        [0x48, 0x89, 0x83, 0x08, 0, 0, 0]
    );
    // str_u32 rax, [rbx+0] : 89 83 00 00 00 00
    assert_eq!(
        bytes(
            "str_u32",
            &[("src", "rax"), ("base", "rbx"), ("offset", "0")]
        ),
        [0x89, 0x83, 0, 0, 0, 0]
    );
    // str_u8 rax, [rbx+1] : 88 83 01 00 00 00  (rax needs no REX for byte form)
    assert_eq!(
        bytes(
            "str_u8",
            &[("src", "rax"), ("base", "rbx"), ("offset", "1")]
        ),
        [0x88, 0x83, 0x01, 0, 0, 0]
    );
    // str_u8 rsi, [rbx+0] : sil requires REX → 40 88 B3 00 00 00 00
    assert_eq!(
        bytes(
            "str_u8",
            &[("src", "rsi"), ("base", "rbx"), ("offset", "0")]
        ),
        [0x40, 0x88, 0xB3, 0, 0, 0, 0]
    );
}

#[test]
fn branch_self_ret_svc() {
    assert_eq!(bytes("branch_self", &[]), [0xEB, 0xFE]);
    assert_eq!(bytes("ret", &[]), [0xC3]);
    assert_eq!(bytes("svc", &[]), [0x0F, 0x05]);
}

#[test]
fn blr() {
    // blr rax : call rax = FF D0
    assert_eq!(bytes("blr", &[("register", "rax")]), [0xFF, 0xD0]);
    // blr r10 : call r10 = 41 FF D2
    assert_eq!(bytes("blr", &[("register", "r10")]), [0x41, 0xFF, 0xD2]);
}

#[test]
fn branches_are_fixed_size() {
    // jmp rel32 = 5 bytes (E9 + 4 placeholder)
    let jmp = bytes("b", &[("target", "L")]);
    assert_eq!(jmp.len(), 5);
    assert_eq!(jmp[0], 0xE9);
    // je rel32 = 6 bytes (0F 84 + 4)
    let je = bytes("b.eq", &[("target", "L")]);
    assert_eq!(je, [0x0F, 0x84, 0, 0, 0, 0]);
    // jne = 0F 85
    assert_eq!(bytes("b.ne", &[("target", "L")]), [0x0F, 0x85, 0, 0, 0, 0]);
    // jge = 0F 8D
    assert_eq!(bytes("b.ge", &[("target", "L")]), [0x0F, 0x8D, 0, 0, 0, 0]);
    // jl = 0F 8C
    assert_eq!(bytes("b.lt", &[("target", "L")]), [0x0F, 0x8C, 0, 0, 0, 0]);
    // jg = 0F 8F
    assert_eq!(bytes("b.gt", &[("target", "L")]), [0x0F, 0x8F, 0, 0, 0, 0]);
    // jle = 0F 8E
    assert_eq!(bytes("b.le", &[("target", "L")]), [0x0F, 0x8E, 0, 0, 0, 0]);
    // ja = 0F 87
    assert_eq!(bytes("b.hi", &[("target", "L")]), [0x0F, 0x87, 0, 0, 0, 0]);
    // jb = 0F 82
    assert_eq!(bytes("b.lo", &[("target", "L")]), [0x0F, 0x82, 0, 0, 0, 0]);
}

#[test]
fn call_emits_5_bytes() {
    // Internal `_mfb_*` call: E8 + rel32 placeholder, no variadic al marker
    // (internal functions are never variadic and may pass a 7th arg in rax).
    let internal = bytes("bl", &[("target", "_mfb_some_fn")]);
    assert_eq!(internal, [0xE8, 0, 0, 0, 0]);
    // External (libc) call: `mov eax, 8` (B8 08 ..) then E8 + rel32 — the SysV
    // variadic ABI's vector-arg-count marker.
    let external = bytes("bl", &[("target", "snprintf")]);
    assert_eq!(external, [0xB8, 8, 0, 0, 0, 0xE8, 0, 0, 0, 0]);
}

#[test]
fn lea_rip_relative() {
    // adrp rsi, sym → lea rsi,[rip+disp32] : 48 8D 35 00 00 00 00
    let lea = bytes("adrp", &[("dst", "rsi"), ("symbol", "g")]);
    assert_eq!(lea, [0x48, 0x8D, 0x35, 0, 0, 0, 0]);
    // adrp r8, sym : REX.W.R = 4C → 4C 8D 05 ...
    let lea_r8 = bytes("adrp", &[("dst", "r8"), ("symbol", "g")]);
    assert_eq!(lea_r8, [0x4C, 0x8D, 0x05, 0, 0, 0, 0]);
}

#[test]
fn add_pageoff_is_zero_bytes() {
    assert_eq!(
        bytes(
            "add_pageoff",
            &[("dst", "rsi"), ("src", "rsi"), ("symbol", "g")]
        )
        .len(),
        0
    );
}

#[test]
fn add_carry_no_carry_in() {
    // add_carry dst=rbx carry_out=rsi lhs=rbx rhs=rdi carry_in=xzr :
    // mov rbx,rbx? dst==lhs so skip ; add rbx,rdi (48 01 FB) ;
    // setc sil (40 0F 92 C6) ; movzx rsi,sil (48 0F B6 F6)
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "rsi"),
                ("lhs", "rbx"),
                ("rhs", "rdi"),
                ("carry_in", "xzr")
            ]
        ),
        [0x48, 0x01, 0xFB, 0x40, 0x0F, 0x92, 0xC6, 0x48, 0x0F, 0xB6, 0xF6]
    );
}

#[test]
fn add_carry_with_carry_in() {
    // carry_in = r10 :
    // bt r10,0 (49 0F BA E2 00) sets CF=carry_in non-destructively (bug-125) ;
    // (dst==lhs) adc rbx,rdi (48 11 FB) ;
    // setc sil (40 0F 92 C6) ; movzx rsi,sil (48 0F B6 F6)
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "rsi"),
                ("lhs", "rbx"),
                ("rhs", "rdi"),
                ("carry_in", "r10")
            ]
        ),
        [
            0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x11, 0xFB, 0x40, 0x0F, 0x92, 0xC6, 0x48, 0x0F,
            0xB6, 0xF6
        ]
    );
}

#[test]
fn sub_borrow_no_borrow_in() {
    // sub_borrow dst=rbx borrow_out=rsi lhs=rbx rhs=rdi borrow_in=xzr :
    // sub rbx,rdi (48 29 FB) ; setc sil (40 0F 92 C6) ; movzx rsi,sil (48 0F B6 F6)
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "rsi"),
                ("lhs", "rbx"),
                ("rhs", "rdi"),
                ("borrow_in", "xzr")
            ]
        ),
        [0x48, 0x29, 0xFB, 0x40, 0x0F, 0x92, 0xC6, 0x48, 0x0F, 0xB6, 0xF6]
    );
}

#[test]
fn add_carry_zero_token_rhs_adds_immediate_zero() {
    // bug-154: the PCG64 seed carry-propagation step
    // (entry_and_arena.rs:1994) emits add_carry(hi, ZERO, hi, ZERO, carry) —
    // dst=lhs=hi, rhs=xzr, carry_in=carry. A zero-token rhs must add exactly 0;
    // it must NOT be encoded as `r8` (sentinel 16 & 7 == 0 + REX.R). Expect:
    // bt r10,0 (49 0F BA E2 00) ; adc rbx,0 (48 81 D3 00 00 00 00) ;
    // carry_out=xzr → no setcc.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "xzr"),
                ("lhs", "rbx"),
                ("rhs", "xzr"),
                ("carry_in", "r10")
            ]
        ),
        [0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x81, 0xD3, 0x00, 0x00, 0x00, 0x00]
    );
}

#[test]
fn add_carry_zero_token_rhs_no_carry_in_is_move() {
    // bug-154: carry_in=xzr and rhs=xzr with dst != lhs is just dst = lhs
    // (nothing added). mov rbx,rdi (48 89 FB) ; carry_out=xzr → no setcc.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "xzr"),
                ("lhs", "rdi"),
                ("rhs", "xzr"),
                ("carry_in", "xzr")
            ]
        ),
        [0x48, 0x89, 0xFB]
    );
}

#[test]
fn sub_borrow_zero_token_rhs_subtracts_immediate_zero() {
    // bug-154 (symmetric, latent today): a zero-token rhs must subtract exactly
    // 0. bt r10,0 (49 0F BA E2 00) ; sbb rbx,0 (48 81 DB 00 00 00 00) ;
    // borrow_out=xzr → no setcc.
    assert_eq!(
        bytes(
            "sub_borrow",
            &[
                ("dst", "rbx"),
                ("borrow_out", "xzr"),
                ("lhs", "rbx"),
                ("rhs", "xzr"),
                ("borrow_in", "r10")
            ]
        ),
        [0x49, 0x0F, 0xBA, 0xE2, 0x00, 0x48, 0x81, 0xDB, 0x00, 0x00, 0x00, 0x00]
    );
}

#[test]
fn cmp_zero_token_rhs_compares_immediate_zero() {
    // bug-397: `cmp lhs, xzr` must compare against a true zero. The zero token is
    // sentinel 16, which the raw `alu_rr` reg field would encode as r8 (48 39 →
    // 4C 39 C0 = `cmp rax, r8`), folding a caller's leftover r8 into the compare.
    // Route it through the immediate form instead: cmp rax, 0 = 48 81 F8 00000000.
    assert_eq!(
        bytes("cmp", &[("lhs", "rax"), ("rhs", "xzr")]),
        [0x48, 0x81, 0xF8, 0, 0, 0, 0]
    );
    // r13 exercises a high `lhs` (REX.B on the rm): cmp r13, 0 = 49 81 FD 00000000.
    assert_eq!(
        bytes("cmp", &[("lhs", "r13"), ("rhs", "xzr")]),
        [0x49, 0x81, 0xFD, 0, 0, 0, 0]
    );
}

#[test]
fn cmp_zero_token_lhs_rejected() {
    // bug-397: `cmp xzr, rhs` (flags of `0 - rhs`) has no scratch-free x86 form,
    // and letting sentinel 16 reach `alu_rr`'s rm would silently encode r8. No
    // producer emits it — reject loudly rather than miscompile.
    let ins = CodeInstruction::new("cmp")
        .physical_field("lhs", "xzr")
        .physical_field("rhs", "rax");
    let err = enc_err(&ins);
    assert!(err.contains("zero-token lhs"), "got: {err}");
}

#[test]
fn cmp_imm_zero_token_lhs_rejected() {
    // bug-397: a zero-token `lhs` on cmp_imm would encode via enc_alu_imm32's
    // rm=16 as r8. `cmp 0, imm` is degenerate and unproduced — reject loudly.
    let ins = CodeInstruction::new("cmp_imm")
        .physical_field("lhs", "xzr")
        .physical_field("rhs", "5");
    let err = enc_err(&ins);
    assert!(err.contains("zero-token lhs"), "got: {err}");
}

#[test]
fn add_carry_zero_token_lhs_commutes() {
    // bug-397: a zero-token `lhs` (dst = 0 + rhs + carry) must not let sentinel 16
    // reach enc_mov as r8. add commutes, so `0 + rdi + 0` is just dst = rdi.
    // mov rbx, rdi (48 89 FB) ; carry_out=xzr → no setcc.
    assert_eq!(
        bytes(
            "add_carry",
            &[
                ("dst", "rbx"),
                ("carry_out", "xzr"),
                ("lhs", "xzr"),
                ("rhs", "rdi"),
                ("carry_in", "xzr")
            ]
        ),
        [0x48, 0x89, 0xFB]
    );
    // Both operands zero → result is carry_in only; unproduced, reject loudly.
    let ins = CodeInstruction::new("add_carry")
        .physical_field("dst", "rbx")
        .physical_field("carry_out", "xzr")
        .physical_field("lhs", "xzr")
        .physical_field("rhs", "xzr")
        .physical_field("carry_in", "xzr");
    let err = enc_err(&ins);
    assert!(err.contains("both operands the zero token"), "got: {err}");
}

#[test]
fn sub_borrow_zero_token_lhs_rejected() {
    // bug-397: `dst = 0 - rhs - borrow` does not commute like add_carry's and has
    // no scratch-free form preserving the borrow flag; sentinel 16 would reach
    // enc_mov as r8. Unproduced — reject loudly.
    let ins = CodeInstruction::new("sub_borrow")
        .physical_field("dst", "rbx")
        .physical_field("borrow_out", "xzr")
        .physical_field("lhs", "xzr")
        .physical_field("rhs", "rdi")
        .physical_field("borrow_in", "xzr");
    let err = enc_err(&ins);
    assert!(err.contains("zero-token lhs"), "got: {err}");
}

/// Encode a single instruction to bytes without also asserting the size (used
/// for ops whose byte count we only drive for coverage, not value-check).
fn just_bytes(op: &str, fields: &[(&'static str, &str)]) -> Vec<u8> {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.physical_field(k, v);
    }
    let encoded = encode_instruction(&ins).expect("encode");
    let size = instruction_size(&ins).expect("size");
    assert_eq!(
        encoded.bytes_len(),
        size,
        "size/emit mismatch for op '{op}'"
    );
    encoded.into_bytes()
}

#[test]
fn label_and_pageoff_are_empty() {
    assert!(bytes("label", &[("name", "L")]).is_empty());
    assert!(bytes(
        "add_pageoff",
        &[("dst", "rax"), ("src", "rax"), ("symbol", "g")]
    )
    .is_empty());
}

#[test]
fn clz_lzcnt() {
    // lzcnt rax, rbx : F3 REX.W 0F BD /r → F3 48 0F BD C3
    assert_eq!(
        bytes("clz", &[("dst", "rax"), ("src", "rbx")]),
        [0xF3, 0x48, 0x0F, 0xBD, 0xC3]
    );
    // extended regs set REX.R/REX.B: lzcnt r8, r9
    assert_eq!(
        bytes("clz", &[("dst", "r8"), ("src", "r9")]),
        [0xF3, 0x4D, 0x0F, 0xBD, 0xC1]
    );
}

#[test]
fn rev_word_and_quad() {
    // rev_x rax, rax : bswap rax in place → 48 0F C8
    assert_eq!(
        bytes("rev_x", &[("dst", "rax"), ("src", "rax")]),
        [0x48, 0x0F, 0xC8]
    );
    // rev_x rbx, rax : mov rbx,rax (48 89 C3) ; bswap rbx (48 0F CB)
    assert_eq!(
        bytes("rev_x", &[("dst", "rbx"), ("src", "rax")]),
        [0x48, 0x89, 0xC3, 0x48, 0x0F, 0xCB]
    );
    // rev_w rax, rax : 32-bit bswap in place → 0F C8
    assert_eq!(
        bytes("rev_w", &[("dst", "rax"), ("src", "rax")]),
        [0x0F, 0xC8]
    );
    // rev_w rbx, rax : mov ebx,eax (89 C3) ; bswap ebx (0F CB)
    assert_eq!(
        bytes("rev_w", &[("dst", "rbx"), ("src", "rax")]),
        [0x89, 0xC3, 0x0F, 0xCB]
    );
    // rev_w r8, r9 : mov r8d,r9d ; bswap r8d (32-bit REX.R/B + high bswap).
    assert_eq!(
        bytes("rev_w", &[("dst", "r8"), ("src", "r9")]),
        [0x45, 0x89, 0xC8, 0x41, 0x0F, 0xC8]
    );
    // rev_x r8, r9 : mov r8,r9 ; bswap r8 (64-bit REX.W.R/B + high bswap).
    assert_eq!(
        bytes("rev_x", &[("dst", "r8"), ("src", "r9")]),
        [0x4D, 0x89, 0xC8, 0x49, 0x0F, 0xC8]
    );
}

#[test]
fn rbit_bit_reverse_sequence() {
    // The rbit expansion is long; drive both the in-place and copy-first arms and
    // confirm they encode (byte-exact value is checked at runtime elsewhere).
    let same = just_bytes("rbit", &[("dst", "rbx"), ("src", "rbx")]);
    let diff = just_bytes("rbit", &[("dst", "rbx"), ("src", "rcx")]);
    // The differing form has the extra leading `mov rbx,rcx` (3 bytes).
    assert_eq!(diff.len(), same.len() + 3);
    // Extended-register form drives the REX.B paths.
    let _ = just_bytes("rbit", &[("dst", "r8"), ("src", "r9")]);
}

#[test]
fn smulh_signed_high() {
    // smulh rbx, rsi, rdi : mov rax,rsi ; imul rdi (F7 /5) ; mov rbx,rdx
    assert_eq!(
        bytes("smulh", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdi")]),
        // push rax ; push rdx ; mov rax,rsi ; imul rdi ; mov rbx,rdx ; pop rdx ;
        // pop rax (bug-125 rax/rdx save).
        [0x50, 0x52, 0x48, 0x89, 0xF0, 0x48, 0xF7, 0xEF, 0x48, 0x89, 0xD3, 0x5A, 0x58]
    );
}

#[test]
fn alu_zero_lhs_forms() {
    // and rax, xzr, rcx : 0 & rcx = 0 → xor rax,rax (48 31 C0)
    assert_eq!(
        bytes("and", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rcx")]),
        [0x48, 0x31, 0xC0]
    );
    // orr rax, xzr, rax : dst==rhs → nothing (rax already holds rax)
    assert!(bytes("orr", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rax")]).is_empty());
    // eor rax, xzr, rcx : 0 ^ rcx = rcx → mov rax,rcx
    assert_eq!(
        bytes("eor", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rcx")]),
        [0x48, 0x89, 0xC8]
    );
}

#[test]
fn alu_zero_rhs_rejected() {
    let ins = CodeInstruction::new("add")
        .physical_field("dst", "rax")
        .physical_field("lhs", "rbx")
        .physical_field("rhs", "xzr");
    let err = enc_err(&ins);
    assert!(err.contains("zero-token rhs"), "got: {err}");
}

#[test]
fn alu_dst_equals_rhs_commutes_and_subtracts() {
    // and rax, rcx, rax : dst==rhs commutative → and rax,rcx
    assert_eq!(
        bytes("and", &[("dst", "rax"), ("lhs", "rcx"), ("rhs", "rax")]),
        [0x48, 0x21, 0xC8]
    );
    // The plain disjoint arm: and rdx, rax, rcx → mov rdx,rax ; and rdx,rcx
    assert_eq!(
        bytes("and", &[("dst", "rdx"), ("lhs", "rax"), ("rhs", "rcx")]),
        [0x48, 0x89, 0xC2, 0x48, 0x21, 0xCA]
    );
}

#[test]
fn lsrv_variable_shift() {
    // lsrv rax, rax, rbx : mov rcx,rbx (48 89 D9) ; shr rax,cl (48 D3 E8)
    assert_eq!(
        bytes("lsrv", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        // push rcx ; mov rcx,rbx ; shr rax,cl ; pop rcx — rcx saved (bug-125).
        [0x51, 0x48, 0x89, 0xD9, 0x48, 0xD3, 0xE8, 0x59]
    );
    // Move-in when dst != value: lslv rdx, rax, rbx
    assert_eq!(
        bytes("lslv", &[("dst", "rdx"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x51, 0x48, 0x89, 0xD9, 0x48, 0x89, 0xC2, 0x48, 0xD3, 0xE2, 0x59]
    );
    // Extended dst drives REX.B on the shift.
    // lslv r8, r8, rbx : push rcx ; mov rcx,rbx ; shl r8,cl ; pop rcx.
    assert_eq!(
        bytes("lslv", &[("dst", "r8"), ("lhs", "r8"), ("rhs", "rbx")]),
        [0x51, 0x48, 0x89, 0xD9, 0x49, 0xD3, 0xE0, 0x59]
    );
}

#[test]
fn rorv_word_variable() {
    // rorv_w rax, rax, rbx : mov ecx,ebx (89 D9) ; ror eax,cl (D3 C8)
    assert_eq!(
        bytes("rorv_w", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        // push rcx ; mov ecx,ebx ; ror eax,cl ; pop rcx (bug-125 rcx save).
        [0x51, 0x89, 0xD9, 0xD3, 0xC8, 0x59]
    );
    // move-in + extended-dst arms
    assert_eq!(
        bytes("rorv_w", &[("dst", "rdx"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x51, 0x89, 0xD9, 0x89, 0xC2, 0xD3, 0xCA, 0x59]
    );
    // rorv_w r8, r9, rbx : push rcx ; mov ecx,ebx ; mov r8d,r9d ; ror r8d,cl ; pop rcx.
    assert_eq!(
        bytes("rorv_w", &[("dst", "r8"), ("lhs", "r9"), ("rhs", "rbx")]),
        [0x51, 0x89, 0xD9, 0x45, 0x89, 0xC8, 0x41, 0xD3, 0xC8, 0x59]
    );
}

#[test]
fn u32_load_store_extended() {
    // ldr_u32 r8, [r9+0] drives the REX arm of the U32 load.
    assert_eq!(
        bytes("ldr_u32", &[("dst", "r8"), ("base", "r9"), ("offset", "0")]),
        [0x45, 0x8B, 0x81, 0, 0, 0, 0]
    );
    // str_u32 r8, [r9+0] drives the REX arm of the U32 store.
    assert_eq!(
        bytes("str_u32", &[("src", "r8"), ("base", "r9"), ("offset", "0")]),
        [0x45, 0x89, 0x81, 0, 0, 0, 0]
    );
    // ldr_u16 with base rsp drives the SIB path.
    assert_eq!(
        bytes(
            "ldr_u16",
            &[("dst", "rax"), ("base", "rsp"), ("offset", "2")]
        ),
        [0x48, 0x0F, 0xB7, 0x84, 0x24, 0x02, 0, 0, 0]
    );
}

/// bug-294: `str_u16` DOES have a `CodeOp` mnemonic (`ops.rs` maps
/// `CodeOp::StrU16 <-> "str_u16"`) and the emitter dispatches it at
/// `"str_u16" => mem_store(instruction, MemWidth::U16)`, so the arm was always
/// reachable -- the previous note here claiming otherwise was simply wrong, and it
/// is what let plan-50-D ship an x86 leg that only ever returned an error.
/// aarch64 (STRH) and riscv64 (sh) both encode it.
///
/// Byte-exact against the 0x66 operand-size-prefixed forms.
#[test]
fn str_u16_encodes_the_operand_size_prefixed_store() {
    // mov [rbx+0], ax : 66 89 83 00 00 00 00
    assert_eq!(
        bytes(
            "str_u16",
            &[("src", "rax"), ("base", "rbx"), ("offset", "0")]
        ),
        [0x66, 0x89, 0x83, 0x00, 0x00, 0x00, 0x00]
    );
    // A high source register adds REX.R: mov [rbx+4], r8w : 66 44 89 83 04 …
    assert_eq!(
        bytes(
            "str_u16",
            &[("src", "r8"), ("base", "rbx"), ("offset", "4")]
        ),
        [0x66, 0x44, 0x89, 0x83, 0x04, 0x00, 0x00, 0x00]
    );
    // A high base adds REX.B: mov [r9+2], ax : 66 41 89 81 02 …
    assert_eq!(
        bytes(
            "str_u16",
            &[("src", "rax"), ("base", "r9"), ("offset", "2")]
        ),
        [0x66, 0x41, 0x89, 0x81, 0x02, 0x00, 0x00, 0x00]
    );
    // An rsp base needs the SIB escape: mov [rsp+8], ax : 66 89 84 24 08 …
    assert_eq!(
        bytes(
            "str_u16",
            &[("src", "rax"), ("base", "rsp"), ("offset", "8")]
        ),
        [0x66, 0x89, 0x84, 0x24, 0x08, 0x00, 0x00, 0x00]
    );
    // The zero token stores an immediate 16-bit zero, matching how the other
    // widths handle `abi::ZERO`: mov word [rbx+0], 0 : 66 C7 83 00 00 00 00 00 00
    assert_eq!(
        bytes(
            "str_u16",
            &[("src", "xzr"), ("base", "rbx"), ("offset", "0")]
        ),
        [0x66, 0xC7, 0x83, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    );
}

#[test]
fn str_u8_extended_register() {
    // str_u8 r8, [rbx+0] : REX.B → 44 88 83 ...
    assert_eq!(
        bytes("str_u8", &[("src", "r8"), ("base", "rbx"), ("offset", "0")]),
        [0x44, 0x88, 0x83, 0, 0, 0, 0]
    );
}

/// plan-183-A: the arena zero fill's `rep stosb`. rax/rcx/rdi are saved first and
/// restored last, and the operands travel through the stack, so the op is correct when
/// `base`/`src` are themselves rcx or rdi (the swapped case below) or r8–r15.
#[test]
fn encodes_rep_stosb_zero_around_its_implicit_registers() {
    let tail = [0x31, 0xC0, 0xF3, 0xAA, 0x5F, 0x59, 0x58];
    for (base, count, operands) in [
        ("rsi", "rdx", vec![0x56, 0x52]),
        ("rcx", "rdi", vec![0x51, 0x57]),
        ("r9", "r12", vec![0x41, 0x51, 0x41, 0x54]),
    ] {
        let mut want = vec![0x50, 0x51, 0x57];
        want.extend(operands);
        want.extend([0x59, 0x5F]);
        want.extend(tail);
        assert_eq!(
            enc("rep_stosb_zero", &[("base", base), ("src", count)]),
            want,
            "rep_stosb_zero base={base} src={count}"
        );
    }
}
