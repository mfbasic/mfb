//! plan-176-O: the x86-64 encoder's integer, move, memory, jump, shift and divide sequences.
//! `Encoder`, the REX/ModRM/SIB helpers and the `InstructionEncoder` impl live in
//! `encode/emitter/mod.rs`.

use super::*;

/// Three-operand ALU (`dst = lhs OP rhs`): `mov dst,lhs` (if needed) then the
/// register-form ALU `dst OP= rhs`. `opcode` is the `MR` reg-form opcode.
pub(super) fn alu3(instruction: &CodeInstruction, opcode: u8) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    if is_zero_token(lhs) {
        // `dst = 0 OP rhs`. AArch64 freely sources `xzr` — most importantly
        // `sub d, xzr, r` to negate — but x86 has no zero register, so the two-
        // operand form `mov dst,<zero>; OP dst,rhs` would compute `rhs OP rhs`
        // whenever `dst == rhs` (the in-place negate). Synthesize the result
        // directly, guarding that aliasing.
        let bytes = match opcode {
            0x29 => {
                // sub: dst = 0 - rhs = -rhs (neg is in-place, so seed dst=rhs).
                let mut b = if dst == rhs {
                    Vec::new()
                } else {
                    enc_mov(dst, rhs)
                };
                b.extend_from_slice(&enc_neg(dst));
                b
            }
            // add / or / xor with zero: dst = rhs.
            0x01 | 0x09 | 0x31 => {
                if dst == rhs {
                    Vec::new()
                } else {
                    enc_mov(dst, rhs)
                }
            }
            // and with zero: dst = 0.
            0x21 => alu_rr(0x31, dst, dst),
            other => {
                return Err(format!(
                    "x86-64 alu3 with a zero lhs and opcode {other:#x} is unsupported"
                ))
            }
        };
        return Ok(Encoded::plain(bytes));
    }
    if is_zero_token(rhs) {
        return Err("x86-64 alu3 with a zero-token rhs is not yet handled".to_string());
    }
    if dst == lhs {
        // dst already holds lhs: operate in place.
        return Ok(Encoded::plain(alu_rr(opcode, dst, rhs)));
    }
    if dst == rhs {
        // dst holds rhs; `mov dst,lhs` would clobber it (giving `lhs OP lhs`).
        let bytes = if opcode == 0x29 {
            // sub is not commutative: dst = lhs - rhs = -(rhs) + lhs.
            let mut b = enc_neg(dst);
            b.extend_from_slice(&alu_rr(0x01, dst, lhs));
            b
        } else {
            // add/and/or/xor commute: `rhs OP lhs == lhs OP rhs`.
            alu_rr(opcode, dst, lhs)
        };
        return Ok(Encoded::plain(bytes));
    }
    let mut bytes = enc_mov(dst, lhs);
    bytes.extend_from_slice(&alu_rr(opcode, dst, rhs));
    Ok(Encoded::plain(bytes))
}

/// `neg r/m64` — REX.W + 0xF7 /3, two's-complement negate in place. `/3` names
/// the NEG operation in the F7 group, so the ModRM reg field is the constant 3
/// (not a register) and only REX.B can extend the `rm` target.
fn enc_neg(target: u8) -> Vec<u8> {
    vec![
        rex(true, false, false, target >= 8),
        0xF7,
        modrm(0b11, 3, target),
    ]
}

/// `mov dst, src` — REX.W 0x89 /r (MR form: rm := reg).
pub(super) fn enc_mov(dst: u8, src: u8) -> Vec<u8> {
    vec![
        rex(true, src >= 8, false, dst >= 8),
        0x89,
        modrm(0b11, src, dst),
    ]
}

/// `mov r64, imm64` — REX.W + (0xB8 + rd) + 8-byte immediate. Always 10 bytes,
/// so the size is trivial and value-independent.
pub(super) fn enc_mov_imm64(dst: u8, value: u64) -> Vec<u8> {
    let mut bytes = vec![rex(true, false, false, dst >= 8), 0xB8 + (dst & 7)];
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

/// `imul dst, src` — two-operand signed multiply, REX.W 0x0F 0xAF /r (RM form:
/// reg := reg * rm), so `dst` is the reg field and `src` the rm field.
pub(super) fn enc_imul_rr(dst: u8, src: u8) -> Vec<u8> {
    vec![
        rex(true, dst >= 8, false, src >= 8),
        0x0F,
        0xAF,
        modrm(0b11, dst, src),
    ]
}

/// `OP r/m64, imm32` group-1 (0x81 /digit). `digit` selects the operation
/// (0=ADD, 5=SUB, 7=CMP). Always uses the imm32 form (6 or 7 bytes) for a
/// value-independent size.
pub(super) fn enc_alu_imm32(digit: u8, rm: u8, imm: i32) -> Vec<u8> {
    let mut bytes = vec![
        rex(true, false, false, rm >= 8),
        0x81,
        modrm(0b11, digit, rm),
    ];
    bytes.extend_from_slice(&imm.to_le_bytes());
    bytes
}

/// `bt r/m64, 0` — REX.W 0F BA /4 ib. Sets CF = bit 0 of `rm` WITHOUT modifying
/// the register. For the {0,1} carry-in domain CF == rm, exactly like the old
/// `add rm,-1` trick but non-destructively (so a carry_in whose interval extends
/// past the op, or aliases lhs, is not corrupted).
fn enc_bt_imm0(rm: u8) -> Vec<u8> {
    vec![
        rex(true, false, false, rm >= 8),
        0x0F,
        0xBA,
        modrm(0b11, 4, rm),
        0x00,
    ]
}

#[derive(Clone, Copy)]
pub(super) enum MemWidth {
    U64,
    U32,
    U16,
    U8,
}

pub(super) fn mem_load(instruction: &CodeInstruction, width: MemWidth) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let base = reg(field(instruction, "base")?)?;
    let disp = checked_disp32(immediate(field(instruction, "offset")?)?)?;
    let mut bytes = Vec::new();
    match width {
        MemWidth::U64 => {
            // mov r64, [base+disp32] : REX.W 0x8B /r
            bytes.push(rex(true, dst >= 8, false, base >= 8));
            bytes.push(0x8B);
            bytes.extend_from_slice(&mem_disp32(dst, base, disp));
        }
        MemWidth::U32 => {
            // mov r32, [base+disp32] : 0x8B /r (no REX.W → zero-extends to 64).
            if dst >= 8 || base >= 8 {
                bytes.push(rex(false, dst >= 8, false, base >= 8));
            }
            bytes.push(0x8B);
            bytes.extend_from_slice(&mem_disp32(dst, base, disp));
        }
        MemWidth::U16 => {
            // movzx r64, word [base+disp32] : REX.W 0x0F 0xB7 /r
            bytes.push(rex(true, dst >= 8, false, base >= 8));
            bytes.push(0x0F);
            bytes.push(0xB7);
            bytes.extend_from_slice(&mem_disp32(dst, base, disp));
        }
        MemWidth::U8 => {
            // movzx r64, byte [base+disp32] : REX.W 0x0F 0xB6 /r
            bytes.push(rex(true, dst >= 8, false, base >= 8));
            bytes.push(0x0F);
            bytes.push(0xB6);
            bytes.extend_from_slice(&mem_disp32(dst, base, disp));
        }
    }
    Ok(Encoded::plain(bytes))
}

pub(super) fn mem_store(instruction: &CodeInstruction, width: MemWidth) -> Result<Encoded, String> {
    let src = reg(field(instruction, "src")?)?;
    let base = reg(field(instruction, "base")?)?;
    let disp = checked_disp32(immediate(field(instruction, "offset")?)?)?;
    // A zero-token store source (`abi::ZERO`, spelled `xzr`) — what AArch64 writes
    // as `str xzr, [..]` to zero arena/record fields — stores an IMMEDIATE zero
    // (`mov r/m, 0`) rather than a pinned zero register. x86 has no zero register,
    // and the immediate form frees `r14` from being reserved to hold 0, so it can
    // join the allocatable pool (plan-34-C: the extra GPR the machine-floor scratch
    // needs to virtualize). `/0` is the ModRM.reg extension for the `C7`/`C6` group.
    if is_zero_token(src) {
        let mut bytes = Vec::new();
        match width {
            MemWidth::U64 => {
                bytes.push(rex(true, false, false, base >= 8));
                bytes.push(0xC7); // mov r/m64, imm32 (sign-extended)
                bytes.extend_from_slice(&mem_disp32(0, base, disp));
                bytes.extend_from_slice(&0i32.to_le_bytes());
            }
            MemWidth::U32 => {
                if base >= 8 {
                    bytes.push(rex(false, false, false, true));
                }
                bytes.push(0xC7); // mov r/m32, imm32
                bytes.extend_from_slice(&mem_disp32(0, base, disp));
                bytes.extend_from_slice(&0i32.to_le_bytes());
            }
            MemWidth::U16 => {
                // bug-294: `mov r/m16, imm16` -- 0x66 selects the 16-bit operand
                // size, and the immediate narrows to 16 bits with it.
                bytes.push(0x66);
                if base >= 8 {
                    bytes.push(rex(false, false, false, true));
                }
                bytes.push(0xC7);
                bytes.extend_from_slice(&mem_disp32(0, base, disp));
                bytes.extend_from_slice(&0i16.to_le_bytes());
            }
            MemWidth::U8 => {
                if base >= 8 {
                    bytes.push(rex(false, false, false, true));
                }
                bytes.push(0xC6); // mov r/m8, imm8
                bytes.extend_from_slice(&mem_disp32(0, base, disp));
                bytes.push(0);
            }
        }
        return Ok(Encoded::plain(bytes));
    }
    let mut bytes = Vec::new();
    match width {
        MemWidth::U64 => {
            // mov [base+disp32], r64 : REX.W 0x89 /r
            bytes.push(rex(true, src >= 8, false, base >= 8));
            bytes.push(0x89);
            bytes.extend_from_slice(&mem_disp32(src, base, disp));
        }
        MemWidth::U32 => {
            // mov [base+disp32], r32 : 0x89 /r
            if src >= 8 || base >= 8 {
                bytes.push(rex(false, src >= 8, false, base >= 8));
            }
            bytes.push(0x89);
            bytes.extend_from_slice(&mem_disp32(src, base, disp));
        }
        MemWidth::U16 => {
            // bug-294: `mov r/m16, r16` -- 0x66 operand-size prefix + the same
            // 0x89 /r form the 32-bit store uses. plan-50-D wired the dispatch
            // here but left both U16 arms erroring, on the mistaken premise that
            // no mnemonic reached them; `ops.rs` maps `CodeOp::StrU16` to
            // `"str_u16"`, so it always did. The prefix precedes any REX.
            bytes.push(0x66);
            if src >= 8 || base >= 8 {
                bytes.push(rex(false, src >= 8, false, base >= 8));
            }
            bytes.push(0x89);
            bytes.extend_from_slice(&mem_disp32(src, base, disp));
        }
        MemWidth::U8 => {
            // mov [base+disp32], r8 : 0x88 /r. A REX prefix (even empty 0x40) is
            // required to address spl/sil/dil/bpl as byte registers; emit REX
            // whenever src is one of rsp/rbp/rsi/rdi or an extended register.
            let need_rex = src >= 8 || base >= 8 || (4..=7).contains(&src);
            if need_rex {
                bytes.push(rex(false, src >= 8, false, base >= 8));
            }
            bytes.push(0x88);
            bytes.extend_from_slice(&mem_disp32(src, base, disp));
        }
    }
    Ok(Encoded::plain(bytes))
}

#[derive(Clone, Copy)]
pub(super) enum JccKind {
    Jmp,
    Je,
    Jne,
    Jge,
    Jl,
    Jg,
    Jle,
    Ja,
    Jb,
    Jo,
    Jno,
    Js,
    Jbe,
    Jae,
    Jp,
    Jnp,
}

pub(super) fn jmp_label(instruction: &CodeInstruction, kind: JccKind) -> Result<Encoded, String> {
    let target = field(instruction, "target")?.into_owned();
    let (bytes, disp_field_offset) = match kind {
        // jmp rel32 : E9 cd  (5 bytes, disp at 1)
        JccKind::Jmp => (vec![0xE9, 0, 0, 0, 0], 1),
        // jcc rel32 : 0F 8x cd  (6 bytes, disp at 2)
        other => {
            let cc = match other {
                JccKind::Je => 0x84,
                JccKind::Jne => 0x85,
                JccKind::Jge => 0x8D,
                JccKind::Jl => 0x8C,
                JccKind::Jg => 0x8F,
                JccKind::Jle => 0x8E,
                JccKind::Ja => 0x87,
                JccKind::Jb => 0x82,
                JccKind::Jo => 0x80,
                JccKind::Jno => 0x81,
                JccKind::Js => 0x88,
                JccKind::Jbe => 0x86,
                JccKind::Jae => 0x83,
                JccKind::Jp => 0x8A,
                JccKind::Jnp => 0x8B,
                JccKind::Jmp => unreachable!(),
            };
            (vec![0x0F, cc, 0, 0, 0, 0], 2)
        }
    };
    Ok(Encoded {
        bytes,
        side_effect: SideEffect::LabelBranch {
            disp_field_offset,
            target,
        },
    })
}

/// Variable shift/rotate by CL: `mov rcx, amount ; mov dst, value ; shift dst,cl`.
/// `digit` is the group-2 /digit (1=ROR, 4=SHL, 5=SHR, 7=SAR). Field convention
/// (abi.rs): `dst`, `lhs` = value, `rhs` = amount.
///
/// rcx is not in the allocatable pool, but ABI-staging and the residual scratch
/// mapping (map_scratch_register uses rcx) can leave a live caller value there
/// across this expansion; the shift-count staging (`mov rcx, amount`) would
/// destroy it. Preserve rcx with push/pop (except when dst==rcx — then rcx is the
/// result carrier). When `value` itself was mapped onto rcx, `mov rcx,amount`
/// overwrites it, so read the preserved copy back from the stack.
pub(super) fn var_shift(instruction: &CodeInstruction, digit: u8) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let value = reg(field(instruction, "lhs")?)?;
    let amount = reg(field(instruction, "rhs")?)?;
    // rcx is the architectural shift count, so when `dst` is coloured onto rcx it
    // cannot also carry the result directly. plan-85: Win64's aligned MFB result
    // bank starts at rcx (`%retMFB0`=rcx), so a variable-shifted result legitimately
    // lands here — do the shift in a scratch register distinct from rcx/value/amount
    // (preserved via push/pop so a live foreign value survives), then move the result
    // into rcx. (bug-284 C6 previously rejected this; the fixpoint's context-sensitive
    // colouring avoided it, but the direct-realization path (plan-85) can produce it.)
    if dst == 1 {
        let scratch = free_shift_scratch(value, amount);
        let mut bytes = vec![0x50 + scratch]; // push scratch
                                              // mov scratch, value  (reads `value` before `mov rcx,amount` clobbers
                                              // rcx, which matters when value aliased rcx).
        bytes.extend_from_slice(&enc_mov(scratch, value));
        bytes.extend_from_slice(&enc_mov(1, amount)); // mov rcx, amount
        bytes.push(rex(true, false, false, false)); // D3 /digit : shift scratch, CL.
        bytes.push(0xD3);
        bytes.push(modrm(0b11, digit, scratch));
        bytes.extend_from_slice(&enc_mov(1, scratch)); // mov rcx, scratch (the result)
        bytes.push(0x58 + scratch); // pop scratch
        return Ok(Encoded::plain(bytes));
    }
    // `dst` is not rcx here (that case returned above), so rcx is always saved.
    let mut bytes = vec![0x51]; // push rcx (preserve foreign/residual value)
    bytes.extend_from_slice(&enc_mov(1, amount)); // mov rcx, amount
    if dst != value {
        if value == 1 {
            // `value` aliased rcx and was just overwritten by the count; reload
            // the preserved copy from [rsp]. mov dst, [rsp] : REX.W 8B /r.
            bytes.push(rex(true, dst >= 8, false, false));
            bytes.push(0x8B);
            bytes.push(modrm(0b00, dst, 4));
            bytes.push(0x24); // SIB: base=rsp
        } else {
            bytes.extend_from_slice(&enc_mov(dst, value)); // mov dst, value
        }
    }
    // D3 /digit : shift r/m64, CL
    bytes.push(rex(true, false, false, dst >= 8));
    bytes.push(0xD3);
    bytes.push(modrm(0b11, digit, dst));
    bytes.push(0x59); // pop rcx
    Ok(Encoded::plain(bytes))
}

/// The register a shift into rcx runs in: the first of rax, rdx and rbx that is
/// neither the value nor the count (rcx itself is the count register). At most two
/// of those three are excluded, so one is always free -- and it is never an
/// extended register, so its push/pop and its moves need no REX prefix of their own.
fn free_shift_scratch(value: u8, amount: u8) -> u8 {
    (0u8..4)
        .find(|&r| r != 1 && r != value && r != amount)
        .expect("rax, rdx and rbx are three candidates and value and amount remove at most two")
}

/// 32-bit variable shift/rotate by CL: `mov ecx,amount ; mov edst,evalue ;
/// shift edst,cl`. The 32-bit ops zero-extend into the full 64-bit register, so
/// the result matches AArch64's `rorv_w` (upper 32 bits cleared).
///
/// Same rcx-preservation as `var_shift` (see there): rcx may hold a live foreign
/// value across this expansion, and `value` may alias rcx and be overwritten by
/// the shift-count staging.
pub(super) fn var_shift_w(instruction: &CodeInstruction, digit: u8) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let value = reg(field(instruction, "lhs")?)?;
    let amount = reg(field(instruction, "rhs")?)?;
    // mov r32, r32 : 89 /r (rm = dst, reg = src), REX only for r8–r15.
    let mov32 = |d: u8, s: u8| -> Vec<u8> {
        let mut b = Vec::new();
        if d >= 8 || s >= 8 {
            b.push(rex(false, s >= 8, false, d >= 8));
        }
        b.push(0x89);
        b.push(modrm(0b11, s, d));
        b
    };
    // plan-85: a variable-shifted result may land on rcx (Win64's aligned MFB result
    // bank starts at rcx). Shift in a scratch distinct from rcx/value/amount and move
    // the result into rcx — same expansion as the 64-bit `var_shift` (see there).
    if dst == 1 {
        let scratch = free_shift_scratch(value, amount);
        let mut bytes = vec![0x50 + scratch]; // push scratch
        bytes.extend_from_slice(&mov32(scratch, value)); // mov escratch, evalue
        bytes.extend_from_slice(&mov32(1, amount)); // mov ecx, amount
        bytes.push(0xD3); // shift r/m32, CL
        bytes.push(modrm(0b11, digit, scratch));
        bytes.extend_from_slice(&mov32(1, scratch)); // mov ecx, escratch (result)
        bytes.push(0x58 + scratch); // pop scratch
        return Ok(Encoded::plain(bytes));
    }
    // `dst` is not rcx here (that case returned above), so rcx is always saved.
    let mut bytes = vec![0x51]; // push rcx (preserve foreign/residual value)
    bytes.extend_from_slice(&mov32(1, amount)); // mov ecx, amount
    if dst != value {
        if value == 1 {
            // `value` aliased rcx, now overwritten by the count; reload the
            // preserved copy. mov r32, [rsp] : 8B /r (zero-extends to 64 bits).
            if dst >= 8 {
                bytes.push(rex(false, false, false, true));
            }
            bytes.push(0x8B);
            bytes.push(modrm(0b00, dst, 4));
            bytes.push(0x24); // SIB: base=rsp
        } else {
            bytes.extend_from_slice(&mov32(dst, value)); // mov edst, evalue
        }
    }
    // D3 /digit : shift r/m32, CL — no REX.W; REX.B only for r8–r15.
    if dst >= 8 {
        bytes.push(rex(false, false, false, true));
    }
    bytes.push(0xD3);
    bytes.push(modrm(0b11, digit, dst));
    bytes.push(0x59); // pop rcx
    Ok(Encoded::plain(bytes))
}

/// Immediate shift: `mov dst,src (if needed) ; shift dst, imm8`. Field convention
/// (abi.rs): `dst`, `src`, `shift`.
pub(super) fn shift_imm(instruction: &CodeInstruction, digit: u8) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let src = reg(field(instruction, "src")?)?;
    let amount = shift(field(instruction, "shift")?)?;
    let mut bytes = if dst == src {
        Vec::new()
    } else {
        enc_mov(dst, src)
    };
    // C1 /digit ib : shift r/m64, imm8
    bytes.push(rex(true, false, false, dst >= 8));
    bytes.push(0xC1);
    bytes.push(modrm(0b11, digit, dst));
    bytes.push(amount);
    Ok(Encoded::plain(bytes))
}

/// `udiv`/`sdiv`: `mov rax,lhs ; (xor rdx,rdx | cqo) ; div/idiv rhs ; mov dst,rax`.
/// rax/rdx are non-allocatable, so clobbering them is safe.
pub(super) fn div_seq(instruction: &CodeInstruction, signed: bool) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    let ext = if signed { 7 } else { 6 }; // idiv : F7 /7 ; div : F7 /6
                                          // The instruction reserves rax (quotient / low dividend) and rdx (remainder /
                                          // high dividend), so a divisor mapped onto either would be destroyed by the
                                          // `mov rax,lhs` / `xor rdx,rdx` (or `cqo`) setup before `div` reads it. When
                                          // that aliasing happens (`map_scratch_register` freely uses rax/rcx/rdx),
                                          // stage the divisor in an 8-byte stack slot and divide from memory instead.
    let rhs_aliases = rhs == 0 || rhs == 2; // rax / rdx
                                            // `div` overwrites rax with the quotient. When the dividend register IS rax
                                            // (x0 — the raw draw in math::rand's `raw mod range`) and the quotient is asked
                                            // for elsewhere, the caller's rax value would be destroyed even though it is
                                            // still live (the following `msub` reads x0 as the minuend). Save rax across
                                            // the div and restore it so x0 survives. (For a non-rax dividend, `mov rax,lhs`
                                            // leaves the source register intact, so no preservation is needed.)
    let preserve_dividend = lhs == 0 && dst != 0;
    let mut bytes = Vec::new();
    if preserve_dividend {
        bytes.extend_from_slice(&enc_alu_imm32(5, 4, 8)); // sub rsp, 8
        bytes.push(rex(true, false, false, false)); // mov [rsp], rax
        bytes.push(0x89);
        bytes.push(modrm(0b00, 0, 4));
        bytes.push(0x24);
    }
    if rhs_aliases {
        // sub rsp,8 ; mov [rsp],rhs   — save the divisor before rax/rdx are set.
        bytes.extend_from_slice(&enc_alu_imm32(5, 4, 8)); // sub rsp, 8
                                                          // mov [rsp], rhs : REX.W 89 /r, modrm mod=00 rm=100 (SIB base=rsp).
        bytes.push(rex(true, rhs >= 8, false, false));
        bytes.push(0x89);
        bytes.push(modrm(0b00, rhs, 4));
        bytes.push(0x24); // SIB: base=rsp(100), index=none(100)
    }
    bytes.extend_from_slice(&enc_mov(0, lhs)); // mov rax, lhs
    if signed {
        bytes.push(rex(true, false, false, false));
        bytes.push(0x99); // cqo : sign-extend rax into rdx:rax
    } else {
        bytes.extend_from_slice(&alu_rr(0x31, 2, 2)); // xor rdx, rdx
    }
    if rhs_aliases {
        // div/idiv qword [rsp] : F7 /ext with a [rsp] memory operand (SIB, base=rsp)
        bytes.push(rex(true, false, false, false));
        bytes.push(0xF7);
        bytes.push(modrm(0b00, ext, 4)); // mod=00, rm=100 → SIB follows
        bytes.push(0x24); // SIB: base=rsp(100), index=none(100)
    } else {
        // div/idiv r/m64 : F7 /ext
        bytes.push(rex(true, false, false, rhs >= 8));
        bytes.push(0xF7);
        bytes.push(modrm(0b11, ext, rhs));
    }
    bytes.extend_from_slice(&enc_mov(dst, 0)); // mov dst, rax (quotient)
    if rhs_aliases {
        bytes.extend_from_slice(&enc_alu_imm32(0, 4, 8)); // add rsp, 8 (pop divisor)
    }
    if preserve_dividend {
        bytes.push(rex(true, false, false, false)); // mov rax, [rsp]
        bytes.push(0x8B);
        bytes.push(modrm(0b00, 0, 4));
        bytes.push(0x24);
        bytes.extend_from_slice(&enc_alu_imm32(0, 4, 8)); // add rsp, 8
    }
    Ok(Encoded::plain(bytes))
}

/// `setcc cl-style → carry_out (0/1)`: `setc r/m8 ; movzx carry_out, r/m8`.
/// We materialize into the destination's byte register: `xor carry_out,carry_out`
/// would clear flags, so instead use `setcc` then `movzx`. `setcc` writes only
/// the low byte; `movzx r64, r/m8` zero-extends.
fn enc_setcc_to(reg_out: u8, cc: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    // setcc r/m8 : 0F 9x /0 ; needs REX to address spl/sil/dil/bpl/r8b..
    let need_rex = reg_out >= 8 || (4..=7).contains(&reg_out);
    if need_rex {
        bytes.push(rex(false, false, false, reg_out >= 8));
    }
    bytes.push(0x0F);
    bytes.push(cc);
    bytes.push(modrm(0b11, 0, reg_out));
    // movzx r64, r/m8 : REX.W 0F B6 /r (reg := zero-extend rm8)
    bytes.push(rex(true, reg_out >= 8, false, reg_out >= 8));
    bytes.push(0x0F);
    bytes.push(0xB6);
    bytes.push(modrm(0b11, reg_out, reg_out));
    bytes
}

/// `add_carry`: `dst = lhs + rhs + carry_in`, `carry_out` = CF as 0/1.
///
/// No carry-in (carry_in is the zero token): `mov dst,lhs ; add dst,rhs`.
/// Carry-in register: set CF from carry_in (0/1) with `bt carry_in, 0` (CF = bit0
/// of carry_in, which for the {0,1} domain equals carry_in), then
/// `mov dst,lhs ; adc dst,rhs`. Finally `setc carry_out`.
///
/// `bt` is used instead of the old `add carry_in,-1`: both set CF = carry_in, but
/// `bt` does NOT modify carry_in, so if the allocator's pure-USE model of carry_in
/// is violated (its interval extends past the op, or it aliases lhs) the register
/// is not corrupted.
///
/// To keep dst/lhs/rhs distinct, we move lhs into dst first; rhs is read after.
/// If carry_in aliases dst/lhs/rhs the CF read must happen before dst is
/// overwritten — we read carry_in into CF first.
pub(super) fn enc_add_carry(instruction: &CodeInstruction) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let carry_out = reg(field(instruction, "carry_out")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    let carry_in = reg(field(instruction, "carry_in")?)?;
    // bug-397: a zero-token `lhs` is symmetric to `rhs` here because add commutes.
    // Normalize the single zero into `rhs` so the guarded zero path below handles
    // it, rather than letting `enc_mov(dst, lhs)` encode sentinel 16 as r8. Both
    // operands zero → the result is `carry_in` alone; unproduced, so reject loudly.
    let (lhs, rhs) = if is_zero_token(lhs) {
        if is_zero_token(rhs) {
            return Err(
                "x86-64 add_carry with both operands the zero token is not handled".to_string(),
            );
        }
        (rhs, lhs)
    } else {
        (lhs, rhs)
    };
    // A zero-token (`abi::ZERO` → "xzr") rhs must contribute exactly 0. `reg`
    // maps it to sentinel 16, which `alu_rr` would encode as `r8` (REX.R + reg 0),
    // folding the caller's leftover `r8` into the sum — this corrupted the PCG64
    // high state word on x86 (bug-154). Route zero-token rhs through the immediate
    // form (add nothing / `adc dst,0`) instead.
    let rhs_zero = is_zero_token(rhs);
    let mut bytes = Vec::new();
    if is_zero_token(carry_in) {
        // dst = lhs + rhs + 0
        if rhs_zero {
            // dst = lhs: a plain move (nothing to add).
            if dst != lhs {
                bytes.extend_from_slice(&enc_mov(dst, lhs));
            }
        } else if dst == rhs {
            // dst already holds rhs; add lhs in place (add commutes) so the
            // `mov dst,lhs` cannot clobber rhs before it is read (mirrors alu3).
            bytes.extend_from_slice(&alu_rr(0x01, dst, lhs));
        } else {
            if dst != lhs {
                bytes.extend_from_slice(&enc_mov(dst, lhs));
            }
            bytes.extend_from_slice(&alu_rr(0x01, dst, rhs)); // add dst, rhs
        }
    } else {
        // Set CF = carry_in (domain {0,1}) without modifying carry_in: bt carry_in, 0.
        bytes.extend_from_slice(&enc_bt_imm0(carry_in)); // CF = bit0(carry_in)
        if rhs_zero {
            // dst = lhs + 0 + CF : mov dst,lhs ; adc dst,0 (imm form: 0x81 /2).
            if dst != lhs {
                bytes.extend_from_slice(&enc_mov(dst, lhs));
            }
            bytes.extend_from_slice(&enc_alu_imm32(2, dst, 0)); // adc dst, 0
        } else if dst == rhs {
            // dst holds rhs; adc lhs in place (adc commutes over lhs/rhs).
            bytes.extend_from_slice(&alu_rr(0x11, dst, lhs));
        } else {
            if dst != lhs {
                bytes.extend_from_slice(&enc_mov(dst, lhs));
            }
            // adc dst, rhs : 0x11 /r (MR form)
            bytes.extend_from_slice(&alu_rr(0x11, dst, rhs));
        }
    }
    // A carry_out of the zero token ("discard the carry", the last-limb
    // convention) must emit no setcc at all: `enc_setcc_to(16, ...)` computes
    // `16 & 7 == 0` with REX.B and would store the carry into r8 (a SysV arg
    // register outside the allocatable pool) instead of discarding it (bug-123).
    if !is_zero_token(carry_out) {
        // setc carry_out ; movzx carry_out, carry_out  (0x92 = SETB/SETC)
        bytes.extend_from_slice(&enc_setcc_to(carry_out, 0x92));
    }
    Ok(Encoded::plain(bytes))
}

/// `sub_borrow`: `dst = lhs - rhs - borrow_in`, `borrow_out` = CF (borrow) as 0/1.
/// Field convention (abi.rs): `dst`, `borrow_out`, `lhs`, `rhs`, `borrow_in`.
///
/// No borrow-in (zero token): `mov dst,lhs ; sub dst,rhs`. Borrow-in register:
/// set CF = borrow_in via `bt borrow_in,0` (non-destructive; CF = bit0 = borrow_in
/// for the {0,1} domain), then `mov dst,lhs ; sbb dst,rhs`. On x86 a subtract sets
/// CF iff there was a borrow, so `setc borrow_out`.
pub(super) fn enc_sub_borrow(instruction: &CodeInstruction) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let borrow_out = reg(field(instruction, "borrow_out")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    let borrow_in = reg(field(instruction, "borrow_in")?)?;
    // bug-397: a zero-token `lhs` ("dst = 0 - rhs - borrow") does NOT commute like
    // add_carry's, and has no scratch-free x86 form that preserves the borrow flag
    // (zeroing a scratch via `xor` would clobber CF). Sentinel 16 would otherwise
    // reach `enc_mov` and encode as r8. Unproduced today — reject loudly rather
    // than emit wrong bytes (mirrors the dst==rhs register-borrow guard below).
    if is_zero_token(lhs) {
        return Err("x86-64 sub_borrow with a zero-token lhs is not handled".to_string());
    }
    // A zero-token rhs must subtract exactly 0 — sentinel 16 would otherwise
    // encode as `r8` (bug-154, symmetric with enc_add_carry). Latent today: no
    // shared caller passes `xzr` as sub_borrow's rhs.
    let rhs_zero = is_zero_token(rhs);
    let mut bytes = Vec::new();
    if is_zero_token(borrow_in) {
        // dst = lhs - rhs - 0
        if rhs_zero {
            // dst = lhs (nothing to subtract).
            if dst != lhs {
                bytes.extend_from_slice(&enc_mov(dst, lhs));
            }
        } else if dst == rhs {
            // dst holds rhs; `dst = lhs - rhs = -(rhs) + lhs` (sub does not
            // commute), mirroring alu3's dst==rhs guard.
            bytes.extend_from_slice(&enc_neg(dst));
            bytes.extend_from_slice(&alu_rr(0x01, dst, lhs)); // add dst, lhs
        } else {
            if dst != lhs {
                bytes.extend_from_slice(&enc_mov(dst, lhs));
            }
            bytes.extend_from_slice(&alu_rr(0x29, dst, rhs)); // sub dst, rhs
        }
    } else if rhs_zero {
        // dst = lhs - 0 - borrow : bt borrow,0 ; mov dst,lhs ; sbb dst,0.
        bytes.extend_from_slice(&enc_bt_imm0(borrow_in));
        if dst != lhs {
            bytes.extend_from_slice(&enc_mov(dst, lhs));
        }
        bytes.extend_from_slice(&enc_alu_imm32(3, dst, 0)); // sbb dst, 0
    } else if dst == rhs {
        // dst = lhs - rhs - borrow with dst aliasing rhs has no scratch-free
        // 2-op form (neg would clobber CF = borrow_in); no current caller emits
        // it. Reject rather than emit wrong bytes (alu3 rejects its gaps too).
        return Err(
            "x86-64 sub_borrow with dst aliasing rhs and a register borrow-in is not handled"
                .to_string(),
        );
    } else {
        bytes.extend_from_slice(&enc_bt_imm0(borrow_in)); // bt borrow_in,0 → CF=borrow_in (non-destructive)
        if dst != lhs {
            bytes.extend_from_slice(&enc_mov(dst, lhs));
        }
        // sbb dst, rhs : 0x19 /r (MR form)
        bytes.extend_from_slice(&alu_rr(0x19, dst, rhs));
    }
    // A borrow_out of the zero token discards the borrow: emit no setcc, else
    // the byte target `16 & 7 == 0` with REX.B would corrupt r8 (bug-123).
    if !is_zero_token(borrow_out) {
        // setc borrow_out (CF set on borrow) ; movzx
        bytes.extend_from_slice(&enc_setcc_to(borrow_out, 0x92));
    }
    Ok(Encoded::plain(bytes))
}
