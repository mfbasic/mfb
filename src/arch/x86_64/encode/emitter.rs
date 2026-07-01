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

use super::operand::{field, fp_reg, immediate, is_zero_token, reg, shift};
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
        // clz: count leading zeros. `lzcnt r64, r/m64` (F3 REX.W 0F BD /r) — ABM,
        // present on every x86-64-v2 CPU (Alpine target). Matches AArch64 `clz`
        // (64 for a zero input), unlike `bsr` which is undefined on zero.
        "clz" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(vec![
                0xF3,
                rex(true, dst >= 8, false, src >= 8),
                0x0F,
                0xBD,
                modrm(0b11, dst, src),
            ]))
        }
        // rev_w / rev_x: byte-reverse a 32/64-bit value (`bswap`, 0F C8+rd; the
        // 64-bit form takes REX.W). AArch64 `rev` has dst/src, x86 `bswap` is
        // in-place, so copy first when they differ.
        "rev_w" | "rev_x" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let wide = m == "rev_x";
            let mut bytes = if dst == src {
                Vec::new()
            } else if wide {
                enc_mov(dst, src)
            } else {
                // 32-bit mov (zero-extends): 89 /r, REX only for r8–r15.
                let mut b = Vec::new();
                if dst >= 8 || src >= 8 {
                    b.push(rex(false, src >= 8, false, dst >= 8));
                }
                b.push(0x89);
                b.push(modrm(0b11, src, dst));
                b
            };
            if wide || dst >= 8 {
                bytes.push(rex(wide, false, false, dst >= 8));
            }
            bytes.push(0x0F);
            bytes.push(0xC8 + (dst & 7));
            Ok(Encoded::plain(bytes))
        }
        // rbit: reverse all 64 bits. x86 has no single instruction, so use the
        // classic swap-by-strides (1,2,4 bits) then a byte reverse (`bswap`). rax
        // (mask) and rdx (temp) are free scratch — both excluded from the pool.
        "rbit" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let mut b = if dst == src { Vec::new() } else { enc_mov(dst, src) };
            // or dst, rdx : REX.W 09 /r (rm=dst, reg=rdx=2)
            let or_dst_rdx = |b: &mut Vec<u8>| {
                b.push(rex(true, false, false, dst >= 8));
                b.push(0x09);
                b.push(modrm(0b11, 2, dst));
            };
            // and reg, rax : REX.W 21 /r (rm=reg, reg=rax=0)
            let and_with_rax = |b: &mut Vec<u8>, r: u8| {
                b.push(rex(true, false, false, r >= 8));
                b.push(0x21);
                b.push(modrm(0b11, 0, r));
            };
            // mov rdx, dst : REX.W 89 /r (rm=rdx, reg=dst)
            let mov_rdx_dst = |b: &mut Vec<u8>| {
                b.push(rex(true, dst >= 8, false, false));
                b.push(0x89);
                b.push(modrm(0b11, dst, 2));
            };
            // Each step swaps adjacent bit-groups of width `shift`:
            //   x = ((x >> s) & mask) | ((x & mask) << s)
            for &(shift, mask) in &[
                (1u8, 0x5555_5555_5555_5555u64),
                (2, 0x3333_3333_3333_3333),
                (4, 0x0F0F_0F0F_0F0F_0F0F),
            ] {
                mov_rdx_dst(&mut b); // rdx = x
                b.extend(enc_mov_imm64(0, mask)); // rax = mask
                and_with_rax(&mut b, 2); // rdx = x & mask
                b.extend(enc_shift_imm_reg(4, 2, shift)); // rdx = (x & mask) << s
                b.extend(enc_shift_imm_reg(5, dst, shift)); // dst = x >> s
                and_with_rax(&mut b, dst); // dst = (x >> s) & mask
                or_dst_rdx(&mut b); // dst = ((x>>s)&mask) | ((x&mask)<<s)
            }
            // Final byte reverse: bswap dst (64-bit).
            b.push(rex(true, false, false, dst >= 8));
            b.push(0x0F);
            b.push(0xC8 + (dst & 7));
            Ok(Encoded::plain(b))
        }
        "mul" => {
            // dst = lhs * rhs (low 64). imul is commutative, so multiply into
            // whichever source already occupies dst. The naive `mov dst,lhs;
            // imul dst,rhs` corrupts the result when `dst == rhs` (the mov
            // clobbers rhs, giving lhs*lhs) — exactly what broke `index *
            // entry_size` in collection element addressing.
            let dst = reg(field(instruction, "dst")?)?;
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            let bytes = if dst == lhs {
                enc_imul_rr(dst, rhs)
            } else if dst == rhs {
                enc_imul_rr(dst, lhs)
            } else {
                let mut b = enc_mov(dst, lhs);
                b.extend_from_slice(&enc_imul_rr(dst, rhs));
                b
            };
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
        // x86-only float-compare branches (plan-00-H): `select_x86` rewrites the
        // branch after a float `ucomisd` into these (CF/ZF/PF semantics).
        "x86.jae" => jmp_label(instruction, JccKind::Jae),
        "x86.jp" => jmp_label(instruction, JccKind::Jp),
        "x86.jnp" => jmp_label(instruction, JccKind::Jnp),
        "x86.ja" => jmp_label(instruction, JccKind::Ja),
        "x86.jb" => jmp_label(instruction, JccKind::Jb),
        "x86.jbe" => jmp_label(instruction, JccKind::Jbe),
        "x86.je" => jmp_label(instruction, JccKind::Je),
        "x86.jne" => jmp_label(instruction, JccKind::Jne),
        "bl" => {
            let target = field(instruction, "target")?;
            // `mov eax, 8` (al = 8) then `E8 rel32` (call). The SysV variadic ABI
            // requires al = number of vector registers used for the variadic args
            // before calling a variadic function (snprintf with a `%f`); 8 is a
            // safe superset (the callee saves xmm0-7). Harmless for non-variadic
            // and internal calls: al is never an argument register and the return
            // value overwrites rax. rel32 disp field is at offset 6 (after the
            // 5-byte mov + the E8 opcode).
            let bytes = vec![0xB8, 8, 0, 0, 0, 0xE8, 0, 0, 0, 0];
            Ok(Encoded {
                bytes,
                side_effect: SideEffect::Reloc {
                    disp_field_offset: 6,
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

        // --- Scalar double (SSE2) — the AArch64 `dN` bank maps to `xmm` --------
        // movq xmm, r64 — reinterpret i64 bits as f64 (66 REX.W 0F 6E /r).
        "fmov_i2f" | "fmov_d_from_x" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_movq_xmm_r64(dst, src)))
        }
        // movq r64, xmm — reinterpret f64 bits as i64 (66 REX.W 0F 7E /r, MR).
        "fmov_f2i" | "fmov_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_movq_r64_xmm(dst, src)))
        }
        // movaps xmm, xmm — register copy (0F 28 /r).
        "fmov_d_from_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(None, 0x28, dst, src)))
        }
        // addsd / subsd / mulsd / divsd — dst = lhs OP rhs (2-operand SSE).
        "fadd_d" => sse_arith(instruction, 0x58, true),
        "fmul_d" => sse_arith(instruction, 0x59, true),
        "fsub_d" => sse_arith(instruction, 0x5c, false),
        "fdiv_d" => sse_arith(instruction, 0x5e, false),
        // sqrtsd dst, src (F2 0F 51 /r).
        "fsqrt_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0xf2), 0x51, dst, src)))
        }
        // ucomisd lhs, rhs — ordered compare into EFLAGS (66 0F 2E /r).
        "fcmp_d" => {
            let lhs = fp_reg(field(instruction, "lhs")?)?;
            let rhs = fp_reg(field(instruction, "rhs")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0x66), 0x2e, lhs, rhs)))
        }
        // fcmp against 0.0: zero the scratch xmm15 then ucomisd src, xmm15.
        "fcmp_zero_d" => {
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(None, 0x57, 15, 15); // xorps xmm15, xmm15
            b.extend(enc_sse_rr(Some(0x66), 0x2e, src, 15)); // ucomisd src, xmm15
            Ok(Encoded::plain(b))
        }
        // -x: flip the sign bit. Build the 0x8000…0000 mask in xmm15 (all-ones
        // via pcmpeqd, then psllq 63) and xorpd it in — no memory mask.
        "fneg_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = Vec::new();
            if dst != src {
                b.extend(enc_sse_rr(Some(0xf2), 0x10, dst, src)); // movsd dst, src
            }
            b.extend(enc_sse_rr(Some(0x66), 0x76, 15, 15)); // pcmpeqd xmm15, xmm15
            b.extend(enc_psxlq(0x06, 15, 63)); // psllq xmm15, 63 -> sign mask
            b.extend(enc_sse_rr(Some(0x66), 0x57, dst, 15)); // xorpd dst, xmm15
            Ok(Encoded::plain(b))
        }
        // |x|: clear the sign bit by shifting the 64-bit lane left 1 then right 1
        // (psllq/psrlq imm), avoiding a memory-resident mask constant.
        "fabs_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = Vec::new();
            if dst != src {
                b.extend(enc_sse_rr(Some(0xf2), 0x10, dst, src)); // movsd dst, src
            }
            b.extend(enc_psxlq(0x06, dst, 1)); // psllq dst, 1
            b.extend(enc_psxlq(0x02, dst, 1)); // psrlq dst, 1
            Ok(Encoded::plain(b))
        }
        // cvtsi2sd xmm, r64 — signed i64 → f64 (F2 REX.W 0F 2A /r).
        "i2f" | "scvtf_d_from_x" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_cvt(0xf2, 0x2a, true, dst, src)))
        }
        // cvttsd2si r64, xmm — f64 → i64 toward zero (F2 REX.W 0F 2C /r).
        "f2i_trunc" | "fcvtzs_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_cvt(0xf2, 0x2c, false, dst, src)))
        }
        // f64 → i64 with a directed rounding mode: `roundsd xmm15, src, mode`
        // (SSE4.1, mode 1=−∞ floor / 2=+∞ ceil) then truncating `cvttsd2si`.
        "f2i_floor" | "fcvtms_x_from_d" | "f2i_ceil" | "fcvtps_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mode = if m.starts_with("f2i_floor") || m.starts_with("fcvtms") {
                1
            } else {
                2
            };
            let mut b = enc_roundsd(15, src, mode); // roundsd xmm15, src, mode
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, dst, 15)); // cvttsd2si dst, xmm15
            Ok(Encoded::plain(b))
        }
        // f64 → i64 round-to-nearest, ties AWAY from zero (AArch64 `fcvtas`).
        // SSE `roundsd`/`cvtsd2si` round ties to EVEN, so realize the ties-away
        // rule directly: result = trunc(src + copysign(0.5, src)). The sign of src
        // is OR-ed into 0.5's bit pattern in the (scratch) dst GPR — rax is free
        // (never allocated: excluded from the scratch pool) so it stages the 0.5
        // constant; xmm15 is the FP scratch.
        "f2i_nearest" | "fcvtas_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_movq_r64_xmm(dst, src); // dst = raw bits of src
            b.extend(enc_shift_imm_reg(5, dst, 63)); // shr dst, 63  → sign bit
            b.extend(enc_shift_imm_reg(4, dst, 63)); // shl dst, 63  → sign << 63
            b.extend(enc_mov_imm64(0, 0x3FE0_0000_0000_0000)); // movabs rax, bits(0.5)
            // or dst, rax : REX.W 09 /r (rm = dst, reg = rax)
            b.push(rex(true, false, false, dst >= 8));
            b.push(0x09);
            b.push(modrm(0b11, 0, dst));
            b.extend(enc_movq_xmm_r64(15, dst)); // xmm15 = copysign(0.5, src)
            b.extend(enc_sse_rr(Some(0xf2), 0x58, 15, src)); // addsd xmm15, src
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, dst, 15)); // cvttsd2si dst, xmm15
            Ok(Encoded::plain(b))
        }
        // 32-bit variable rotate-right (`rorv_w`/`rotr_w`): ror r32, cl.
        "rorv_w" | "rotr_w" => var_shift_w(instruction, 1),
        // movsd xmm, [base+disp] / movsd [base+disp], xmm (F2 0F 10 / 11 /r).
        "ldr_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?.parse::<i32>().map_err(|_| {
                "x86 ldr_d: bad offset".to_string()
            })?;
            Ok(Encoded::plain(enc_movsd_mem(0x10, dst, base, disp)))
        }
        "str_d" => {
            let src = fp_reg(field(instruction, "src")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?.parse::<i32>().map_err(|_| {
                "x86 str_d: bad offset".to_string()
            })?;
            Ok(Encoded::plain(enc_movsd_mem(0x11, src, base, disp)))
        }

        // --- v128 SIMD (SSE2/SSE4.1) — plan-00-H Phase 3 --------------------
        // 128-bit unaligned load/store (movups, 0F 10 / 11). `v`/`q` map to xmm.
        "ldr_q" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?
                .parse::<i32>()
                .map_err(|_| "x86 ldr_q: bad offset".to_string())?;
            Ok(Encoded::plain(enc_movups_mem(0x10, dst, base, disp)))
        }
        "str_q" => {
            let src = fp_reg(field(instruction, "src")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?
                .parse::<i32>()
                .map_err(|_| "x86 str_q: bad offset".to_string())?;
            Ok(Encoded::plain(enc_movups_mem(0x11, src, base, disp)))
        }
        // Packed f64×2 three-operand arithmetic (66 0F op); commutativity noted.
        "fadd_v" => vec3_op(instruction, 0x58, true),
        "fmul_v" => vec3_op(instruction, 0x59, true),
        "fsub_v" => vec3_op(instruction, 0x5C, false),
        "fdiv_v" => vec3_op(instruction, 0x5E, false),
        "fmin_v" => vec3_op(instruction, 0x5D, false),
        "fmax_v" => vec3_op(instruction, 0x5F, false),
        // Packed integer i64×2 add/sub (paddq/psubq).
        "add_v" => vec3_op(instruction, 0xD4, true),
        "sub_v" => vec3_op(instruction, 0xFB, false),
        // Bitwise 128-bit (pand/por/pxor) — all commutative.
        "and_v" => vec3_op(instruction, 0xDB, true),
        "orr_v" => vec3_op(instruction, 0xEB, true),
        "eor_v" => vec3_op(instruction, 0xEF, true),
        // Packed f64×2 unary sqrt (sqrtpd, 66 0F 51).
        "fsqrt_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0x66), 0x51, dst, src)))
        }
        // fneg/fabs: xor/and each lane with the sign mask, built in xmm15 from
        // an all-ones vector shifted (psllq 63 = 0x8000…; psrlq 1 = 0x7FFF…).
        "fneg_v" | "fabs_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let neg = m == "fneg_v";
            let mut b = enc_sse_rr(Some(0x66), 0x76, 15, 15); // pcmpeqd xmm15,xmm15
            b.extend(enc_psxlq(if neg { 0x06 } else { 0x02 }, 15, if neg { 63 } else { 1 }));
            if dst != src {
                b.extend(enc_movaps(dst, src));
            }
            // xorpd (66 0F 57) / andpd (66 0F 54) dst, xmm15
            b.extend(enc_sse_rr(Some(0x66), if neg { 0x57 } else { 0x54 }, dst, 15));
            Ok(Encoded::plain(b))
        }
        // Integer i64 negate: 0 - src (staged through xmm15 so dst may alias src).
        "neg_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15
            b.extend(enc_sse_rr(Some(0x66), 0xFB, 15, src)); // psubq xmm15, src
            b.extend(enc_movaps(dst, 15));
            Ok(Encoded::plain(b))
        }
        // Integer i64 absolute: mask = (0 > x) ? -1 : 0; |x| = (x ^ mask) - mask.
        "abs_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15
            b.extend(enc_sse38_rr(0x37, 15, src)); // pcmpgtq xmm15, src  (0 > src)
            if dst != src {
                b.extend(enc_movaps(dst, src));
            }
            b.extend(enc_sse_rr(Some(0x66), 0xEF, dst, 15)); // pxor dst, mask
            b.extend(enc_sse_rr(Some(0x66), 0xFB, dst, 15)); // psubq dst, mask
            Ok(Encoded::plain(b))
        }
        // Signed integer i64 lane compares → all-ones/all-zeros mask.
        "cmgt_v" => {
            // pcmpgtq dst(=lhs), rhs  (SSE4.2, 66 0F 38 37).
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let b = if dst == rhs && dst != lhs {
                let mut b = enc_movaps(15, rhs);
                b.extend(enc_movaps(dst, lhs));
                b.extend(enc_sse38_rr(0x37, dst, 15));
                b
            } else {
                let mut b = if dst == lhs { Vec::new() } else { enc_movaps(dst, lhs) };
                b.extend(enc_sse38_rr(0x37, dst, rhs));
                b
            };
            Ok(Encoded::plain(b))
        }
        "cmeq_v" => {
            // pcmpeqq (SSE4.1, 66 0F 38 29), commutative.
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(vec_sse38_commutative(0x29, dst, lhs, rhs)))
        }
        "cmge_v" => {
            // a>=b = ~(b>a): pcmpgtq(rhs,lhs) then NOT via pxor all-ones.
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = enc_movaps(15, lhs); // xmm15 = lhs (a)
            if dst != rhs {
                b.extend(enc_movaps(dst, rhs)); // dst = rhs (b)
            }
            b.extend(enc_sse38_rr(0x37, dst, 15)); // dst = b > a
            b.extend(enc_sse_rr(Some(0x66), 0x76, 15, 15)); // xmm15 = all ones
            b.extend(enc_sse_rr(Some(0x66), 0xEF, dst, 15)); // dst = ~(b>a) = a>=b
            Ok(Encoded::plain(b))
        }
        // Packed f64×2 compares (cmppd) → NaN-correct lane masks. fcmgt(a,b)=b<a
        // (LT, false on NaN); fcmge=b<=a (LE); fcmeq=a==b (EQ).
        "fcmgt_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(vec_cmppd_swapped(dst, lhs, rhs, 1)))
        }
        "fcmge_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(vec_cmppd_swapped(dst, lhs, rhs, 2)))
        }
        "fcmeq_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = if dst == rhs { enc_movaps(15, rhs) } else { Vec::new() };
            let r = if dst == rhs { 15 } else { rhs };
            if dst != lhs {
                b.extend(enc_movaps(dst, lhs));
            }
            b.extend(enc_cmppd(dst, r, 0));
            Ok(Encoded::plain(b))
        }
        // Packed f64×2 compares against zero. xmm15 = 0.0×2.
        "fcmgt_zero_v" | "fcmge_zero_v" | "fcmlt_zero_v" | "fcmle_zero_v" | "fcmeq_zero_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15 (0.0)
            match m {
                "fcmgt_zero_v" => {
                    // src>0 = 0<src : dst=0; cmpltpd dst,src
                    b.extend(enc_movaps(dst, 15));
                    b.extend(enc_cmppd(dst, src, 1));
                }
                "fcmge_zero_v" => {
                    // src>=0 = 0<=src : dst=0; cmplepd dst,src
                    b.extend(enc_movaps(dst, 15));
                    b.extend(enc_cmppd(dst, src, 2));
                }
                "fcmlt_zero_v" => {
                    // src<0 : dst=src; cmpltpd dst,0
                    if dst != src {
                        b.extend(enc_movaps(dst, src));
                    }
                    b.extend(enc_cmppd(dst, 15, 1));
                }
                "fcmle_zero_v" => {
                    // src<=0 : dst=src; cmplepd dst,0
                    if dst != src {
                        b.extend(enc_movaps(dst, src));
                    }
                    b.extend(enc_cmppd(dst, 15, 2));
                }
                _ => {
                    // src==0 : dst=src; cmpeqpd dst,0
                    if dst != src {
                        b.extend(enc_movaps(dst, src));
                    }
                    b.extend(enc_cmppd(dst, 15, 0));
                }
            }
            Ok(Encoded::plain(b))
        }
        // Directed rounding (roundpd, SSE4.1 66 0F 3A 09 /r ib).
        "frintp_v" | "frintm_v" | "frintz_v" | "frintn_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mode = match m {
                "frintn_v" => 0, // nearest-even
                "frintm_v" => 1, // floor
                "frintp_v" => 2, // ceil
                _ => 3,          // trunc
            };
            Ok(Encoded::plain(enc_roundpd(dst, src, mode)))
        }
        // Immediate lane shifts (i64): psllq /6, psrlq /2 (66 0F 73).
        "shl_v" | "ushr_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let shift: u8 = field(instruction, "shift")?
                .parse()
                .map_err(|_| "x86 shl_v/ushr_v: bad shift".to_string())?;
            let mut b = if dst == src { Vec::new() } else { enc_movaps(dst, src) };
            b.extend(enc_psxlq(if m == "shl_v" { 0x06 } else { 0x02 }, dst, shift));
            Ok(Encoded::plain(b))
        }
        // Broadcast a GPR into both i64 lanes: movq xmm,r64 ; punpcklqdq xmm,xmm.
        "dup_v_from_x" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let mut b = enc_movq_xmm_r64(dst, src);
            b.extend(enc_sse_rr(Some(0x66), 0x6C, dst, dst)); // punpcklqdq dst,dst
            Ok(Encoded::plain(b))
        }
        // Extract a lane to a GPR. Lane 0 = movq; lane 1 = pextrq (SSE4.1).
        "umov_x_from_v" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let index: u8 = field(instruction, "index")?
                .parse()
                .map_err(|_| "x86 umov_x_from_v: bad index".to_string())?;
            if index == 0 {
                Ok(Encoded::plain(enc_movq_r64_xmm(dst, src)))
            } else {
                // pextrq r64, xmm, imm : 66 REX.W 0F 3A 16 /r ib
                Ok(Encoded::plain(vec![
                    0x66,
                    rex(true, src >= 8, false, dst >= 8),
                    0x0F,
                    0x3A,
                    0x16,
                    modrm(0b11, src, dst),
                    index,
                ]))
            }
        }
        // Bit-select (BSL): dst = (dst & lhs) | (~dst & rhs), dst is the mask.
        "bsl_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = enc_movaps(15, dst); // xmm15 = mask
            b.extend(enc_sse_rr(Some(0x66), 0xDB, dst, lhs)); // dst = mask & lhs
            b.extend(enc_sse_rr(Some(0x66), 0xDF, 15, rhs)); // xmm15 = ~mask & rhs (pandn)
            b.extend(enc_sse_rr(Some(0x66), 0xEB, dst, 15)); // dst |= xmm15
            Ok(Encoded::plain(b))
        }
        // Bit-insert-if-true (BIT): dst = (dst & ~rhs) | (lhs & rhs), mask in rhs.
        "bit_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = enc_movaps(15, lhs); // xmm15 = lhs
            b.extend(enc_sse_rr(Some(0x66), 0xDB, 15, rhs)); // xmm15 = lhs & rhs
            b.extend(enc_movaps(14, rhs)); // xmm14 = rhs
            b.extend(enc_sse_rr(Some(0x66), 0xDF, 14, dst)); // xmm14 = ~rhs & dst
            b.extend(enc_movaps(dst, 14));
            b.extend(enc_sse_rr(Some(0x66), 0xEB, dst, 15)); // dst |= lhs&rhs
            Ok(Encoded::plain(b))
        }
        // Fused multiply-add/-subtract (single rounding, matches AArch64 fmla/fmls)
        // via FMA3 (x86-64-v3). fmla_v: dst += lhs*rhs → vfmadd231pd (B8); fmls_v:
        // dst -= lhs*rhs → vfnmadd231pd (BC). 231 form: reg=dst, vvvv=lhs, rm=rhs.
        "fmla_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(enc_vfma231pd(0xB8, dst, lhs, rhs)))
        }
        "fmls_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(enc_vfma231pd(0xBC, dst, lhs, rhs)))
        }
        // Packed f64↔i64×2 conversions have no SSE2 form (AVX-512 only), so do
        // them lane-serial through rax/rdx (both free — excluded from the pool)
        // and xmm15. Lane 1 is brought to lane 0 with `pshufd imm=0xEE`.
        "fcvtzs_v" => {
            // dst[i] = trunc(src[i])  (cvttsd2si per lane)
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_cvt(0xf2, 0x2c, false, 0, src); // cvttsd2si rax, src.lo
            b.extend(enc_pshufd(15, src, 0xEE)); // xmm15.lo = src.hi
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, 2, 15)); // cvttsd2si rdx, src.hi
            b.extend(enc_movq_xmm_r64(dst, 0)); // dst.lo = rax
            b.extend(enc_movq_xmm_r64(15, 2)); // xmm15.lo = rdx
            b.extend(enc_sse_rr(Some(0x66), 0x6C, dst, 15)); // punpcklqdq dst,xmm15 → [rax,rdx]
            Ok(Encoded::plain(b))
        }
        "scvtf_v" => {
            // dst[i] = (f64) src[i]  (cvtsi2sd per lane)
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_movq_r64_xmm(0, src); // rax = src.lo
            b.extend(enc_pshufd(15, src, 0xEE)); // xmm15.lo = src.hi
            b.extend(enc_movq_r64_xmm(2, 15)); // rdx = src.hi
            b.extend(enc_sse_cvt(0xf2, 0x2a, true, dst, 0)); // cvtsi2sd dst, rax
            b.extend(enc_sse_cvt(0xf2, 0x2a, true, 15, 2)); // cvtsi2sd xmm15, rdx
            b.extend(enc_sse_rr(Some(0x66), 0x14, dst, 15)); // unpcklpd dst,xmm15 → [lo,hi]
            Ok(Encoded::plain(b))
        }
        // Arithmetic i64 lane shift-right by imm. SSE2 has no `psraq` (only 32-bit
        // `psrad`), so emulate: logical `psrlq` then OR in the sign fill for the
        // top `k` bits — sign_mask = (0 > src) per lane, shifted left by 64−k.
        "sshr_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let k: u8 = field(instruction, "shift")?
                .parse()
                .map_err(|_| "x86 sshr_v: bad shift".to_string())?;
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15
            b.extend(enc_sse38_rr(0x37, 15, src)); // pcmpgtq xmm15,src → src<0 ? -1 : 0
            if k > 0 && k < 64 {
                b.extend(enc_psxlq(0x06, 15, 64 - k)); // psllq xmm15, 64-k (top k bits)
            } else {
                // k==0 is a no-op shift; clear the sign fill.
                b.extend(enc_sse_rr(Some(0x66), 0xEF, 15, 15));
            }
            if dst != src {
                b.extend(enc_movaps(dst, src));
            }
            b.extend(enc_psxlq(0x02, dst, k)); // psrlq dst, k (logical)
            b.extend(enc_sse_rr(Some(0x66), 0xEB, dst, 15)); // por dst, sign fill
            Ok(Encoded::plain(b))
        }
        // Still unsupported: fcvtas_v (nearest ties-away), frinta_v, sshl_v, ushl_v.
        other => Err(format!("x86 encode: unsupported op {other}")),
    }
}

/// Three-operand ALU (`dst = lhs OP rhs`): `mov dst,lhs` (if needed) then the
/// register-form ALU `dst OP= rhs`. `opcode` is the `MR` reg-form opcode.
fn alu3(instruction: &CodeInstruction, opcode: u8) -> Result<Encoded, String> {
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
                let mut b = if dst == rhs { Vec::new() } else { enc_mov(dst, rhs) };
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

/// SSE reg-reg: `[prefix] [REX] 0F opcode modrm(11,reg,rm)` on xmm indices.
/// REX.R extends `reg_x`, REX.B extends `rm_x` (needed for xmm8-15); a REX byte
/// is only emitted when one of them is high.
fn enc_sse_rr(prefix: Option<u8>, opcode: u8, reg_x: u8, rm_x: u8) -> Vec<u8> {
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
fn enc_movq_xmm_r64(dst_x: u8, src_r: u8) -> Vec<u8> {
    vec![
        0x66,
        rex(true, dst_x >= 8, false, src_r >= 8),
        0x0f,
        0x6e,
        modrm(0b11, dst_x, src_r),
    ]
}

/// `movq r64, xmm` — 66 REX.W 0F 7E /r, MR form (reg = xmm src, rm = r64 dst).
fn enc_movq_r64_xmm(dst_r: u8, src_x: u8) -> Vec<u8> {
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
fn enc_roundsd(dst_x: u8, src_x: u8, mode: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if dst_x >= 8 || src_x >= 8 {
        b.push(rex(false, dst_x >= 8, false, src_x >= 8));
    }
    b.extend_from_slice(&[0x0F, 0x3A, 0x0B, modrm(0b11, dst_x, src_x), mode]);
    b
}

/// `shift r64, imm8` — REX.W C1 /digit ib (digit: 4=SHL, 5=SHR, 7=SAR).
fn enc_shift_imm_reg(digit: u8, reg_n: u8, imm: u8) -> Vec<u8> {
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
fn enc_sse_cvt(prefix: u8, opcode: u8, _dst_is_xmm: bool, dst: u8, src: u8) -> Vec<u8> {
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
fn enc_psxlq(ext: u8, xmm: u8, imm: u8) -> Vec<u8> {
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
fn enc_movsd_mem(opcode: u8, xmm: u8, base: u8, disp: i32) -> Vec<u8> {
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
fn enc_movups_mem(opcode: u8, xmm: u8, base: u8, disp: i32) -> Vec<u8> {
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
fn enc_movaps(dst: u8, src: u8) -> Vec<u8> {
    let mut b = Vec::new();
    if dst >= 8 || src >= 8 {
        b.push(rex(false, dst >= 8, false, src >= 8));
    }
    b.extend_from_slice(&[0x0f, 0x28, modrm(0b11, dst, src)]);
    b
}

/// Three-byte SSE (66 0F 38 opcode /r) — pcmpgtq (0x37), pcmpeqq (0x29).
fn enc_sse38_rr(opcode: u8, reg_x: u8, rm_x: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if reg_x >= 8 || rm_x >= 8 {
        b.push(rex(false, reg_x >= 8, false, rm_x >= 8));
    }
    b.extend_from_slice(&[0x0f, 0x38, opcode, modrm(0b11, reg_x, rm_x)]);
    b
}

/// `cmppd dst, src, pred` (66 0F C2 /r ib) — packed f64 compare to a lane mask.
fn enc_cmppd(dst: u8, src: u8, pred: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if dst >= 8 || src >= 8 {
        b.push(rex(false, dst >= 8, false, src >= 8));
    }
    b.extend_from_slice(&[0x0f, 0xc2, modrm(0b11, dst, src), pred]);
    b
}

/// `pshufd dst, src, imm` (66 0F 70 /r ib) — shuffle 32-bit lanes. `imm=0xEE`
/// copies the high 64 bits (dwords 2,3) into the low 64 (a "high lane → lane 0").
fn enc_pshufd(dst: u8, src: u8, imm: u8) -> Vec<u8> {
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
fn enc_vfma231pd(opcode: u8, dst: u8, lhs: u8, rhs: u8) -> Vec<u8> {
    // P0: [~R][~X=1][~B][mmmmm=00010 (0F38)]; ~R extends reg(dst), ~B extends rm(rhs).
    let r_bar = if dst >= 8 { 0 } else { 1 };
    let b_bar = if rhs >= 8 { 0 } else { 1 };
    let p0 = (r_bar << 7) | (1 << 6) | (b_bar << 5) | 0b00010;
    // P1: [W=1][vvvv=~lhs][L=0][pp=01 (66)].
    let vvvv = (!(lhs as u32)) & 0xF;
    let p1 = (1u8 << 7) | ((vvvv as u8) << 3) | 0b01;
    vec![0xC4, p0, p1, opcode, modrm(0b11, dst, rhs)]
}

/// `roundpd dst, src, mode` (SSE4.1 66 0F 3A 09 /r ib).
fn enc_roundpd(dst: u8, src: u8, mode: u8) -> Vec<u8> {
    let mut b = vec![0x66];
    if dst >= 8 || src >= 8 {
        b.push(rex(false, dst >= 8, false, src >= 8));
    }
    b.extend_from_slice(&[0x0f, 0x3a, 0x09, modrm(0b11, dst, src), mode]);
    b
}

/// Parse the `dst`/`lhs`/`rhs` fields of a three-operand SIMD op as xmm indices.
fn three_fp(instruction: &CodeInstruction) -> Result<(u8, u8, u8), String> {
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

fn vec3_op(instruction: &CodeInstruction, opcode: u8, commutative: bool) -> Result<Encoded, String> {
    let (dst, lhs, rhs) = three_fp(instruction)?;
    Ok(Encoded::plain(vec3(0x66, opcode, commutative, dst, lhs, rhs)))
}

/// Commutative three-byte SSE op (pcmpeqq) with in-place dst handling.
fn vec_sse38_commutative(opcode: u8, dst: u8, lhs: u8, rhs: u8) -> Vec<u8> {
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
fn vec_cmppd_swapped(dst: u8, lhs: u8, rhs: u8, pred: u8) -> Vec<u8> {
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
fn sse_arith(
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
    Jae,
    Jp,
    Jnp,
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

/// 32-bit variable shift/rotate by CL: `mov ecx,amount ; mov edst,evalue ;
/// shift edst,cl`. The 32-bit ops zero-extend into the full 64-bit register, so
/// the result matches AArch64's `rorv_w` (upper 32 bits cleared). rcx is
/// non-allocatable, so clobbering it is safe.
fn var_shift_w(instruction: &CodeInstruction, digit: u8) -> Result<Encoded, String> {
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
    let mut bytes = mov32(1, amount); // mov ecx, amount
    if dst != value {
        bytes.extend_from_slice(&mov32(dst, value)); // mov edst, evalue
    }
    // D3 /digit : shift r/m32, CL — no REX.W; REX.B only for r8–r15.
    if dst >= 8 {
        bytes.push(rex(false, false, false, true));
    }
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
    let ext = if signed { 7 } else { 6 }; // idiv : F7 /7 ; div : F7 /6
    // The instruction reserves rax (quotient / low dividend) and rdx (remainder /
    // high dividend), so a divisor mapped onto either would be destroyed by the
    // `mov rax,lhs` / `xor rdx,rdx` (or `cqo`) setup before `div` reads it. When
    // that aliasing happens (`map_scratch_register` freely uses rax/rcx/rdx),
    // stage the divisor in an 8-byte stack slot and divide from memory instead.
    let rhs_aliases = rhs == 0 || rhs == 2; // rax / rdx
    let mut bytes = Vec::new();
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
    bytes.extend_from_slice(&enc_mov(dst, 0)); // mov dst, rax
    if rhs_aliases {
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
