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
use crate::codegen::engine::types::RelocIntent;

mod instruction;
mod integer;
mod sse;
pub(super) use instruction::encode_instruction;
use integer::*;
use sse::*;

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
                // For an imported data symbol the adrp-spelled `lea dst,[rip+disp32]`
                // (opcode 0x8D, which computes the *address of* the GOT slot) must
                // become `mov dst,[rip+disp32]` (REX.W 0x8B, which *loads* the GOT
                // entry = the symbol address) so the reference is dereferenced
                // through the GOT exactly once, matching aarch64/riscv64 and the
                // `got_pc32` reloc kind (bug-192). The two forms share REX.W + the
                // rip-relative ModRM, differing only in this opcode byte; non-import
                // data addresses stay `lea`.
                if matches!(
                    intent,
                    RelocIntent::DataAddrHi
                        | RelocIntent::DataAddrLo
                        | RelocIntent::GotLoadHi
                        | RelocIntent::GotLoadLo
                ) && self.imports.contains_key(&target)
                    && self.text.get(start + 1) == Some(&0x8D)
                {
                    self.text[start + 1] = 0x8B;
                }
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
        // The internal/external/data binding split is shared across backends
        // (bug-341-B5); the reloc `kind` is the x86-specific part.
        match intent {
            RelocIntent::Call => {
                let (binding, library) = crate::arch::image::resolve_call_binding(
                    &self.symbols,
                    &self.imports,
                    &target,
                    "x86-64",
                )?
                .into_relocation_fields();
                self.relocations.push(EncodedRelocation {
                    offset,
                    target,
                    kind,
                    binding,
                    library,
                });
            }
            // A data address: an imported symbol routes through the GOT
            // (`got_pc32`), an internal symbol is referenced directly
            // (`data_pc32`). `record_reloc` is only called with the *Lo intent
            // (select_x86 emits a single RIP-relative reference).
            _ => {
                let binding = crate::arch::image::resolve_data_binding(&self.imports, &target);
                // Re-derive an import as a GOT load so the kind string matches.
                let kind = match &binding {
                    crate::arch::image::Binding::External(_) => {
                        crate::arch::x86_64::reloc::reloc_kind(RelocIntent::GotLoadLo).to_string()
                    }
                    _ => kind,
                };
                let (binding, library) = binding.into_relocation_fields();
                self.relocations.push(EncodedRelocation {
                    offset,
                    target,
                    kind,
                    binding,
                    library,
                });
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

/// `push r64` / `pop r64` — 0x50+rd / 0x58+rd, with REX.B for r8-r15. The 64-bit
/// operand size is implicit, so no REX.W.
fn enc_push_reg(reg_n: u8) -> Vec<u8> {
    let mut b = Vec::new();
    if reg_n >= 8 {
        b.push(rex(false, false, false, true));
    }
    b.push(0x50 + (reg_n & 7));
    b
}

fn enc_pop_reg(reg_n: u8) -> Vec<u8> {
    let mut b = Vec::new();
    if reg_n >= 8 {
        b.push(rex(false, false, false, true));
    }
    b.push(0x58 + (reg_n & 7));
    b
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

impl crate::arch::encode_plan::InstructionEncoder for Encoder {
    fn new(data: Vec<u8>, imports: HashMap<String, String>) -> Self {
        Encoder {
            text: Vec::new(),
            data,
            symbols: Vec::new(),
            relocations: Vec::new(),
            imports,
            labels: HashMap::new(),
            patches: Vec::new(),
        }
    }

    fn instruction_size(instruction: &CodeInstruction) -> Result<usize, String> {
        super::sizing::instruction_size(instruction)
    }

    fn label_name(instruction: &CodeInstruction) -> Result<String, String> {
        super::operand::field(instruction, "name").map(|c| c.into_owned())
    }

    fn emit_one(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        self.emit_instruction(instruction)
    }

    fn resolve_patches(&mut self) -> Result<(), String> {
        self.patch_labels()
    }

    fn text_len(&self) -> usize {
        self.text.len()
    }

    fn reserve_text(&mut self, len: usize) {
        self.text.resize(len, 0);
    }

    fn truncate_text(&mut self, len: usize) {
        self.text.truncate(len);
    }

    fn push_symbol(&mut self, symbol: EncodedSymbol) {
        self.symbols.push(symbol);
    }

    fn insert_label(&mut self, name: String, offset: usize) -> Option<usize> {
        self.labels.insert(name, offset)
    }

    fn clear_labels(&mut self) {
        self.labels.clear();
    }

    fn clear_patches(&mut self) {
        self.patches.clear();
    }

    fn into_parts(self) -> (Vec<u8>, Vec<u8>, Vec<EncodedSymbol>, Vec<EncodedRelocation>) {
        (self.text, self.data, self.symbols, self.relocations)
    }
}
