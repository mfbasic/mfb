//! x86-64 instruction emission.
//!
//! ## Size/emit consistency
//!
//! The two-pass framework requires `instruction_size` to equal the number of
//! bytes `emit_instruction` produces, byte-for-byte — a mismatch shifts every
//! later label and corrupts branch displacements. To make that impossible to get
//! wrong, both paths funnel through one function, [`encode_instruction`], which
//! returns the exact machine bytes plus any relocation/label *intent*.
//! `instruction_size` returns `bytes.len()`; `emit_instruction` appends the bytes
//! and records the relocation/patch. There is a single source of truth.
//!
//! Every encoding here is fixed-size and distance-independent (rel32, imm32/imm64,
//! disp32), so a size never depends on a label that hasn't been placed yet.

use super::operand::{field, immediate, is_zero_token, reg, shift};
use super::*;
use crate::target::shared::code::RelocIntent;

pub(super) struct Encoder {
    pub(super) text: Vec<u8>,
    pub(super) data: Vec<u8>,
    pub(super) symbols: Vec<EncodedSymbol>,
    pub(super) relocations: Vec<EncodedRelocation>,
    pub(super) imports: HashMap<String, String>,
    pub(super) labels: HashMap<String, usize>,
    pub(super) patches: Vec<LabelPatch>,
}

pub(super) struct LabelPatch {
    /// Byte offset of the rel32 displacement field within `text`.
    disp_offset: usize,
    target: String,
}

/// What an instruction's bytes additionally produce: a relocation against a
/// symbol, or an intra-function branch displacement to patch.
enum SideEffect {
    None,
    /// A relocation whose `offset` is `bytes_start + disp_field_offset`.
    Reloc {
        /// Offset of the disp32 field from the start of this instruction's bytes.
        disp_field_offset: usize,
        target: String,
        intent: RelocIntent,
    },
    /// An intra-function branch: patch the rel32 at `bytes_start +
    /// disp_field_offset` to reach `target`'s label.
    LabelBranch {
        disp_field_offset: usize,
        target: String,
    },
}

pub(super) struct Encoded {
    bytes: Vec<u8>,
    side_effect: SideEffect,
}

impl Encoded {
    fn plain(bytes: Vec<u8>) -> Self {
        Encoded {
            bytes,
            side_effect: SideEffect::None,
        }
    }

    /// Byte length of this instruction — the single source of truth `sizing`
    /// reports.
    pub(super) fn bytes_len(&self) -> usize {
        self.bytes.len()
    }

    /// Consume into the raw machine bytes (test helper).
    #[cfg(test)]
    pub(super) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Encoder {
    pub(super) fn emit_instruction(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        let start = self.text.len();
        let encoded = encode_instruction(instruction)?;
        self.text.extend_from_slice(&encoded.bytes);
        match encoded.side_effect {
            SideEffect::None => {}
            SideEffect::Reloc {
                disp_field_offset,
                target,
                intent,
            } => {
                self.record_reloc(start + disp_field_offset, target, intent)?;
            }
            SideEffect::LabelBranch {
                disp_field_offset,
                target,
            } => {
                self.patches.push(LabelPatch {
                    disp_offset: start + disp_field_offset,
                    target,
                });
            }
        }
        Ok(())
    }

    /// Materialize a symbol relocation: an internal call, an import-stub call, or
    /// a data/GOT address load. Binding distinguishes them for the linker, exactly
    /// as the AArch64 encoder does.
    fn record_reloc(
        &mut self,
        offset: usize,
        target: String,
        intent: RelocIntent,
    ) -> Result<(), String> {
        let kind = crate::arch::x86_64::reloc::reloc_kind(intent).to_string();
        match intent {
            RelocIntent::Call => {
                if self.symbols.iter().any(|symbol| symbol.name == target) {
                    self.relocations.push(EncodedRelocation {
                        offset,
                        target,
                        kind,
                        binding: "internal".to_string(),
                        library: None,
                    });
                } else if let Some(library) = self.imports.get(&target) {
                    self.relocations.push(EncodedRelocation {
                        offset,
                        target,
                        kind,
                        binding: "external".to_string(),
                        library: Some(library.clone()),
                    });
                } else {
                    return Err(format!(
                        "x86-64 branch target symbol '{target}' does not resolve"
                    ));
                }
            }
            // A data address: an imported symbol routes through the GOT
            // (`got_pc32`), an internal symbol is referenced directly
            // (`data_pc32`). `record_reloc` is only called with the *Lo intent
            // (select_x86 emits a single RIP-relative reference).
            _ => {
                if let Some(library) = self.imports.get(&target) {
                    // Re-derive as a GOT load so the kind string matches.
                    let kind = crate::arch::x86_64::reloc::reloc_kind(RelocIntent::GotLoadLo)
                        .to_string();
                    self.relocations.push(EncodedRelocation {
                        offset,
                        target,
                        kind,
                        binding: "external".to_string(),
                        library: Some(library.clone()),
                    });
                } else {
                    self.relocations.push(EncodedRelocation {
                        offset,
                        target,
                        kind,
                        binding: "data".to_string(),
                        library: None,
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn patch_labels(&mut self) -> Result<(), String> {
        for patch in &self.patches {
            let Some(&target) = self.labels.get(&patch.target) else {
                return Err(format!(
                    "x86-64 branch target label '{}' does not resolve",
                    patch.target
                ));
            };
            // rel32 is relative to the address of the *next* instruction, i.e. the
            // 4-byte disp field end.
            let next = patch.disp_offset + 4;
            let delta = target as isize - next as isize;
            let disp = delta as i32 as u32;
            self.text[patch.disp_offset..patch.disp_offset + 4]
                .copy_from_slice(&disp.to_le_bytes());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Encoding primitives
// ---------------------------------------------------------------------------

/// REX prefix byte. `w` selects a 64-bit operand; `r`/`x`/`b` extend the ModRM
/// reg / SIB index / ModRM rm-or-base fields to registers 8..15.
fn rex(w: bool, r: bool, x: bool, b: bool) -> u8 {
    0x40 | ((w as u8) << 3) | ((r as u8) << 2) | ((x as u8) << 1) | (b as u8)
}

/// ModRM byte.
fn modrm(md: u8, reg: u8, rm: u8) -> u8 {
    (md << 6) | ((reg & 7) << 3) | (rm & 7)
}

/// SIB byte.
fn sib(scale: u8, index: u8, base: u8) -> u8 {
    (scale << 6) | ((index & 7) << 3) | (base & 7)
}

/// Encode `op reg64, reg64` for a register-to-register ALU instruction whose
/// opcode uses the standard direction (`reg` field = source, `rm` field =
/// destination — the `/r` form with the 0x01/0x09/… opcodes). We always emit the
/// `MR` form (opcode operates rm := rm OP reg), so `reg` is the source and `rm`
/// is the destination.
fn alu_rr(opcode: u8, dst: u8, src: u8) -> Vec<u8> {
    // REX.R extends `src` (reg field), REX.B extends `dst` (rm field).
    vec![
        rex(true, src >= 8, false, dst >= 8),
        opcode,
        modrm(0b11, src, dst),
    ]
}

/// Emit a memory operand `[base + disp32]` with a fixed 4-byte displacement
/// (mod=10). `reg` is the ModRM reg field (the register operand). Handles the
/// rsp/r12 SIB requirement. REX is supplied by the caller via `reg`/`base` bits.
/// Returns the ModRM(+SIB)+disp32 tail (no opcode, no REX).
fn mem_disp32(reg: u8, base: u8, disp: i32) -> Vec<u8> {
    let mut out = Vec::new();
    let base_low = base & 7;
    if base_low == 4 {
        // rsp/r12 as base requires a SIB byte with index=none(rsp).
        out.push(modrm(0b10, reg, 4));
        out.push(sib(0, 4, base));
    } else {
        out.push(modrm(0b10, reg, base));
    }
    out.extend_from_slice(&disp.to_le_bytes());
    out
}

/// A RIP-relative memory operand `[rip + disp32]`: mod=00, rm=101. The disp32 is
/// a placeholder (0) the linker patches via the relocation.
fn mem_rip(reg: u8) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(modrm(0b00, reg, 0b101));
    out.extend_from_slice(&0i32.to_le_bytes());
    out
}

fn checked_disp32(offset: u64) -> Result<i32, String> {
    i32::try_from(offset).map_err(|_| format!("x86-64 memory offset {offset} exceeds disp32"))
}

fn checked_imm32(value: u64) -> Result<i32, String> {
    // Accept values that fit a sign-extended 32-bit immediate (covers small
    // positive immediates and -1-style masks reached via wraparound).
    if let Ok(v) = i32::try_from(value) {
        return Ok(v);
    }
    if let Ok(v) = i32::try_from(value as i64) {
        return Ok(v);
    }
    Err(format!("x86-64 immediate {value} exceeds imm32"))
}

// ---------------------------------------------------------------------------
// Per-instruction encoding (single source of truth for size + bytes)
// ---------------------------------------------------------------------------

pub(super) fn encode_instruction(instruction: &CodeInstruction) -> Result<Encoded, String> {
    let m = instruction.op.mnemonic();
    match m {
        "label" => Ok(Encoded::plain(Vec::new())),
        "mov" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_mov(dst, src)))
        }
        "mov_imm" => {
            let dst = reg(field(instruction, "dst")?)?;
            let value = immediate(field(instruction, "value")?)?;
            Ok(Encoded::plain(enc_mov_imm64(dst, value)))
        }
        // x86 add/sub always set EFLAGS, so the flag-setting `adds`/`subs`
        // variants are the same encoding (the following flag-branch reads them).
        "add" | "adds" => alu3(instruction, 0x01),
        "sub" | "subs" => alu3(instruction, 0x29),
        "and" => alu3(instruction, 0x21),
        "orr" => alu3(instruction, 0x09),
        "eor" => alu3(instruction, 0x31),
        "mvn" => {
            // dst = ~src : mov dst,src ; not dst
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let mut bytes = enc_mov(dst, src);
            // F7 /2 = NOT r/m64
            bytes.push(rex(true, false, false, dst >= 8));
            bytes.push(0xF7);
            bytes.push(modrm(0b11, 2, dst));
            Ok(Encoded::plain(bytes))
        }
        "mul" => {
            // dst = lhs * rhs (low 64): mov dst,lhs ; imul dst,rhs
            let dst = reg(field(instruction, "dst")?)?;
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            let mut bytes = enc_mov(dst, lhs);
            bytes.extend_from_slice(&enc_imul_rr(dst, rhs));
            Ok(Encoded::plain(bytes))
        }
        "umulh" | "smulh" => {
            // rdx:rax = lhs * rhs ; dst = rdx.  Clobbers rax/rdx (non-allocatable).
            // F7 /4 = MUL (unsigned high), F7 /5 = IMUL (signed high).
            let dst = reg(field(instruction, "dst")?)?;
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            let ext = if instruction.op == crate::arch::aarch64::ops::CodeOp::SMulH {
                5
            } else {
                4
            };
            let mut bytes = enc_mov(0, lhs); // mov rax, lhs
            bytes.push(rex(true, false, false, rhs >= 8));
            bytes.push(0xF7);
            bytes.push(modrm(0b11, ext, rhs));
            bytes.extend_from_slice(&enc_mov(dst, 2)); // mov dst, rdx
            Ok(Encoded::plain(bytes))
        }
        "udiv" => div_seq(instruction, false),
        "sdiv" => div_seq(instruction, true),
        "msub" => {
            // dst = minuend - lhs*rhs.  imul rax,lhs,rhs is awkward (two-operand
            // needs rax=lhs); use: mov rax,lhs ; imul rax,rhs ; mov dst,minuend ;
            // sub dst,rax.  rax non-allocatable.
            let dst = reg(field(instruction, "dst")?)?;
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            let minuend = reg(field(instruction, "minuend")?)?;
            let mut bytes = enc_mov(0, lhs); // mov rax, lhs
            bytes.extend_from_slice(&enc_imul_rr(0, rhs)); // imul rax, rhs
            bytes.extend_from_slice(&enc_mov(dst, minuend)); // mov dst, minuend
            bytes.extend_from_slice(&alu_rr(0x29, dst, 0)); // sub dst, rax
            Ok(Encoded::plain(bytes))
        }
        "add_carry" => enc_add_carry(instruction),
        "sub_borrow" => enc_sub_borrow(instruction),
        "rorv" => var_shift(instruction, 1), // /1 = ROR
        "lslv" => var_shift(instruction, 4), // /4 = SHL
        "lsrv" => var_shift(instruction, 5), // /5 = SHR
        "asrv" => var_shift(instruction, 7), // /7 = SAR
        "lsl_imm" => shift_imm(instruction, 4),
        "lsr_imm" => shift_imm(instruction, 5),
        "asr_imm" => shift_imm(instruction, 7),
        "add_imm" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            // mov dst,src (if different) ; add dst, imm32
            let mut bytes = if dst == src { Vec::new() } else { enc_mov(dst, src) };
            bytes.extend_from_slice(&enc_alu_imm32(0, dst, imm)); // /0 = ADD
            Ok(Encoded::plain(bytes))
        }
        "sub_imm" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            let mut bytes = if dst == src { Vec::new() } else { enc_mov(dst, src) };
            bytes.extend_from_slice(&enc_alu_imm32(5, dst, imm)); // /5 = SUB
            Ok(Encoded::plain(bytes))
        }
        "add_sp" => {
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            Ok(Encoded::plain(enc_alu_imm32(0, 4, imm))) // add rsp, imm32
        }
        "sub_sp" => {
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            Ok(Encoded::plain(enc_alu_imm32(5, 4, imm))) // sub rsp, imm32
        }
        "cmp" => {
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            // cmp lhs, rhs : 0x39 /r (rm=lhs, reg=rhs)
            Ok(Encoded::plain(alu_rr(0x39, lhs, rhs)))
        }
        "cmp_imm" => {
            let lhs = reg(field(instruction, "lhs")?)?;
            let imm = checked_imm32(immediate(field(instruction, "rhs")?)?)?;
            Ok(Encoded::plain(enc_alu_imm32(7, lhs, imm))) // /7 = CMP
        }
        "ldr_u64" => mem_load(instruction, MemWidth::U64),
        "ldr_u32" => mem_load(instruction, MemWidth::U32),
        "ldr_u16" => mem_load(instruction, MemWidth::U16),
        "ldr_u8" => mem_load(instruction, MemWidth::U8),
        "str_u64" => mem_store(instruction, MemWidth::U64),
        "str_u32" => mem_store(instruction, MemWidth::U32),
        "str_u8" => mem_store(instruction, MemWidth::U8),
        "b" => jmp_label(instruction, JccKind::Jmp),
        "b.eq" => jmp_label(instruction, JccKind::Je),
        "b.ne" => jmp_label(instruction, JccKind::Jne),
        "b.ge" => jmp_label(instruction, JccKind::Jge),
        "b.lt" => jmp_label(instruction, JccKind::Jl),
        "b.gt" => jmp_label(instruction, JccKind::Jg),
        "b.le" => jmp_label(instruction, JccKind::Jle),
        "b.hi" => jmp_label(instruction, JccKind::Ja),
        "b.lo" => jmp_label(instruction, JccKind::Jb),
        // Overflow / sign / unsigned-LE — the *_ovf checks and the IEEE float
        // flag-branches (plan-00-B/16/17). V→OF, N→SF, "C clear or Z" → BE.
        "b.vs" => jmp_label(instruction, JccKind::Jo),
        "b.vc" => jmp_label(instruction, JccKind::Jno),
        "b.mi" => jmp_label(instruction, JccKind::Js),
        "b.ls" => jmp_label(instruction, JccKind::Jbe),
        "bl" => {
            let target = field(instruction, "target")?;
            // E8 rel32 ; relocation against the call target at disp offset 1.
            let bytes = vec![0xE8, 0, 0, 0, 0];
            Ok(Encoded {
                bytes,
                side_effect: SideEffect::Reloc {
                    disp_field_offset: 1,
                    target,
                    intent: RelocIntent::Call,
                },
            })
        }
        "blr" => {
            // call r/m64 : FF /2
            let r = reg(field(instruction, "register")?)?;
            let mut bytes = Vec::new();
            if r >= 8 {
                bytes.push(rex(false, false, false, true));
            }
            bytes.push(0xFF);
            bytes.push(modrm(0b11, 2, r));
            Ok(Encoded::plain(bytes))
        }
        "branch_self" => Ok(Encoded::plain(vec![0xEB, 0xFE])), // jmp $ (rel8 -2)
        "ret" => Ok(Encoded::plain(vec![0xC3])),
        "svc" => Ok(Encoded::plain(vec![0x0F, 0x05])), // syscall
        "adrp" => {
            // lea dst, [rip+disp32] ; disp32 patched by a data/GOT relocation.
            let dst = reg(field(instruction, "dst")?)?;
            let symbol = field(instruction, "symbol")?;
            // REX.W + 0x8D /r, ModRM rip-relative. disp32 starts after REX(1)+
            // opcode(1)+modrm(1) = 3 bytes in.
            let mut bytes = vec![rex(true, dst >= 8, false, false), 0x8D];
            bytes.extend_from_slice(&mem_rip(dst));
            Ok(Encoded {
                bytes,
                side_effect: SideEffect::Reloc {
                    disp_field_offset: 3,
                    target: symbol,
                    // select_x86 collapses the page pair: the single reference is
                    // the *Lo intent. record_reloc re-routes to GOT for imports.
                    intent: RelocIntent::DataAddrLo,
                },
            })
        }
        // The low half of the AArch64 page pair: x86 already loaded the full
        // address into `dst` via the `adrp`-spelled lea, so this emits nothing.
        "add_pageoff" => Ok(Encoded::plain(Vec::new())),
        // Float / v128 are Phase 2/3.
        other => Err(format!("x86 encode: unsupported op {other}")),
    }
}

/// Three-operand ALU (`dst = lhs OP rhs`): `mov dst,lhs` (if needed) then the
/// register-form ALU `dst OP= rhs`. `opcode` is the `MR` reg-form opcode.
fn alu3(instruction: &CodeInstruction, opcode: u8) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    let mut bytes = if dst == lhs { Vec::new() } else { enc_mov(dst, lhs) };
    bytes.extend_from_slice(&alu_rr(opcode, dst, rhs));
    Ok(Encoded::plain(bytes))
}

/// `mov dst, src` — REX.W 0x89 /r (MR form: rm := reg).
fn enc_mov(dst: u8, src: u8) -> Vec<u8> {
    vec![
        rex(true, src >= 8, false, dst >= 8),
        0x89,
        modrm(0b11, src, dst),
    ]
}

/// `mov r64, imm64` — REX.W + (0xB8 + rd) + 8-byte immediate. Always 10 bytes,
/// so the size is trivial and value-independent.
fn enc_mov_imm64(dst: u8, value: u64) -> Vec<u8> {
    let mut bytes = vec![rex(true, false, false, dst >= 8), 0xB8 + (dst & 7)];
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

/// `imul dst, src` — two-operand signed multiply, REX.W 0x0F 0xAF /r (RM form:
/// reg := reg * rm), so `dst` is the reg field and `src` the rm field.
fn enc_imul_rr(dst: u8, src: u8) -> Vec<u8> {
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
fn enc_alu_imm32(digit: u8, rm: u8, imm: i32) -> Vec<u8> {
    let mut bytes = vec![rex(true, false, false, rm >= 8), 0x81, modrm(0b11, digit, rm)];
    bytes.extend_from_slice(&imm.to_le_bytes());
    bytes
}

#[derive(Clone, Copy)]
enum MemWidth {
    U64,
    U32,
    U16,
    U8,
}

fn mem_load(instruction: &CodeInstruction, width: MemWidth) -> Result<Encoded, String> {
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

fn mem_store(instruction: &CodeInstruction, width: MemWidth) -> Result<Encoded, String> {
    let src = reg(field(instruction, "src")?)?;
    let base = reg(field(instruction, "base")?)?;
    let disp = checked_disp32(immediate(field(instruction, "offset")?)?)?;
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
            return Err("x86 encode: str_u16 unsupported".to_string());
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
enum JccKind {
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
}

fn jmp_label(instruction: &CodeInstruction, kind: JccKind) -> Result<Encoded, String> {
    let target = field(instruction, "target")?;
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
/// `digit` is the group-2 /digit (1=ROR, 4=SHL, 5=SHR, 7=SAR). rcx is
/// non-allocatable, so clobbering it is safe. Field convention (abi.rs): `dst`,
/// `lhs` = value, `rhs` = amount.
fn var_shift(instruction: &CodeInstruction, digit: u8) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let value = reg(field(instruction, "lhs")?)?;
    let amount = reg(field(instruction, "rhs")?)?;
    let mut bytes = enc_mov(1, amount); // mov rcx, amount
    if dst != value {
        bytes.extend_from_slice(&enc_mov(dst, value)); // mov dst, value
    }
    // D3 /digit : shift r/m64, CL
    bytes.push(rex(true, false, false, dst >= 8));
    bytes.push(0xD3);
    bytes.push(modrm(0b11, digit, dst));
    Ok(Encoded::plain(bytes))
}

/// Immediate shift: `mov dst,src (if needed) ; shift dst, imm8`. Field convention
/// (abi.rs): `dst`, `src`, `shift`.
fn shift_imm(instruction: &CodeInstruction, digit: u8) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let src = reg(field(instruction, "src")?)?;
    let amount = shift(field(instruction, "shift")?)?;
    let mut bytes = if dst == src { Vec::new() } else { enc_mov(dst, src) };
    // C1 /digit ib : shift r/m64, imm8
    bytes.push(rex(true, false, false, dst >= 8));
    bytes.push(0xC1);
    bytes.push(modrm(0b11, digit, dst));
    bytes.push(amount);
    Ok(Encoded::plain(bytes))
}

/// `udiv`/`sdiv`: `mov rax,lhs ; (xor rdx,rdx | cqo) ; div/idiv rhs ; mov dst,rax`.
/// rax/rdx are non-allocatable, so clobbering them is safe.
fn div_seq(instruction: &CodeInstruction, signed: bool) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    let mut bytes = enc_mov(0, lhs); // mov rax, lhs
    if signed {
        // cqo : REX.W 0x99 — sign-extend rax into rdx:rax.
        bytes.push(rex(true, false, false, false));
        bytes.push(0x99);
        // idiv r/m64 : F7 /7
        bytes.push(rex(true, false, false, rhs >= 8));
        bytes.push(0xF7);
        bytes.push(modrm(0b11, 7, rhs));
    } else {
        // xor rdx, rdx : 0x31 /r (rm=rdx, reg=rdx)
        bytes.extend_from_slice(&alu_rr(0x31, 2, 2));
        // div r/m64 : F7 /6
        bytes.push(rex(true, false, false, rhs >= 8));
        bytes.push(0xF7);
        bytes.push(modrm(0b11, 6, rhs));
    }
    bytes.extend_from_slice(&enc_mov(dst, 0)); // mov dst, rax
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
/// Carry-in register: set CF from carry_in (0/1) with `add carry_in, -1` (sets CF
/// iff carry_in != 0 for the {0,1} domain... actually `add 0xFF..FF` to 0 leaves
/// CF=0, to 1 leaves CF=1), then `mov dst,lhs ; adc dst,rhs`. Finally
/// `setc carry_out`.
///
/// To keep dst/lhs/rhs distinct, we move lhs into dst first; rhs is read after.
/// If carry_in aliases dst/lhs/rhs the `add carry_in,-1` must happen before dst
/// is overwritten — we read carry_in into CF first.
fn enc_add_carry(instruction: &CodeInstruction) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let carry_out = reg(field(instruction, "carry_out")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    let carry_in = reg(field(instruction, "carry_in")?)?;
    let mut bytes = Vec::new();
    if is_zero_token(carry_in) {
        // mov dst,lhs ; add dst,rhs
        if dst != lhs {
            bytes.extend_from_slice(&enc_mov(dst, lhs));
        }
        bytes.extend_from_slice(&alu_rr(0x01, dst, rhs)); // add dst, rhs
    } else {
        // Set CF = carry_in (domain {0,1}): add carry_in, -1.
        bytes.extend_from_slice(&enc_alu_imm32(0, carry_in, -1)); // add carry_in, -1
        if dst != lhs {
            bytes.extend_from_slice(&enc_mov(dst, lhs));
        }
        // adc dst, rhs : 0x11 /r (MR form)
        bytes.extend_from_slice(&alu_rr(0x11, dst, rhs));
    }
    // setc carry_out ; movzx carry_out, carry_out  (0x92 = SETB/SETC)
    bytes.extend_from_slice(&enc_setcc_to(carry_out, 0x92));
    Ok(Encoded::plain(bytes))
}

/// `sub_borrow`: `dst = lhs - rhs - borrow_in`, `borrow_out` = CF (borrow) as 0/1.
/// Field convention (abi.rs): `dst`, `borrow_out`, `lhs`, `rhs`, `borrow_in`.
///
/// No borrow-in (zero token): `mov dst,lhs ; sub dst,rhs`. Borrow-in register:
/// set CF = borrow_in via `add borrow_in,-1`, then `mov dst,lhs ; sbb dst,rhs`.
/// On x86 a subtract sets CF iff there was a borrow, so `setc borrow_out`.
fn enc_sub_borrow(instruction: &CodeInstruction) -> Result<Encoded, String> {
    let dst = reg(field(instruction, "dst")?)?;
    let borrow_out = reg(field(instruction, "borrow_out")?)?;
    let lhs = reg(field(instruction, "lhs")?)?;
    let rhs = reg(field(instruction, "rhs")?)?;
    let borrow_in = reg(field(instruction, "borrow_in")?)?;
    let mut bytes = Vec::new();
    if is_zero_token(borrow_in) {
        if dst != lhs {
            bytes.extend_from_slice(&enc_mov(dst, lhs));
        }
        bytes.extend_from_slice(&alu_rr(0x29, dst, rhs)); // sub dst, rhs
    } else {
        bytes.extend_from_slice(&enc_alu_imm32(0, borrow_in, -1)); // add borrow_in,-1 → CF=borrow_in
        if dst != lhs {
            bytes.extend_from_slice(&enc_mov(dst, lhs));
        }
        // sbb dst, rhs : 0x19 /r (MR form)
        bytes.extend_from_slice(&alu_rr(0x19, dst, rhs));
    }
    // setc borrow_out (CF set on borrow) ; movzx
    bytes.extend_from_slice(&enc_setcc_to(borrow_out, 0x92));
    Ok(Encoded::plain(bytes))
}
