//! Byte-exact encoding tests — the correctness gate for the x86-64 encoder.
//! Each expected sequence is hand-verified against the x86-64 instruction
//! reference. These also implicitly verify size==emit consistency, since
//! `encode_one` exercises the same `encode_instruction` path `instruction_size`
//! uses.

use super::emitter::encode_instruction;
use super::sizing::instruction_size;
use crate::target::shared::code::CodeInstruction;

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
    assert_eq!(bytes("mov", &[("dst", "rax"), ("src", "rbx")]), [0x48, 0x89, 0xD8]);
    // mov r8, r15 : REX.W.R.B 89 /r → 4D 89 F8
    assert_eq!(bytes("mov", &[("dst", "r8"), ("src", "r15")]), [0x4D, 0x89, 0xF8]);
}

#[test]
fn mov_imm64() {
    // mov rax, 1 : 48 B8 + 8-byte imm
    assert_eq!(
        bytes("mov_imm", &[("dst", "rax"), ("type", "Integer"), ("value", "1")]),
        [0x48, 0xB8, 0x01, 0, 0, 0, 0, 0, 0, 0]
    );
    // mov r15, 0 : REX.W.B 49 BF + 8 zero bytes
    assert_eq!(
        bytes("mov_imm", &[("dst", "r15"), ("type", "Integer"), ("value", "0")]),
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
fn mvn() {
    // mvn rax, rbx : mov rax,rbx (48 89 D8) ; not rax (48 F7 D0)
    assert_eq!(
        bytes("mvn", &[("dst", "rax"), ("src", "rbx")]),
        [0x48, 0x89, 0xD8, 0x48, 0xF7, 0xD0]
    );
}

#[test]
fn mul_low() {
    // mul rax, rax, rbx : dst==lhs → mov rax,rax (48 89 C0) ; imul rax,rbx
    // imul rax,rbx = 48 0F AF C3
    assert_eq!(
        bytes("mul", &[("dst", "rax"), ("lhs", "rax"), ("rhs", "rbx")]),
        [0x48, 0x89, 0xC0, 0x48, 0x0F, 0xAF, 0xC3]
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
    // msub rbx, rsi, rdi, rax : mov rax,rsi (48 89 F0) ; imul rax,rdi (48 0F AF C7);
    // mov rbx,rax (48 89 C3) ; sub rbx,rax (48 29 C3)
    assert_eq!(
        bytes(
            "msub",
            &[("dst", "rbx"), ("lhs", "rsi"), ("rhs", "rdi"), ("minuend", "rax")]
        ),
        [0x48, 0x89, 0xF0, 0x48, 0x0F, 0xAF, 0xC7, 0x48, 0x89, 0xC3, 0x48, 0x29, 0xC3]
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
    assert_eq!(bytes("add_sp", &[("imm", "32")]), [0x48, 0x81, 0xC4, 0x20, 0, 0, 0]);
    // sub rsp, 32 : 48 81 EC 20 00 00 00
    assert_eq!(bytes("sub_sp", &[("imm", "32")]), [0x48, 0x81, 0xEC, 0x20, 0, 0, 0]);
}

#[test]
fn cmp_cmp_imm() {
    // cmp rax, rbx : 39 /r rm=rax reg=rbx → 48 39 D8
    assert_eq!(bytes("cmp", &[("lhs", "rax"), ("rhs", "rbx")]), [0x48, 0x39, 0xD8]);
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
        bytes("asr_imm", &[("dst", "rax"), ("src", "rax"), ("shift", "63")]),
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
        bytes("ldr_u64", &[("dst", "rdi"), ("base", "rsp"), ("offset", "16")]),
        [0x48, 0x8B, 0xBC, 0x24, 0x10, 0, 0, 0]
    );
    // ldr_u64 rax, [rbx+8] : 48 8B 83 08 00 00 00  (rbx base, no SIB)
    assert_eq!(
        bytes("ldr_u64", &[("dst", "rax"), ("base", "rbx"), ("offset", "8")]),
        [0x48, 0x8B, 0x83, 0x08, 0, 0, 0]
    );
    // ldr_u32 rax, [rbx+0] : 8B 83 00 00 00 00  (no REX.W, zero-extends)
    assert_eq!(
        bytes("ldr_u32", &[("dst", "rax"), ("base", "rbx"), ("offset", "0")]),
        [0x8B, 0x83, 0, 0, 0, 0]
    );
    // ldr_u8 rax, [rbx+4] : movzx 48 0F B6 83 04 00 00 00
    assert_eq!(
        bytes("ldr_u8", &[("dst", "rax"), ("base", "rbx"), ("offset", "4")]),
        [0x48, 0x0F, 0xB6, 0x83, 0x04, 0, 0, 0]
    );
    // ldr_u16 rax, [rbx+2] : movzx 48 0F B7 83 02 00 00 00
    assert_eq!(
        bytes("ldr_u16", &[("dst", "rax"), ("base", "rbx"), ("offset", "2")]),
        [0x48, 0x0F, 0xB7, 0x83, 0x02, 0, 0, 0]
    );
}

#[test]
fn stores() {
    // str_u64 rax, [rbx+8] : 48 89 83 08 00 00 00
    assert_eq!(
        bytes("str_u64", &[("src", "rax"), ("base", "rbx"), ("offset", "8")]),
        [0x48, 0x89, 0x83, 0x08, 0, 0, 0]
    );
    // str_u32 rax, [rbx+0] : 89 83 00 00 00 00
    assert_eq!(
        bytes("str_u32", &[("src", "rax"), ("base", "rbx"), ("offset", "0")]),
        [0x89, 0x83, 0, 0, 0, 0]
    );
    // str_u8 rax, [rbx+1] : 88 83 01 00 00 00  (rax needs no REX for byte form)
    assert_eq!(
        bytes("str_u8", &[("src", "rax"), ("base", "rbx"), ("offset", "1")]),
        [0x88, 0x83, 0x01, 0, 0, 0]
    );
    // str_u8 rsi, [rbx+0] : sil requires REX → 40 88 B3 00 00 00 00
    assert_eq!(
        bytes("str_u8", &[("src", "rsi"), ("base", "rbx"), ("offset", "0")]),
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
    // bl sym : E8 + 4 placeholder
    let call = bytes("bl", &[("target", "_some_fn")]);
    assert_eq!(call, [0xE8, 0, 0, 0, 0]);
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
    assert_eq!(bytes("add_pageoff", &[("dst", "rsi"), ("src", "rsi"), ("symbol", "g")]).len(), 0);
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
            0x49, 0x81, 0xC2, 0xFF, 0xFF, 0xFF, 0xFF, 0x48, 0x11, 0xFB, 0x40, 0x0F, 0x92,
            0xC6, 0x48, 0x0F, 0xB6, 0xF6
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
fn unsupported_op_errors() {
    let ins = CodeInstruction::new("fadd_d")
        .field("dst", "d0")
        .field("lhs", "d1")
        .field("rhs", "d2");
    let err = match encode_instruction(&ins) {
        Ok(_) => panic!("expected fadd_d to be unsupported"),
        Err(err) => err,
    };
    assert!(err.contains("unsupported op"), "got: {err}");
}
