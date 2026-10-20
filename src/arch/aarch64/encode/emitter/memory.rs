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
