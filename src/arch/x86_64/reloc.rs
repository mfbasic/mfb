//! x86-64 relocation realization (plan-00-H; the x86 counterpart of
//! `arch::aarch64::reloc`).
//!
//! The neutral layer carries a [`RelocIntent`] — *what* a reference means, not
//! how an ISA encodes it. AArch64 splits a data address into an `adrp;add`
//! page pair (two relocations, `*Hi`/`*Lo`); x86-64 references memory
//! RIP-relative in a single instruction, so `select_x86` emits exactly **one**
//! relocation per `addr_of` / call (it uses the `*Lo` intent as the single
//! reference). This table maps each intent to the kind string the x86 linker
//! path patches:
//!
//! - `call_pc32`  → `call rel32`               (linker: `R_X86_64_PLT32`/`PC32`)
//! - `data_pc32`  → `lea reg, [rip+disp32]`     (`R_X86_64_PC32`)
//! - `got_pc32`   → `mov reg, [rip+disp32]` GOT (`R_X86_64_GOTPCREL`)
//!
//! The `*Hi` intents map to the same kinds for totality, but `select_x86` never
//! emits them (it produces a single reference, not a page pair).

use crate::target::shared::code::RelocIntent;

/// The concrete x86-64 reloc kind a neutral [`RelocIntent`] realizes as, used by
/// the x86 encoder when it materializes a `call`/`lea`/GOT-load relocation and
/// by the linker's x86 patch path.
pub(crate) fn reloc_kind(intent: RelocIntent) -> &'static str {
    match intent {
        RelocIntent::Call => "call_pc32",
        RelocIntent::DataAddrHi | RelocIntent::DataAddrLo => "data_pc32",
        RelocIntent::GotLoadHi | RelocIntent::GotLoadLo => "got_pc32",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intents_map_to_x86_kinds() {
        assert_eq!(reloc_kind(RelocIntent::Call), "call_pc32");
        // x86 references are single (RIP-relative): both halves of the neutral
        // page-pair intent collapse to the one kind select_x86 emits.
        assert_eq!(reloc_kind(RelocIntent::DataAddrLo), "data_pc32");
        assert_eq!(reloc_kind(RelocIntent::DataAddrHi), "data_pc32");
        assert_eq!(reloc_kind(RelocIntent::GotLoadLo), "got_pc32");
        assert_eq!(reloc_kind(RelocIntent::GotLoadHi), "got_pc32");
    }
}
