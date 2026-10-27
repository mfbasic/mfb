//! plan-176-O: the aarch64 encoder's the load/store, branch, call and symbol-reference
//! emitters. `Encoder`, the `emit_instruction` dispatch and `patch_labels` live in
//! `encode/emitter/mod.rs`.

use super::*;

impl Encoder {
    pub(super) fn emit_ldr_u64(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(8) {
            return Err(format!("unaligned AArch64 ldr offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xf940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xf940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    pub(super) fn emit_ldr_u32(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(4) {
            return Err(format!("unaligned AArch64 ldr u32 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 4) {
            return self.emit_word(0xb940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xb940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    pub(super) fn emit_ldr_u16(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(2) {
            return Err(format!("unaligned AArch64 ldr u16 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 2) {
            return self.emit_word(0x7940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x7940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    /// 64-bit `LDR Dt, [Xn, #offset]` — FP scalar load; offset scaled by 8.
    pub(super) fn emit_ldr_d(&mut self, dt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(8) {
            return Err(format!("unaligned AArch64 ldr d offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xfd40_0000 | (imm << 10) | ((rn as u32) << 5) | dt as u32);
        }
        let scratch = scratch_excluding(rn, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xfd40_0000 | ((scratch as u32) << 5) | dt as u32)
    }

    /// 64-bit `STR Dt, [Xn, #offset]` — FP scalar store; offset scaled by 8.
    pub(super) fn emit_str_d(&mut self, dt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(8) {
            return Err(format!("unaligned AArch64 str d offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xfd00_0000 | (imm << 10) | ((rn as u32) << 5) | dt as u32);
        }
        let scratch = scratch_excluding(rn, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xfd00_0000 | ((scratch as u32) << 5) | dt as u32)
    }

    pub(super) fn emit_str_u64(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(8) {
            return Err(format!("unaligned AArch64 str offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xf900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xf900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    pub(super) fn emit_str_u32(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(4) {
            return Err(format!("unaligned AArch64 str u32 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 4) {
            return self.emit_word(0xb900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xb900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    /// 16-bit `STRH Wt, [Xn, #offset]` — offset scaled by 2 (plan-50-D).
    ///
    /// Load/store register, unsigned immediate: `size(2) 111 V 01 opc(2) imm12 Rn Rt`.
    /// `STRH` is size=01 opc=00 → `0x7900_0000`, the store counterpart of
    /// `LDRH`'s `0x7940_0000` and the halfword sibling of `STR`/`STRB`
    /// (`0xb900_0000` / `0x3900_0000`).
    pub(super) fn emit_str_u16(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if !offset.is_multiple_of(2) {
            return Err(format!("unaligned AArch64 str u16 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 2) {
            return self.emit_word(0x7900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x7900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    pub(super) fn emit_ldr_u8(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(offset) {
            return self.emit_word(0x3940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    pub(super) fn emit_str_u8(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(offset) {
            return self.emit_word(0x3900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    /// plan-171-A: `STP`/`LDP` of two X registers (`width` 8) or two Q registers
    /// (`width` 16) — the load/store register pair class
    /// `opc(2) 101 V mode(3) L imm7 Rt2 Rn Rt`, `opc` 10 for both, `V` 1 for Q,
    /// `mode` 010 signed offset, 011 pre-index, 001 post-index. `imm7` is the
    /// byte offset over the width, so X reaches −512…504 and Q −1024…1008 in
    /// multiples of the width. Every form the architecture leaves UNPREDICTABLE is
    /// refused: an `ldp` into one register twice, and a write-back through a base
    /// the pair also names as a data register. Producers check range and
    /// alignment before building the op, so an error here is a compiler bug.
    pub(super) fn emit_pair(
        &mut self,
        instruction: &CodeInstruction,
        load: bool,
    ) -> Result<(), String> {
        let mnemonic = if load { "ldp" } else { "stp" };
        let (first, second) = if load {
            ("dst", "dst2")
        } else {
            ("src", "src2")
        };
        let base = field(instruction, "base")?;
        if matches!(base.as_ref(), "xzr" | "wzr") {
            return Err(format!(
                "AArch64 {mnemonic} base cannot be the zero register"
            ));
        }
        let rn = reg(&base)?;
        let width = immediate(field(instruction, "width")?)?;
        let (rt, rt2, simd) = match width {
            8 => {
                let gpr = |name: &str| -> Result<u8, String> {
                    let spelled = field(instruction, name)?;
                    if is_stack_pointer(&spelled) {
                        return Err(format!(
                            "AArch64 {mnemonic} {name} cannot be '{spelled}': register 31 \
                             encodes XZR there"
                        ));
                    }
                    reg(spelled)
                };
                (gpr(first)?, gpr(second)?, false)
            }
            16 => (
                vreg(field(instruction, first)?)?,
                vreg(field(instruction, second)?)?,
                true,
            ),
            other => return Err(format!("AArch64 {mnemonic} width {other} is not 8 or 16")),
        };
        let offset_text = field(instruction, "offset")?;
        let offset = offset_text
            .parse::<i64>()
            .map_err(|_| format!("invalid AArch64 {mnemonic} offset '{offset_text}'"))?;
        let width = width as i64;
        if offset % width != 0 {
            return Err(format!(
                "unaligned AArch64 {mnemonic} offset {offset} (width {width})"
            ));
        }
        let scaled = offset / width;
        if !(-64..=63).contains(&scaled) {
            return Err(format!(
                "AArch64 {mnemonic} offset {offset} exceeds the imm7 range {}..={}",
                -64 * width,
                63 * width
            ));
        }
        let mode: u32 = match field(instruction, "addressing")?.as_ref() {
            "offset" => 0b010,
            "pre" => 0b011,
            "post" => 0b001,
            other => return Err(format!("unknown AArch64 {mnemonic} addressing '{other}'")),
        };
        if load && rt == rt2 {
            return Err(format!(
                "AArch64 ldp cannot load one register twice ({})",
                field(instruction, first)?
            ));
        }
        if mode != 0b010 && !simd && rn != 31 && (rt == rn || rt2 == rn) {
            return Err(format!(
                "AArch64 {mnemonic} write-back through '{base}' cannot also name it as data"
            ));
        }
        let word = (0b10 << 30)
            | (0b101 << 27)
            | (u32::from(simd) << 26)
            | (mode << 23)
            | (u32::from(load) << 22)
            | (((scaled as u32) & 0x7f) << 15)
            | (u32::from(rt2) << 10)
            | (u32::from(rn) << 5)
            | u32::from(rt);
        self.emit_word(word)
    }

    pub(super) fn emit_label_branch(&mut self, kind: &str, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(0)?;
        self.patches.push(LabelPatch {
            offset,
            target,
            kind: kind.to_string(),
        });
        Ok(())
    }

    /// plan-171-B: `cbz`/`cbnz Xt, label` (`sf 011010 op imm19 Rt`, X form) and
    /// `tbz`/`tbnz Rt, #bit, label` (`b5 011011 op b40 imm14 Rt`). The register
    /// and bit are written now; [`Encoder::patch_labels`] ORs the displacement
    /// into the word once the label is placed (`imm19` ±1 MiB, `imm14` ±32 KiB;
    /// `relax_conditional_branches` keeps every one in reach).
    pub(super) fn emit_zero_or_bit_branch(
        &mut self,
        instruction: &CodeInstruction,
    ) -> Result<(), String> {
        let name = field(instruction, "register")?;
        if is_stack_pointer(&name) || name.starts_with('d') {
            return Err(format!(
                "AArch64 {} needs a general-purpose register, not '{name}'",
                instruction.op.mnemonic()
            ));
        }
        let rt = u32::from(reg(&name)?);
        let kind = instruction.op.mnemonic();
        let word = match instruction.op {
            CodeOp::Cbz => 0xb400_0000 | rt,
            CodeOp::Cbnz => 0xb500_0000 | rt,
            CodeOp::Tbz | CodeOp::Tbnz => {
                let bit = immediate(field(instruction, "bit")?)?;
                if bit > 63 {
                    return Err(format!("AArch64 {kind} bit {bit} is outside 0..=63"));
                }
                let bit = bit as u32;
                let op = u32::from(instruction.op == CodeOp::Tbnz) << 24;
                ((bit >> 5) << 31) | 0x3600_0000 | op | ((bit & 0x1f) << 19) | rt
            }
            other => return Err(format!("'{}' is not a cbz/tbz form", other.mnemonic())),
        };
        let offset = self.text.len();
        self.emit_word(word)?;
        self.patches.push(LabelPatch {
            offset,
            target: field(instruction, "target")?.into_owned(),
            kind: kind.to_string(),
        });
        Ok(())
    }

    pub(super) fn emit_bl(&mut self, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(0x9400_0000)?;
        // A `bl` is the neutral `Call` intent; the AArch64 table realizes it as
        // `branch26` (plan-00-D). The internal/external binding split is shared
        // across backends (bug-341-B5).
        let call_kind = crate::arch::aarch64::reloc::reloc_kind(RelocIntent::Call).to_string();
        let (binding, library) = crate::arch::image::resolve_call_binding(
            &self.symbols,
            &self.imports,
            &target,
            "AArch64",
        )?
        .into_relocation_fields();
        self.relocations.push(EncodedRelocation {
            offset,
            target,
            kind: call_kind,
            binding,
            library,
        });
        Ok(())
    }

    pub(super) fn emit_blr(&mut self, rn: u8) -> Result<(), String> {
        self.emit_word(0xd63f_0000 | ((rn as u32) << 5))
    }

    pub(super) fn emit_symbol_ref(
        &mut self,
        kind: &str,
        rd: u8,
        symbol: String,
    ) -> Result<(), String> {
        let offset = self.text.len();
        match kind {
            "adrp" => self.emit_word(0x9000_0000 | rd as u32)?,
            "add_pageoff" => self.emit_word(0x9100_0000 | ((rd as u32) << 5) | rd as u32)?,
            _ => unreachable!(),
        }
        // The `adrp; add :lo12:` page pair realizes a neutral hi/lo address
        // intent (plan-00-D): a GOT load for an imported symbol, an internal
        // data address otherwise. The AArch64 table maps both back to
        // `page21`/`pageoff12`, so the encoded relocation is byte-identical.
        let high_part = kind == "adrp";
        // The internal/external split is shared (bug-341-B5); the hi/lo intent
        // (and thus the reloc kind) is the AArch64-specific part.
        let binding = crate::arch::image::resolve_data_binding(&self.imports, &symbol);
        let intent = match (high_part, &binding) {
            (true, crate::arch::image::Binding::External(_)) => RelocIntent::GotLoadHi,
            (false, crate::arch::image::Binding::External(_)) => RelocIntent::GotLoadLo,
            (true, _) => RelocIntent::DataAddrHi,
            (false, _) => RelocIntent::DataAddrLo,
        };
        let (binding, library) = binding.into_relocation_fields();
        self.relocations.push(EncodedRelocation {
            offset,
            target: symbol,
            kind: crate::arch::aarch64::reloc::reloc_kind(intent).to_string(),
            binding,
            library,
        });
        Ok(())
    }
}
