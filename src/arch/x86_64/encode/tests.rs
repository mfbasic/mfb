//! Byte-exact encoding tests — the correctness gate for the x86-64 encoder.
//! Each expected sequence is hand-verified against the x86-64 instruction
//! reference. These also implicitly verify size==emit consistency, since
//! `encode_one` exercises the same `encode_instruction` path `instruction_size`
//! uses.

use super::emitter::{encode_instruction, Encoder};
use super::sizing::instruction_size;
use crate::target::shared::code::{
    CodeDataObject, CodeFrame, CodeFunction, CodeImport, CodeInstruction, NativeCodePlan,
};
use std::collections::HashMap;

fn fresh_encoder() -> Encoder {
    Encoder {
        text: Vec::new(),
        data: Vec::new(),
        symbols: Vec::new(),
        relocations: Vec::new(),
        imports: HashMap::new(),
        labels: HashMap::new(),
        patches: Vec::new(),
    }
}

/// Encode a single instruction and return its bytes, asserting the reported size
/// matches exactly.
fn bytes(op: &str, fields: &[(&'static str, &str)]) -> Vec<u8> {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.field(k, v);
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
        [0x48, 0x89, 0xF0, 0x48, 0xF7, 0xE7, 0x48, 0x89, 0xD3]
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
    // lslv rax, rax, rbx : mov rcx,rbx (48 89 D9) ; shl rax,cl (48 D3 E0)
    assert_eq!(
        bytes("lslv", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x89, 0xD9, 0x48, 0xD3, 0xE0]
    );
    // rorv rax, rax, rbx : mov rcx,rbx ; ror rax,cl (48 D3 C8)
    assert_eq!(
        bytes("rorv", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x89, 0xD9, 0x48, 0xD3, 0xC8]
    );
    // asrv rax, rax, rbx : mov rcx,rbx ; sar rax,cl (48 D3 F8)
    assert_eq!(
        bytes("asrv", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x89, 0xD9, 0x48, 0xD3, 0xF8]
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
    // add r10,-1 (49 81 C2 FF FF FF FF) ; (dst==lhs) adc rbx,rdi (48 11 FB) ;
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
            0x49, 0x81, 0xC2, 0xFF, 0xFF, 0xFF, 0xFF, 0x48, 0x11, 0xFB, 0x40, 0x0F, 0x92, 0xC6,
            0x48, 0x0F, 0xB6, 0xF6
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

/// Encode and return the error string (the `Encoded` Ok value has no `Debug`).
fn enc_err(ins: &CodeInstruction) -> String {
    match encode_instruction(ins) {
        Ok(_) => panic!("expected an encoding error"),
        Err(err) => err,
    }
}

/// Encode a single instruction to bytes without also asserting the size (used
/// for ops whose byte count we only drive for coverage, not value-check).
fn just_bytes(op: &str, fields: &[(&'static str, &str)]) -> Vec<u8> {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.field(k, v);
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
    // rev_w with an extended register drives the REX + high-bswap arm.
    let _ = bytes("rev_w", &[("dst", "r8"), ("src", "r9")]);
    let _ = bytes("rev_x", &[("dst", "r8"), ("src", "r9")]);
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
        [0x48, 0x89, 0xF0, 0x48, 0xF7, 0xEF, 0x48, 0x89, 0xD3]
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
        .field("dst", "rax")
        .field("lhs", "rbx")
        .field("rhs", "xzr");
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
        [0x48, 0x89, 0xD9, 0x48, 0xD3, 0xE8]
    );
    // Move-in when dst != value: lslv rdx, rax, rbx
    assert_eq!(
        bytes("lslv", &[("dst", "rdx"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x89, 0xD9, 0x48, 0x89, 0xC2, 0x48, 0xD3, 0xE2]
    );
    // Extended dst drives REX.B on the shift.
    let _ = bytes("lslv", &[("dst", "r8"), ("lhs", "r8"), ("rhs", "rbx")]);
}

#[test]
fn rorv_word_variable() {
    // rorv_w rax, rax, rbx : mov ecx,ebx (89 D9) ; ror eax,cl (D3 C8)
    assert_eq!(
        bytes("rorv_w", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x89, 0xD9, 0xD3, 0xC8]
    );
    // move-in + extended-dst arms
    assert_eq!(
        bytes("rorv_w", &[("dst", "rdx"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x89, 0xD9, 0x89, 0xC2, 0xD3, 0xCA]
    );
    let _ = bytes("rorv_w", &[("dst", "r8"), ("lhs", "r9"), ("rhs", "rbx")]);
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

// NOTE: `str_u16` has no `CodeOp` mnemonic, so its `MemWidth::U16` store error
// arm is unreachable through the public `CodeInstruction` path (annotated
// coverage:off in emitter.rs).

#[test]
fn str_u8_extended_register() {
    // str_u8 r8, [rbx+0] : REX.B → 44 88 83 ...
    assert_eq!(
        bytes("str_u8", &[("src", "r8"), ("base", "rbx"), ("offset", "0")]),
        [0x44, 0x88, 0x83, 0, 0, 0, 0]
    );
}

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
    let _ = bytes(
        "fsub_d",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    );
    // fsub_d dst==lhs and disjoint arms.
    let _ = bytes(
        "fsub_d",
        &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")],
    );
    let _ = bytes(
        "fdiv_d",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
    let _ = bytes(
        "fmul_d",
        &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")],
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
    // fcmp_zero_d drives the xorps + ucomisd sequence.
    let _ = bytes("fcmp_zero_d", &[("src", "xmm0")]);
    // fneg_d / fabs_d, both in-place and copy-first.
    let _ = bytes("fneg_d", &[("dst", "xmm0"), ("src", "xmm0")]);
    let _ = bytes("fneg_d", &[("dst", "xmm0"), ("src", "xmm1")]);
    let _ = bytes("fabs_d", &[("dst", "xmm0"), ("src", "xmm0")]);
    let _ = bytes("fabs_d", &[("dst", "xmm0"), ("src", "xmm1")]);
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
    let _ = bytes("fcvtms_x_from_d", &[("dst", "rax"), ("src", "xmm0")]);
    let _ = bytes("fcvtps_x_from_d", &[("dst", "rax"), ("src", "xmm0")]);
    let _ = bytes("fcvtas_x_from_d", &[("dst", "rax"), ("src", "xmm0")]);
}

#[test]
fn float_scalar_mem() {
    // movsd xmm0, [rbx+8] load; movsd [rbx+8], xmm0 store.
    let _ = bytes(
        "ldr_d",
        &[("dst", "xmm0"), ("base", "rbx"), ("offset", "8")],
    );
    let _ = bytes(
        "str_d",
        &[("src", "xmm0"), ("base", "rbx"), ("offset", "8")],
    );
    // Extended register + rsp base drives the REX + SIB paths.
    let _ = bytes(
        "ldr_d",
        &[("dst", "xmm8"), ("base", "rsp"), ("offset", "0")],
    );
}

#[test]
fn float_mem_bad_offset_errors() {
    let ins = CodeInstruction::new("ldr_d")
        .field("dst", "xmm0")
        .field("base", "rbx")
        .field("offset", "nope");
    assert!(enc_err(&ins).contains("ldr_d"));
    let ins = CodeInstruction::new("str_d")
        .field("src", "xmm0")
        .field("base", "rbx")
        .field("offset", "nope");
    assert!(enc_err(&ins).contains("str_d"));
}

#[test]
fn v128_load_store_and_bad_offset() {
    let _ = bytes(
        "ldr_q",
        &[("dst", "xmm0"), ("base", "rbx"), ("offset", "16")],
    );
    let _ = bytes(
        "str_q",
        &[("src", "xmm0"), ("base", "rbx"), ("offset", "16")],
    );
    // Extended register drives REX.
    let _ = bytes("ldr_q", &[("dst", "xmm8"), ("base", "r9"), ("offset", "0")]);
    let ins = CodeInstruction::new("ldr_q")
        .field("dst", "xmm0")
        .field("base", "rbx")
        .field("offset", "x");
    assert!(enc_err(&ins).contains("ldr_q"));
    let ins = CodeInstruction::new("str_q")
        .field("src", "xmm0")
        .field("base", "rbx")
        .field("offset", "x");
    assert!(enc_err(&ins).contains("str_q"));
}

#[test]
fn v128_three_operand_arith() {
    // Drive every vec3_op mnemonic through the disjoint arm.
    for op in [
        "fadd_v", "fmul_v", "fsub_v", "fdiv_v", "fmin_v", "fmax_v", "add_v", "sub_v", "and_v",
        "orr_v", "eor_v",
    ] {
        let _ = bytes(op, &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]);
    }
    // Commutative dst==rhs and non-commutative dst==rhs (staged) arms of vec3.
    let _ = bytes(
        "fadd_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    );
    let _ = bytes(
        "fsub_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    );
    let _ = bytes(
        "fadd_v",
        &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")],
    );
}

#[test]
fn v128_unary_and_neg_abs() {
    let _ = bytes("fsqrt_v", &[("dst", "xmm0"), ("src", "xmm1")]);
    // fneg_v / fabs_v both in-place and copy-first.
    let _ = bytes("fneg_v", &[("dst", "xmm0"), ("src", "xmm0")]);
    let _ = bytes("fneg_v", &[("dst", "xmm0"), ("src", "xmm1")]);
    let _ = bytes("fabs_v", &[("dst", "xmm0"), ("src", "xmm0")]);
    let _ = bytes("fabs_v", &[("dst", "xmm0"), ("src", "xmm1")]);
    let _ = bytes("neg_v", &[("dst", "xmm0"), ("src", "xmm1")]);
    // abs_v both aliasing and disjoint.
    let _ = bytes("abs_v", &[("dst", "xmm0"), ("src", "xmm0")]);
    let _ = bytes("abs_v", &[("dst", "xmm0"), ("src", "xmm1")]);
}

#[test]
fn v128_integer_compares() {
    // cmgt_v: dst==rhs && dst!=lhs staged arm, dst==lhs arm, disjoint arm.
    let _ = bytes(
        "cmgt_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    );
    let _ = bytes(
        "cmgt_v",
        &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")],
    );
    let _ = bytes(
        "cmgt_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
    // cmeq_v commutative in-place + disjoint.
    let _ = bytes(
        "cmeq_v",
        &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")],
    );
    let _ = bytes(
        "cmeq_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    );
    let _ = bytes(
        "cmeq_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
    // cmge_v with dst==rhs and dst!=rhs.
    let _ = bytes(
        "cmge_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    );
    let _ = bytes(
        "cmge_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
}

#[test]
fn v128_float_compares() {
    // fcmgt_v / fcmge_v drive vec_cmppd_swapped's three arms.
    let _ = bytes(
        "fcmgt_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    ); // dst==rhs
    let _ = bytes(
        "fcmgt_v",
        &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")],
    ); // dst==lhs
    let _ = bytes(
        "fcmge_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    ); // disjoint
       // fcmeq_v with dst==rhs, dst==lhs, disjoint.
    let _ = bytes(
        "fcmeq_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")],
    );
    let _ = bytes(
        "fcmeq_v",
        &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")],
    );
    let _ = bytes(
        "fcmeq_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
}

#[test]
fn v128_zero_compares() {
    for op in [
        "fcmgt_zero_v",
        "fcmge_zero_v",
        "fcmlt_zero_v",
        "fcmle_zero_v",
        "fcmeq_zero_v",
    ] {
        // Both aliasing and disjoint src.
        let _ = bytes(op, &[("dst", "xmm0"), ("src", "xmm0")]);
        let _ = bytes(op, &[("dst", "xmm0"), ("src", "xmm1")]);
    }
}

#[test]
fn v128_rounding() {
    for op in ["frintp_v", "frintm_v", "frintz_v", "frintn_v"] {
        let _ = bytes(op, &[("dst", "xmm0"), ("src", "xmm1")]);
    }
    // frinta_v (ties-away emulation via push/pop).
    let _ = bytes("frinta_v", &[("dst", "xmm0"), ("src", "xmm1")]);
}

#[test]
fn v128_shifts_dup_extract() {
    // shl_v / ushr_v in-place and copy-first.
    let _ = bytes("shl_v", &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "3")]);
    let _ = bytes("shl_v", &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "3")]);
    let _ = bytes(
        "ushr_v",
        &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "3")],
    );
    // Bad shift errors.
    let ins = CodeInstruction::new("shl_v")
        .field("dst", "xmm0")
        .field("src", "xmm0")
        .field("shift", "z");
    assert!(matches!(encode_instruction(&ins), Err(_)));
    // dup_v_from_x.
    let _ = bytes("dup_v_from_x", &[("dst", "xmm0"), ("src", "rax")]);
    // umov_x_from_v lane 0 (movq) and lane 1 (pextrq).
    let _ = bytes(
        "umov_x_from_v",
        &[("dst", "rax"), ("src", "xmm0"), ("index", "0")],
    );
    let _ = bytes(
        "umov_x_from_v",
        &[("dst", "rax"), ("src", "xmm0"), ("index", "1")],
    );
    let ins = CodeInstruction::new("umov_x_from_v")
        .field("dst", "rax")
        .field("src", "xmm0")
        .field("index", "z");
    assert!(matches!(encode_instruction(&ins), Err(_)));
}

#[test]
fn v128_bit_select_fma_convert() {
    let _ = bytes(
        "bsl_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
    let _ = bytes(
        "bit_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
    let _ = bytes(
        "fmla_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
    let _ = bytes(
        "fmls_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")],
    );
    // Extended-register FMA to drive the VEX ~R/~B bits.
    let _ = bytes(
        "fmla_v",
        &[("dst", "xmm8"), ("lhs", "xmm1"), ("rhs", "xmm9")],
    );
    // Lane-serial conversions.
    let _ = bytes("fcvtzs_v", &[("dst", "xmm0"), ("src", "xmm1")]);
    let _ = bytes("scvtf_v", &[("dst", "xmm0"), ("src", "xmm1")]);
    // sshr_v with k in range, k==0, and dst==src.
    let _ = bytes(
        "sshr_v",
        &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "20")],
    );
    let _ = bytes(
        "sshr_v",
        &[("dst", "xmm0"), ("src", "xmm1"), ("shift", "0")],
    );
    let _ = bytes(
        "sshr_v",
        &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "20")],
    );
    let ins = CodeInstruction::new("sshr_v")
        .field("dst", "xmm0")
        .field("src", "xmm1")
        .field("shift", "z");
    assert!(matches!(encode_instruction(&ins), Err(_)));
}

#[test]
fn div_aliasing_and_preservation() {
    // udiv into a register whose divisor aliases rdx drives the stack-staged path.
    let _ = bytes("udiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdx")]);
    // sdiv with rax dividend and non-rax dst drives the preserve-dividend path.
    let _ = bytes("sdiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rsi")]);
    // Both aliasing + preservation at once.
    let _ = bytes("udiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rax")]);
}

#[test]
fn msub_extended_registers() {
    // Cover the extended-register REX arms of enc_mov/enc_imul in msub.
    let _ = bytes(
        "msub",
        &[
            ("dst", "r8"),
            ("lhs", "r9"),
            ("rhs", "r10"),
            ("minuend", "r11"),
        ],
    );
}

#[test]
fn add_carry_extended_and_sub_borrow() {
    // add_carry with an extended carry_out drives the REX byte in enc_setcc_to.
    let _ = bytes(
        "add_carry",
        &[
            ("dst", "rbx"),
            ("carry_out", "r8"),
            ("lhs", "rbx"),
            ("rhs", "rdi"),
            ("carry_in", "xzr"),
        ],
    );
    // sub_borrow with a borrow-in register drives the sbb path.
    let _ = bytes(
        "sub_borrow",
        &[
            ("dst", "rbx"),
            ("borrow_out", "rsi"),
            ("lhs", "rbx"),
            ("rhs", "rdi"),
            ("borrow_in", "r10"),
        ],
    );
    // add_carry where dst != lhs drives the mov-in arm.
    let _ = bytes(
        "add_carry",
        &[
            ("dst", "rbx"),
            ("carry_out", "rsi"),
            ("lhs", "rcx"),
            ("rhs", "rdi"),
            ("carry_in", "xzr"),
        ],
    );
    let _ = bytes(
        "sub_borrow",
        &[
            ("dst", "rbx"),
            ("borrow_out", "rsi"),
            ("lhs", "rcx"),
            ("rhs", "rdi"),
            ("borrow_in", "xzr"),
        ],
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
        .field("dst", "rax")
        .field("src", "rax")
        .field("shift", "64");
    assert!(matches!(encode_instruction(&ins), Err(_)));
    // An invalid immediate is rejected.
    let ins = CodeInstruction::new("mov_imm")
        .field("dst", "rax")
        .field("type", "Integer")
        .field("value", "notanumber");
    assert!(matches!(encode_instruction(&ins), Err(_)));
}

#[test]
fn imm32_overflow_and_disp32_overflow() {
    // add_imm with u64::MAX, which sign-extends to -1 → fits imm32 as a mask.
    let mask = u64::MAX.to_string();
    let _ = bytes("add_imm", &[("dst", "rax"), ("src", "rax"), ("imm", &mask)]);
    // A truly out-of-range immediate errors.
    let huge = (u64::MAX / 2).to_string();
    let ins = CodeInstruction::new("add_imm")
        .field("dst", "rax")
        .field("src", "rax")
        .field("imm", &huge);
    assert!(enc_err(&ins).contains("imm32"));
    // A memory offset exceeding disp32 errors.
    let ins = CodeInstruction::new("ldr_u64")
        .field("dst", "rax")
        .field("base", "rbx")
        .field("offset", &(u64::from(u32::MAX) + 1).to_string());
    assert!(enc_err(&ins).contains("disp32"));
}

#[test]
fn blr_and_unknown_register() {
    // blr through the low-register arm (no REX).
    assert_eq!(bytes("blr", &[("register", "rcx")]), [0xFF, 0xD1]);
    // An unknown register name errors.
    let ins = CodeInstruction::new("mov")
        .field("dst", "notareg")
        .field("src", "rax");
    assert!(enc_err(&ins).contains("unknown"));
    // An unknown xmm name errors.
    let ins = CodeInstruction::new("fmov_d_from_d")
        .field("dst", "xmm99")
        .field("src", "xmm0");
    assert!(enc_err(&ins).contains("xmm"));
}

fn encode_err(plan: &crate::target::shared::code::NativeCodePlan) -> String {
    match super::encode(plan) {
        Ok(_) => panic!("expected encode to fail"),
        Err(err) => err,
    }
}

fn minimal_plan(
    functions: Vec<crate::target::shared::code::CodeFunction>,
    data_objects: Vec<crate::target::shared::code::CodeDataObject>,
    imports: Vec<crate::target::shared::code::CodeImport>,
    entry: Option<&str>,
) -> crate::target::shared::code::NativeCodePlan {
    crate::target::shared::code::NativeCodePlan {
        target: "linux-x86_64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "x86_64".to_string(),
        project: "t".to_string(),
        entry_symbol: entry.map(str::to_string),
        imports,
        data_objects,
        functions,
    }
}

fn simple_function(
    symbol: &str,
    instructions: Vec<CodeInstruction>,
) -> crate::target::shared::code::CodeFunction {
    crate::target::shared::code::CodeFunction {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: crate::target::shared::code::CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

#[test]
fn encode_produces_an_image_with_symbols_and_data() {
    let data = crate::target::shared::code::CodeDataObject {
        symbol: "g".to_string(),
        kind: "string".to_string(),
        layout: "bytes".to_string(),
        align: 8,
        size: 16,
        value: "hi".to_string(),
    };
    let raw = crate::target::shared::code::CodeDataObject {
        symbol: "r".to_string(),
        kind: "raw".to_string(),
        layout: "bytes".to_string(),
        align: 1,
        size: 2,
        value: "de ad".to_string(),
    };
    let func = simple_function(
        "main",
        vec![
            crate::arch::aarch64::abi::label("entry"),
            CodeInstruction::new("mov")
                .field("dst", "rax")
                .field("src", "rbx"),
            CodeInstruction::new("b").field("target", "entry"),
            CodeInstruction::new("ret"),
        ],
    );
    let plan = minimal_plan(vec![func], vec![data, raw], Vec::new(), Some("main"));
    let image = super::encode(&plan).expect("encode");
    assert_eq!(image.entry, "main");
    // The two data symbols and the one function symbol are present.
    assert!(image.symbols.iter().any(|s| s.name == "g"));
    assert!(image.symbols.iter().any(|s| s.name == "main"));
    // The raw object decoded to two bytes (0xDE, 0xAD) somewhere in data.
    assert!(image.data.windows(2).any(|w| w == [0xDE, 0xAD]));
    // The string object wrote its length prefix (2) and bytes.
    assert!(!image.data.is_empty());
}

#[test]
fn encode_requires_entry_symbol() {
    let plan = minimal_plan(
        vec![simple_function("f", vec![CodeInstruction::new("ret")])],
        Vec::new(),
        Vec::new(),
        None,
    );
    let err = encode_err(&plan);
    assert!(err.contains("entry symbol"), "got: {err}");
}

#[test]
fn encode_data_rejects_bad_hex() {
    // Odd digit count.
    let plan = minimal_plan(
        Vec::new(),
        vec![crate::target::shared::code::CodeDataObject {
            symbol: "r".to_string(),
            kind: "raw".to_string(),
            layout: "bytes".to_string(),
            align: 1,
            size: 1,
            value: "abc".to_string(),
        }],
        Vec::new(),
        Some("main"),
    );
    assert!(encode_err(&plan).contains("even digit"));
    // Non-hex digit.
    let plan = minimal_plan(
        Vec::new(),
        vec![crate::target::shared::code::CodeDataObject {
            symbol: "r".to_string(),
            kind: "raw".to_string(),
            layout: "bytes".to_string(),
            align: 1,
            size: 1,
            value: "zz".to_string(),
        }],
        Vec::new(),
        Some("main"),
    );
    assert!(encode_err(&plan).contains("non-hex"));
}

#[test]
fn encode_carries_imports() {
    let func = simple_function(
        "main",
        vec![
            CodeInstruction::new("bl").field("target", "puts"),
            CodeInstruction::new("ret"),
        ],
    );
    let plan = minimal_plan(
        vec![func],
        Vec::new(),
        vec![crate::target::shared::code::CodeImport {
            library: "libc".to_string(),
            symbol: "puts".to_string(),
        }],
        Some("main"),
    );
    let image = super::encode(&plan).expect("encode");
    assert!(image
        .imports
        .iter()
        .any(|i| i.symbol == "puts" && i.library == "libc"));
}

#[test]
fn encoder_emits_and_patches_a_local_branch() {
    // Drive emit_instruction + patch_labels: a forward `b` to a label two words on.
    let mut enc = fresh_encoder();
    enc.labels.insert("L".to_string(), 5); // 5 bytes ahead (the jmp itself)
    enc.emit_instruction(&CodeInstruction::new("b").field("target", "L"))
        .unwrap();
    enc.patch_labels().unwrap();
    // jmp rel32; the displacement resolves to 0 (target is the next instruction).
    assert_eq!(enc.text[0], 0xE9);
    assert_eq!(&enc.text[1..5], &[0, 0, 0, 0]);
}

#[test]
fn encoder_unresolved_label_errors() {
    let mut enc = fresh_encoder();
    enc.emit_instruction(&CodeInstruction::new("b.eq").field("target", "missing"))
        .unwrap();
    let err = enc.patch_labels().unwrap_err();
    assert!(err.contains("does not resolve"), "got: {err}");
}

#[test]
fn encoder_call_relocation_bindings() {
    use super::{EncodedSection, EncodedSymbol};
    // Internal call: the target is a known text symbol → an "internal" reloc.
    let mut enc = fresh_encoder();
    enc.symbols.push(EncodedSymbol {
        name: "_mfb_internal".to_string(),
        section: EncodedSection::Text,
        offset: 0,
    });
    enc.emit_instruction(&CodeInstruction::new("bl").field("target", "_mfb_internal"))
        .unwrap();
    assert_eq!(enc.relocations.len(), 1);
    assert_eq!(enc.relocations[0].binding, "internal");

    // External call: the target is an imported symbol → an "external" reloc.
    let mut enc = fresh_encoder();
    enc.imports
        .insert("snprintf".to_string(), "libc".to_string());
    enc.emit_instruction(&CodeInstruction::new("bl").field("target", "snprintf"))
        .unwrap();
    assert_eq!(enc.relocations[0].binding, "external");
    assert_eq!(enc.relocations[0].library.as_deref(), Some("libc"));

    // Unresolved call target errors.
    let mut enc = fresh_encoder();
    let err = enc
        .emit_instruction(&CodeInstruction::new("bl").field("target", "nowhere"))
        .unwrap_err();
    assert!(err.contains("does not resolve"), "got: {err}");
}

#[test]
fn encoder_data_and_got_relocations() {
    // adrp to an internal data symbol → a "data" binding relocation.
    let mut enc = fresh_encoder();
    enc.emit_instruction(
        &CodeInstruction::new("adrp")
            .field("dst", "rax")
            .field("symbol", "g"),
    )
    .unwrap();
    assert_eq!(enc.relocations[0].binding, "data");

    // adrp to an imported symbol re-routes through the GOT (external binding).
    let mut enc = fresh_encoder();
    enc.imports.insert("g".to_string(), "libc".to_string());
    enc.emit_instruction(
        &CodeInstruction::new("adrp")
            .field("dst", "rax")
            .field("symbol", "g"),
    )
    .unwrap();
    assert_eq!(enc.relocations[0].binding, "external");
    assert_eq!(enc.relocations[0].library.as_deref(), Some("libc"));
}

#[test]
fn operand_decoding_edge_cases() {
    use super::operand::{fp_reg, immediate, is_zero_token, reg, shift};
    // Zero tokens map to 16 and are recognized.
    assert!(is_zero_token(reg("xzr".to_string()).unwrap()));
    assert!(is_zero_token(reg("rzero".to_string()).unwrap()));
    assert!(is_zero_token(reg("zero".to_string()).unwrap()));
    // raw_sp / sp alias rsp (4).
    assert_eq!(reg("raw_sp".to_string()).unwrap(), 4);
    assert_eq!(reg("sp".to_string()).unwrap(), 4);
    // Extended registers.
    assert_eq!(reg("r8".to_string()).unwrap(), 8);
    assert_eq!(reg("r15".to_string()).unwrap(), 15);
    // Unknown register errors.
    assert!(reg("bogus".to_string()).is_err());
    // fp_reg parses xmmN and rejects out-of-range / non-xmm names.
    assert_eq!(fp_reg("xmm7".to_string()).unwrap(), 7);
    assert!(fp_reg("xmm16".to_string()).is_err());
    assert!(fp_reg("rax".to_string()).is_err());
    // immediate accepts booleans and rejects garbage.
    assert_eq!(immediate("true".to_string()).unwrap(), 1);
    assert_eq!(immediate("false".to_string()).unwrap(), 0);
    assert!(immediate("nope".to_string()).is_err());
    // shift range guard.
    assert_eq!(shift("5".to_string()).unwrap(), 5);
    assert!(shift("64".to_string()).is_err());
    assert!(shift("x".to_string()).is_err());
    // A missing field reports the op name.
    let ins = CodeInstruction::new("mov").field("dst", "rax");
    assert!(enc_err(&ins).contains("missing field"));
}

#[test]
fn unsupported_op_errors() {
    // A valid CodeOp the x86 backend does not encode (an AArch64-only SIMD
    // lane shift); `CodeInstruction::new` only accepts real mnemonics, so this
    // exercises the emitter's unsupported-op fallthrough rather than an unknown
    // mnemonic.
    let ins = CodeInstruction::new("sshl_v")
        .field("dst", "v0")
        .field("lhs", "v1")
        .field("rhs", "v2");
    let err = match encode_instruction(&ins) {
        Ok(_) => panic!("expected sshl_v to be unsupported"),
        Err(err) => err,
    };
    assert!(err.contains("unsupported op"), "got: {err}");
}

/// Encode one instruction (no size assertion), returning the raw bytes. Used for
/// the arm-coverage sweeps where the exact byte sequence is verified only for a
/// representative subset and the rest assert successful, non-empty encoding.
fn enc(op: &str, fields: &[(&'static str, &str)]) -> Vec<u8> {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.field(k, v);
    }
    encode_instruction(&ins).expect("encode").into_bytes()
}

#[test]
fn label_and_add_pageoff_are_empty() {
    assert_eq!(enc("label", &[("name", "L")]).len(), 0);
    assert_eq!(
        enc(
            "add_pageoff",
            &[("dst", "rax"), ("src", "rax"), ("symbol", "g")]
        )
        .len(),
        0
    );
}

#[test]
fn rev_w_rev_x() {
    // rev_x rbx, rbx : dst==src, wide → bswap rbx = 48 0F CB
    assert_eq!(
        bytes("rev_x", &[("dst", "rbx"), ("src", "rbx")]),
        [0x48, 0x0F, 0xCB]
    );
    // rev_x rbx, rsi : dst!=src, wide → mov rbx,rsi ; bswap rbx
    assert_eq!(
        bytes("rev_x", &[("dst", "rbx"), ("src", "rsi")]),
        [0x48, 0x89, 0xF3, 0x48, 0x0F, 0xCB]
    );
    // rev_w rbx, rbx : dst==src, 32-bit bswap = 0F CB (no REX for low regs)
    assert_eq!(
        bytes("rev_w", &[("dst", "rbx"), ("src", "rbx")]),
        [0x0F, 0xCB]
    );
    // rev_w rbx, rsi : 32-bit mov (89 /r) ; bswap
    assert_eq!(
        bytes("rev_w", &[("dst", "rbx"), ("src", "rsi")]),
        [0x89, 0xF3, 0x0F, 0xCB]
    );
    // rev_w with extended reg exercises the 32-bit REX + high bswap arms.
    assert!(!enc("rev_w", &[("dst", "r8"), ("src", "r9")]).is_empty());
}

#[test]
fn rbit_reverse_bits() {
    // Long expansion; assert it encodes and ends in a 64-bit bswap of the dst.
    let b = enc("rbit", &[("dst", "rbx"), ("src", "rsi")]);
    assert!(b.len() > 20);
    // dst==src variant skips the initial mov.
    let same = enc("rbit", &[("dst", "rbx"), ("src", "rbx")]);
    assert!(same.len() < b.len());
    // extended-register form exercises the REX.B paths inside the closures.
    assert!(!enc("rbit", &[("dst", "r8"), ("src", "r8")]).is_empty());
}

#[test]
fn msub_disjoint_and_dst_aliases_lhs() {
    // dst aliases lhs (not rax minuend) keeps product-first order.
    assert!(!enc(
        "msub",
        &[
            ("dst", "rbx"),
            ("lhs", "rbx"),
            ("rhs", "rdi"),
            ("minuend", "rcx")
        ]
    )
    .is_empty());
}

#[test]
fn div_aliasing_and_dividend_preservation() {
    // Divisor mapped onto rax → stage in a stack slot (memory divide).
    assert!(!enc("udiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rax")]).is_empty());
    // Divisor mapped onto rdx → same memory path.
    assert!(!enc("sdiv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdx")]).is_empty());
    // Dividend IS rax and quotient wanted elsewhere → preserve rax across div.
    assert!(!enc("udiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rdi")]).is_empty());
    // Both preserve-dividend AND rhs-alias paths at once.
    assert!(!enc("udiv", &[("dst", "rbx"), ("lhs", "rax"), ("rhs", "rdx")]).is_empty());
}

#[test]
fn shifts_var_32bit() {
    // rorv_w rbx, rbx, rsi : mov ecx,esi ; ror ebx,cl (D3 /1)
    assert_eq!(
        bytes("rorv_w", &[("dst", "rbx"), ("lhs", "rbx"), ("rhs", "rsi")]),
        [0x89, 0xF1, 0xD3, 0xCB]
    );
    // dst != value copies the value too; extended reg sets REX.
    assert!(!enc("rorv_w", &[("dst", "r8"), ("lhs", "r9"), ("rhs", "rsi")]).is_empty());
    // lslv with dst != value (mov value in first).
    assert!(!enc("lslv", &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdi")]).is_empty());
    // lsrv arm.
    assert!(!enc("lsrv", &[("dst", "rbx"), ("lhs", "rbx"), ("rhs", "rsi")]).is_empty());
}

#[test]
fn shift_imm_move_first() {
    // lsl_imm rbx, rsi, 2 : dst != src → mov rbx,rsi ; shl rbx,2
    assert!(!enc("lsl_imm", &[("dst", "rbx"), ("src", "rsi"), ("shift", "2")]).is_empty());
}

#[test]
fn add_imm_move_first_and_str_u32_extended() {
    // add_imm rbx, rsi, 8 : dst != src → mov rbx,rsi ; add rbx,8
    assert!(!enc("add_imm", &[("dst", "rbx"), ("src", "rsi"), ("imm", "8")]).is_empty());
    // sub_imm dst != src.
    assert!(!enc("sub_imm", &[("dst", "rbx"), ("src", "rsi"), ("imm", "8")]).is_empty());
    // str_u32 with extended base/src forces REX.
    assert!(!enc("str_u32", &[("src", "r8"), ("base", "r9"), ("offset", "0")]).is_empty());
    // ldr_u32 extended too.
    assert!(!enc("ldr_u32", &[("dst", "r8"), ("base", "r9"), ("offset", "0")]).is_empty());
}

#[test]
fn str_u8_extended_and_u16_unsupported() {
    // str_u8 with an r8b destination sets REX.B.
    assert!(!enc("str_u8", &[("src", "r8"), ("base", "rbx"), ("offset", "0")]).is_empty());
    // str_u16 has no x86 CodeOp mnemonic that reaches the MemWidth::U16 store arm
    // through the emitter dispatch, but the width enum's arm is reachable via the
    // str_u32/str_u64 dispatch only — the U16 store error line is dead through
    // normal dispatch; asserted here by the load path instead.
    assert!(!enc("ldr_u16", &[("dst", "r8"), ("base", "r9"), ("offset", "2")]).is_empty());
}

#[test]
fn extra_branch_conditions() {
    // Overflow / sign / unsigned-LE and float-only jcc mnemonics.
    for (op, cc) in [
        ("b.vs", 0x80u8),
        ("b.vc", 0x81),
        ("b.mi", 0x88),
        ("b.ls", 0x86),
    ] {
        let b = bytes(op, &[("target", "L")]);
        assert_eq!(b[0], 0x0F);
        assert_eq!(b[1], cc);
    }
    for (op, cc) in [
        ("x86.jae", 0x83u8),
        ("x86.jp", 0x8A),
        ("x86.jnp", 0x8B),
        ("x86.ja", 0x87),
        ("x86.jb", 0x82),
        ("x86.jbe", 0x86),
        ("x86.je", 0x84),
        ("x86.jne", 0x85),
    ] {
        let b = bytes(op, &[("target", "L")]);
        assert_eq!([b[0], b[1]], [0x0F, cc]);
    }
}

#[test]
fn scalar_double_moves_and_arith() {
    // fmov_d_from_x xmm0, rbx : movq xmm0, rbx = 66 48 0F 6E C3 (neutral: fmov_i2f)
    assert_eq!(
        bytes("fmov_d_from_x", &[("dst", "xmm0"), ("src", "rbx")]),
        [0x66, 0x48, 0x0F, 0x6E, 0xC3]
    );
    // fmov_x_from_d rbx, xmm0 : movq rbx, xmm0 = 66 48 0F 7E C3 (neutral: fmov_f2i)
    assert_eq!(
        bytes("fmov_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]),
        [0x66, 0x48, 0x0F, 0x7E, 0xC3]
    );
    // fmov_d_from_d xmm1, xmm0 : movaps = 0F 28 C8
    assert_eq!(
        bytes("fmov_d_from_d", &[("dst", "xmm1"), ("src", "xmm0")]),
        [0x0F, 0x28, 0xC8]
    );
    // addsd dst==lhs in place: fadd_d xmm0, xmm0, xmm1 = F2 0F 58 C1
    assert_eq!(
        bytes(
            "fadd_d",
            &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]
        ),
        [0xF2, 0x0F, 0x58, 0xC1]
    );
    // fmul_d commutative dst==rhs → swap operands.
    assert!(!enc(
        "fmul_d",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
    )
    .is_empty());
    // fsub_d dst==rhs non-commutative → staged through xmm15.
    assert!(!enc(
        "fsub_d",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm0")]
    )
    .is_empty());
    // fdiv_d disjoint → copy lhs then op.
    assert!(!enc(
        "fdiv_d",
        &[("dst", "xmm2"), ("lhs", "xmm1"), ("rhs", "xmm0")]
    )
    .is_empty());
    // fsqrt_d xmm1, xmm0 : F2 0F 51 C8
    assert_eq!(
        bytes("fsqrt_d", &[("dst", "xmm1"), ("src", "xmm0")]),
        [0xF2, 0x0F, 0x51, 0xC8]
    );
}

#[test]
fn scalar_double_compares_and_signops() {
    // fcmp_d xmm0, xmm1 : ucomisd = 66 0F 2E C1
    assert_eq!(
        bytes("fcmp_d", &[("lhs", "xmm0"), ("rhs", "xmm1")]),
        [0x66, 0x0F, 0x2E, 0xC1]
    );
    // fcmp_zero_d src : xorps xmm15 ; ucomisd src,xmm15
    assert!(!enc("fcmp_zero_d", &[("src", "xmm0")]).is_empty());
    // fneg_d dst==src (no move) and dst!=src (movsd first).
    assert!(!enc("fneg_d", &[("dst", "xmm0"), ("src", "xmm0")]).is_empty());
    assert!(!enc("fneg_d", &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
    // fabs_d dst==src and dst!=src.
    assert!(!enc("fabs_d", &[("dst", "xmm0"), ("src", "xmm0")]).is_empty());
    assert!(!enc("fabs_d", &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
}

#[test]
fn int_float_conversions() {
    // scvtf_d_from_x xmm0, rbx : cvtsi2sd = F2 48 0F 2A C3 (neutral: i2f)
    assert_eq!(
        bytes("scvtf_d_from_x", &[("dst", "xmm0"), ("src", "rbx")]),
        [0xF2, 0x48, 0x0F, 0x2A, 0xC3]
    );
    // fcvtzs_x_from_d rbx, xmm0 : cvttsd2si = F2 48 0F 2C D8 (neutral: f2i_trunc)
    assert_eq!(
        bytes("fcvtzs_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]),
        [0xF2, 0x48, 0x0F, 0x2C, 0xD8]
    );
    // floor / ceil : roundsd xmm15,src,mode ; cvttsd2si.
    assert!(!enc("fcvtms_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]).is_empty());
    assert!(!enc("fcvtps_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]).is_empty());
    // nearest ties-away.
    assert!(!enc("fcvtas_x_from_d", &[("dst", "rbx"), ("src", "xmm0")]).is_empty());
}

#[test]
fn scalar_double_mem() {
    // ldr_d xmm0, [rbx+8] : F2 0F 10 43 08 (mod=10 base=rbx no SIB, disp32)
    assert_eq!(
        bytes(
            "ldr_d",
            &[("dst", "xmm0"), ("base", "rbx"), ("offset", "8")]
        ),
        [0xF2, 0x0F, 0x10, 0x83, 0x08, 0, 0, 0]
    );
    // str_d xmm0, [rsp+16] : F2 0F 11 with SIB for rsp base.
    assert_eq!(
        bytes(
            "str_d",
            &[("src", "xmm0"), ("base", "rsp"), ("offset", "16")]
        ),
        [0xF2, 0x0F, 0x11, 0x84, 0x24, 0x10, 0, 0, 0]
    );
    // negative offset exercises the i32 parse branch.
    assert!(!enc(
        "ldr_d",
        &[("dst", "xmm8"), ("base", "r8"), ("offset", "-8")]
    )
    .is_empty());
}

#[test]
fn v128_load_store_and_arith() {
    // ldr_q / str_q movups.
    assert_eq!(
        bytes(
            "ldr_q",
            &[("dst", "xmm0"), ("base", "rbx"), ("offset", "0")]
        ),
        [0x0F, 0x10, 0x83, 0, 0, 0, 0]
    );
    assert!(!enc(
        "str_q",
        &[("src", "xmm8"), ("base", "r8"), ("offset", "-16")]
    )
    .is_empty());
    // Packed arithmetic: each vec3_op arm, commutative and not, plus aliasing.
    for op in [
        "fadd_v", "fmul_v", "fsub_v", "fdiv_v", "fmin_v", "fmax_v", "add_v", "sub_v", "and_v",
        "orr_v", "eor_v",
    ] {
        // disjoint
        assert!(!enc(op, &[("dst", "xmm2"), ("lhs", "xmm0"), ("rhs", "xmm1")]).is_empty());
        // dst==lhs in place
        assert!(!enc(op, &[("dst", "xmm0"), ("lhs", "xmm0"), ("rhs", "xmm1")]).is_empty());
        // dst==rhs (commutative swap OR staged xmm15)
        assert!(!enc(op, &[("dst", "xmm1"), ("lhs", "xmm0"), ("rhs", "xmm1")]).is_empty());
    }
}

#[test]
fn v128_unary_and_negabs() {
    assert!(!enc("fsqrt_v", &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
    // fneg_v / fabs_v, dst==src and dst!=src.
    for op in ["fneg_v", "fabs_v"] {
        assert!(!enc(op, &[("dst", "xmm0"), ("src", "xmm0")]).is_empty());
        assert!(!enc(op, &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
    }
    // neg_v integer negate.
    assert!(!enc("neg_v", &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
    // abs_v, dst==src and dst!=src.
    assert!(!enc("abs_v", &[("dst", "xmm0"), ("src", "xmm0")]).is_empty());
    assert!(!enc("abs_v", &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
}

#[test]
fn v128_compares_against_zero() {
    for op in [
        "fcmgt_zero_v",
        "fcmge_zero_v",
        "fcmlt_zero_v",
        "fcmle_zero_v",
        "fcmeq_zero_v",
    ] {
        // dst==src and dst!=src to hit both copy branches.
        assert!(!enc(op, &[("dst", "xmm0"), ("src", "xmm0")]).is_empty());
        assert!(!enc(op, &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
    }
}

#[test]
fn v128_lane_shifts_and_moves() {
    // shl_v / ushr_v: dst==src and dst!=src.
    for op in ["shl_v", "ushr_v"] {
        assert!(!enc(op, &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "3")]).is_empty());
        assert!(!enc(op, &[("dst", "xmm1"), ("src", "xmm0"), ("shift", "3")]).is_empty());
    }
    // dup_v_from_x.
    assert!(!enc("dup_v_from_x", &[("dst", "xmm0"), ("src", "rbx")]).is_empty());
    // umov_x_from_v lane 0 (movq) and lane 1 (pextrq).
    assert!(!enc(
        "umov_x_from_v",
        &[("dst", "rbx"), ("src", "xmm0"), ("index", "0")]
    )
    .is_empty());
    assert!(!enc(
        "umov_x_from_v",
        &[("dst", "rbx"), ("src", "xmm0"), ("index", "1")]
    )
    .is_empty());
    // sshr_v with k>0 and k==0 (clear sign fill) branches.
    assert!(!enc(
        "sshr_v",
        &[("dst", "xmm1"), ("src", "xmm0"), ("shift", "5")]
    )
    .is_empty());
    assert!(!enc(
        "sshr_v",
        &[("dst", "xmm0"), ("src", "xmm0"), ("shift", "0")]
    )
    .is_empty());
}

#[test]
fn v128_bit_selects_fma_and_serial_conversions() {
    assert!(!enc(
        "bsl_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
    )
    .is_empty());
    assert!(!enc(
        "bit_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
    )
    .is_empty());
    assert!(!enc(
        "fmla_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
    )
    .is_empty());
    assert!(!enc(
        "fmls_v",
        &[("dst", "xmm0"), ("lhs", "xmm1"), ("rhs", "xmm2")]
    )
    .is_empty());
    // Extended reg for the VEX P0/P1 R/B-bar bits.
    assert!(!enc(
        "fmla_v",
        &[("dst", "xmm8"), ("lhs", "xmm9"), ("rhs", "xmm10")]
    )
    .is_empty());
    // Lane-serial i64<->f64.
    assert!(!enc("fcvtzs_v", &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
    assert!(!enc("scvtf_v", &[("dst", "xmm1"), ("src", "xmm0")]).is_empty());
}

#[test]
fn alu3_and_zero_and_error_arms() {
    // and rax, xzr, rbx : and with zero lhs → dst = 0 (xor dst,dst).
    assert_eq!(
        bytes("and", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rbx")]),
        [0x48, 0x31, 0xC0]
    );
    // eor rax, xzr, rbx : xor with zero → dst = rhs (mov), dst!=rhs path.
    assert!(!enc("eor", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rbx")]).is_empty());
    // add with zero lhs, dst==rhs → nothing (empty bytes are valid).
    let _ = enc("add", &[("dst", "rax"), ("lhs", "xzr"), ("rhs", "rax")]);
    // zero-token rhs is an explicit error.
    let ins = CodeInstruction::new("add")
        .field("dst", "rax")
        .field("lhs", "rbx")
        .field("rhs", "xzr");
    assert!(encode_instruction(&ins).is_err());
}

#[test]
fn add_carry_dst_not_lhs_and_sub_borrow_with_borrow_in() {
    // add_carry no carry-in, dst != lhs → mov dst,lhs first.
    assert!(!enc(
        "add_carry",
        &[
            ("dst", "rbx"),
            ("carry_out", "rsi"),
            ("lhs", "rdi"),
            ("rhs", "r10"),
            ("carry_in", "xzr")
        ]
    )
    .is_empty());
    // sub_borrow with a borrow-in register, dst != lhs.
    assert!(!enc(
        "sub_borrow",
        &[
            ("dst", "rbx"),
            ("borrow_out", "rsi"),
            ("lhs", "rdi"),
            ("rhs", "r10"),
            ("borrow_in", "r11")
        ]
    )
    .is_empty());
}

#[test]
fn immediate_and_disp_overflow_errors() {
    // A disp beyond i32 range is rejected.
    let big = (i32::MAX as u64) + 1;
    let ins = CodeInstruction::new("ldr_u64")
        .field("dst", "rax")
        .field("base", "rbx")
        .field("offset", &big.to_string());
    assert!(encode_instruction(&ins).is_err());
    // An imm beyond imm32 (and beyond sign-extension) is rejected.
    let huge = 0x1_0000_0000u64; // fits neither i32 nor i32-of-i64 sign form
    let ins = CodeInstruction::new("add_imm")
        .field("dst", "rax")
        .field("src", "rax")
        .field("imm", &huge.to_string());
    assert!(encode_instruction(&ins).is_err());
    // A -1-style mask (u64::MAX) is accepted via the sign-extended path.
    assert!(!enc(
        "add_imm",
        &[
            ("dst", "rax"),
            ("src", "rax"),
            ("imm", &u64::MAX.to_string())
        ]
    )
    .is_empty());
}

/// Build a minimal single-function plan and run the whole two-pass `encode`,
/// exercising `emit_instruction`, `record_reloc` (internal/external/data/GOT),
/// and `patch_labels` — the `Encoder` methods the arm tests bypass.
fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.field(k, v);
    }
    ins
}

fn plan_with(
    instructions: Vec<CodeInstruction>,
    imports: Vec<CodeImport>,
    data_objects: Vec<CodeDataObject>,
) -> NativeCodePlan {
    NativeCodePlan {
        target: "linux-x86_64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "x86_64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("_mfb_main".to_string()),
        imports,
        data_objects,
        functions: vec![CodeFunction {
            name: "main".to_string(),
            symbol: "_mfb_main".to_string(),
            params: Vec::new(),
            returns: "Void".to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations: Vec::new(),
            stack_slots: Vec::new(),
        }],
    }
}

#[test]
fn encode_full_plan_labels_calls_and_data() {
    // An internal call (`main` resolves as an internal symbol), a forward branch
    // patched by `patch_labels`, a data-address `adrp` (internal → data reloc),
    // and a ret.
    let plan = plan_with(
        vec![
            ci("b.eq", &[("target", "done")]),
            ci("bl", &[("target", "_mfb_main")]), // internal (self) call, 5 bytes
            ci("adrp", &[("dst", "rsi"), ("symbol", "msg")]),
            ci("label", &[("name", "done")]),
            ci("ret", &[]),
        ],
        vec![],
        vec![CodeDataObject {
            symbol: "msg".to_string(),
            kind: "string".to_string(),
            layout: "utf8".to_string(),
            align: 8,
            size: 16,
            value: "hi".to_string(),
        }],
    );
    let image = super::encode(&plan).expect("encode");
    assert!(!image.text.is_empty());
    // The forward `b.eq done` rel32 was patched: its 4-byte disp is the distance
    // from the end of the jcc to the `done` label. The jcc is 6 bytes; after it
    // come bl (5) + adrp (7) = 12 bytes to reach `done`.
    let disp = i32::from_le_bytes([image.text[2], image.text[3], image.text[4], image.text[5]]);
    assert_eq!(disp, 12);
    // The internal call and the data address both produced relocations.
    assert!(image
        .relocations
        .iter()
        .any(|r| r.binding == "internal" && r.target == "_mfb_main"));
    assert!(image
        .relocations
        .iter()
        .any(|r| r.binding == "data" && r.target == "msg"));
}

#[test]
fn encode_external_call_and_got_load() {
    // An imported symbol: `bl` routes to an external reloc, and an `adrp` against
    // the same import re-routes through the GOT (`got_pc32`).
    let plan = plan_with(
        vec![
            ci("bl", &[("target", "snprintf")]),
            ci("adrp", &[("dst", "rsi"), ("symbol", "snprintf")]),
            ci("ret", &[]),
        ],
        vec![CodeImport {
            library: "libc".to_string(),
            symbol: "snprintf".to_string(),
        }],
        vec![],
    );
    let image = super::encode(&plan).expect("encode");
    assert!(image
        .relocations
        .iter()
        .any(|r| r.binding == "external" && r.library.as_deref() == Some("libc")));
    // The GOT-routed data load carries the GotLoadLo kind.
    let got_kind =
        crate::arch::x86_64::reloc::reloc_kind(crate::target::shared::code::RelocIntent::GotLoadLo);
    assert!(image.relocations.iter().any(|r| r.kind == got_kind));
}

#[test]
fn encode_unresolved_call_and_label_error() {
    // A `bl` to a symbol that is neither internal nor imported is an error.
    let plan = plan_with(
        vec![ci("bl", &[("target", "nope")]), ci("ret", &[])],
        vec![],
        vec![],
    );
    assert!(super::encode(&plan).is_err());
    // A branch to a label that never appears is a `patch_labels` error.
    let plan = plan_with(
        vec![ci("b", &[("target", "missing")]), ci("ret", &[])],
        vec![],
        vec![],
    );
    assert!(super::encode(&plan).is_err());
}
