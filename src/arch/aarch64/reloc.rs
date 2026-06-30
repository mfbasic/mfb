//! AArch64 relocation realization (plan-00-D §1/§3, `mir.md §8`).
//!
//! The neutral layer carries a [`RelocIntent`] — *what* a reference means, not
//! how AArch64 encodes it. This is the AArch64 backend's intent→kind table: it
//! maps each neutral intent to the concrete reloc kind string the encoder and
//! the per-OS linkers (`src/os/{macos,linux}/link`) emit today
//! (`branch26`/`page21`/`pageoff12`). x86_64/rv64 supply their own table
//! (`R_X86_64_*` / `R_RISCV_*`) in their backends without touching the neutral
//! layer. The mapping is total and stays byte-identical — `Call` is always a
//! `branch26`, both `*Hi` intents are a `page21`, both `*Lo` a `pageoff12`; the
//! data-vs-GOT split (and the direct-vs-stub call split) is carried by
//! [`CodeRelocation::binding`](crate::target::shared::code::CodeRelocation), as
//! it is in the linker today.

use crate::target::shared::code::RelocIntent;

/// The concrete AArch64 reloc kind a neutral [`RelocIntent`] realizes as. Used
/// by the encoder when it materializes `bl`/`adrp`/`add :lo12:` relocations and
/// by the `-ncode` serializer (so the concrete backend dump still reads
/// `branch26`/`page21`/`pageoff12`, byte-identical to before plan-00-D).
pub(crate) fn reloc_kind(intent: RelocIntent) -> &'static str {
    match intent {
        RelocIntent::Call => "branch26",
        RelocIntent::DataAddrHi | RelocIntent::GotLoadHi => "page21",
        RelocIntent::DataAddrLo | RelocIntent::GotLoadLo => "pageoff12",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AArch64 table reproduces exactly the three concrete kinds the backend
    /// emitted before plan-00-D — the byte-identical contract.
    #[test]
    fn intents_map_to_todays_aarch64_kinds() {
        assert_eq!(reloc_kind(RelocIntent::Call), "branch26");
        assert_eq!(reloc_kind(RelocIntent::DataAddrHi), "page21");
        assert_eq!(reloc_kind(RelocIntent::DataAddrLo), "pageoff12");
        // GOT and internal-data share the page pair encoding; binding splits them.
        assert_eq!(reloc_kind(RelocIntent::GotLoadHi), "page21");
        assert_eq!(reloc_kind(RelocIntent::GotLoadLo), "pageoff12");
    }
}
