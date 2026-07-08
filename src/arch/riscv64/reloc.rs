//! RISC-V 64 relocation realization (plan-99, plan-00-D §1/§3, `mir.md §8`).
//!
//! The neutral layer carries a [`RelocIntent`] — *what* a reference means, not
//! how RISC-V encodes it. This is the rv64 backend's intent→kind table, the
//! sibling of `arch::aarch64::reloc` and the x86 table. Every RISC-V PC-relative
//! reference is a **pair** of instructions (a hi20 that materializes the upper
//! 20 bits with `auipc`, and a lo12 that adds/loads the low 12 bits), so a data
//! address / GOT load splits into `*Hi`/`*Lo` kinds exactly like AArch64's
//! `page21`/`pageoff12`. A call is the single `auipc; jalr` pair the linker
//! patches as one unit from the `auipc` site (so it needs only one kind).
//!
//! The concrete kind strings are consumed by the encoder (which records the
//! relocation), by the `-ncode` serializer, and by the Linux linker
//! (`src/os/linux/link`), which patches the RISC-V instruction immediates.

use crate::target::shared::code::RelocIntent;

/// The concrete rv64 reloc kind a neutral [`RelocIntent`] realizes as.
///
/// - `Call` → `riscv_call`: the `auipc ra, hi; jalr ra, lo(ra)` pair (the `call`
///   pseudo). One reloc at the `auipc`; the linker patches both words.
/// - `DataAddrHi` → `riscv_pcrel_hi20`: the `auipc rd, %pcrel_hi(sym)`.
/// - `DataAddrLo` → `riscv_pcrel_lo12`: the `addi rd, rd, %pcrel_lo(sym)`.
/// - `GotLoadHi` → `riscv_got_hi20`: the `auipc rd, %got_pcrel_hi(sym)`.
/// - `GotLoadLo` → `riscv_got_lo12`: the `ld rd, %pcrel_lo(sym)(rd)`.
pub(crate) fn reloc_kind(intent: RelocIntent) -> &'static str {
    match intent {
        RelocIntent::Call => "riscv_call",
        RelocIntent::DataAddrHi => "riscv_pcrel_hi20",
        RelocIntent::DataAddrLo => "riscv_pcrel_lo12",
        RelocIntent::GotLoadHi => "riscv_got_hi20",
        RelocIntent::GotLoadLo => "riscv_got_lo12",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intents_map_to_riscv_kinds() {
        assert_eq!(reloc_kind(RelocIntent::Call), "riscv_call");
        assert_eq!(reloc_kind(RelocIntent::DataAddrHi), "riscv_pcrel_hi20");
        assert_eq!(reloc_kind(RelocIntent::DataAddrLo), "riscv_pcrel_lo12");
        assert_eq!(reloc_kind(RelocIntent::GotLoadHi), "riscv_got_hi20");
        assert_eq!(reloc_kind(RelocIntent::GotLoadLo), "riscv_got_lo12");
    }
}
