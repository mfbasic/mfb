//! A typed operand value (plan-78-A).
//!
//! `CodeInstruction` operand *values* are stored as `String` today; every
//! consumer disambiguates a register / immediate / label / symbol by field-name
//! role plus a prefix sniff plus a numeric parse, with no tag. That string
//! re-classification is the register allocator's measured hot cost (the
//! `str::eq`/SipHash self-time in `regalloc::analysis`). [`Operand`] introduces a
//! tag so the hot path can eventually read register identity without parsing.
//!
//! This sub-plan (A) lands the type and proves it renders back to the *exact*
//! current operand strings — storage still stays `String` (`CodeInstruction::
//! field` stores `operand.render()`), so nothing about the emitted bytes changes.
//! plan-78-B flips the stored representation to `Operand`; plan-78-C reads it on
//! the allocator hot path.
//!
//! ## Arm set
//!
//! Only the kinds the hot path benefits from are lifted to typed form:
//!
//! - [`Operand::VReg`] — a virtual-register sentinel (`%vN` integer / `%fN`
//!   floating-point), the allocator's interning key.
//! - [`Operand::Imm`] — a decimal integer immediate.
//! - [`Operand::Raw`] — the long tail, rendered verbatim: physical register
//!   names, labels, symbols, type names, stack-offset sentinels, booleans, and
//!   the `%scratch`/`%sysnr`/`%local` occupancy tokens.
//!
//! **No `Phys { class, index }` arm yet, on purpose.** A physical register's
//! spelling is *not* recoverable from `(class, index)` alone: `x0` / `rax` /
//! `zero` all sit at integer index 0 on their respective ISAs, and `d3` / `v3`
//! alias the same floating-point index 3 *within* AArch64. So a `Phys` arm that
//! rendered from an index could not reproduce the exact source string, which is
//! the one property A must guarantee. Physical registers therefore stay `Raw`
//! (their name is the render source of truth) through A and B; plan-78-C, which
//! writes physical names at the allocation rewrite site where the name *and*
//! class+index are both in hand, is where a `Phys { class, index, name }` arm is
//! introduced and read on the hot path.
//!
//! ## Render fidelity
//!
//! Byte-identity of every `.ncode`/`.mir` dump and every encoded instruction is
//! decided purely by these value strings, so `render()` reproducing the source
//! string exactly is what makes A a no-op. [`Operand::parse`] only assigns a
//! typed arm when that arm round-trips (`Imm` requires the decimal to be
//! canonical; a `%v`/`%f` sentinel with a non-numeric tail falls through to
//! `Raw`), so `parse(s).render() == s` holds for every string — proven by the
//! corpus test below.

use crate::target::shared::regmodel::RegClass;

use super::regalloc::{fp_vreg_name, parse_fp_vreg, parse_vreg, vreg_name};

/// A typed operand value. See the module docs for the arm set and why physical
/// registers stay `Raw` until plan-78-C.
///
/// The `VReg` arm carries a targeted `#[allow(dead_code)]`: plan-78-B stores
/// register operands as `Raw` (the vreg→`VReg` write migration is coupled to
/// plan-78-C's typed reads and lands there), so until C constructs it the arm is
/// exercised only by the render-fidelity corpus test in this module. That test is
/// load-bearing *now* — it proves flipping storage to `Operand` (B) and reading
/// it on the allocator hot path (C) is byte-safe. `Imm` is constructed in
/// production (the `finalize_frame` offset rewrite, plan-78-B), so it needs no
/// allow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Operand {
    /// A virtual-register sentinel: `%vN` for the integer class, `%fN` for the
    /// floating-point class. `id` is the register number the allocator interns.
    #[allow(dead_code)] // constructed by plan-78-C's typed reads; proven live by the corpus test
    VReg { class: RegClass, id: u32 },
    /// A decimal integer immediate.
    Imm(i64),
    /// Any operand string not lifted to a typed arm — rendered verbatim.
    Raw(Box<str>),
}

impl Operand {
    /// A virtual register of `class` numbered `id`.
    #[allow(dead_code)] // proven by the corpus test; production writers land in plan-78-B/C
    pub(crate) fn vreg(class: RegClass, id: u32) -> Self {
        Operand::VReg { class, id }
    }

    /// A decimal integer immediate.
    pub(crate) fn imm(value: i64) -> Self {
        Operand::Imm(value)
    }

    /// Reproduce the exact operand string the codegen emits for this value.
    /// Virtual registers render through the same `vreg_name`/`fp_vreg_name` the
    /// allocator uses; immediates render as decimal; `Raw` is verbatim.
    pub(crate) fn render(&self) -> String {
        match self {
            Operand::VReg {
                class: RegClass::Int,
                id,
            } => vreg_name(*id),
            Operand::VReg {
                class: RegClass::Fp,
                id,
            } => fp_vreg_name(*id),
            Operand::Imm(value) => value.to_string(),
            Operand::Raw(text) => text.to_string(),
        }
    }

    /// Classify an operand string back into a typed `Operand`, mirroring the
    /// sniff order the encoder / allocator use (vreg prefix → decimal immediate →
    /// else `Raw`). A typed arm is chosen only when it renders back to the exact
    /// input, so the classification is always faithful: a `%v`/`%f` prefix with a
    /// non-numeric tail, or a non-canonical decimal (`007`, `+5`, or a `u64` past
    /// `i64::MAX`), falls through to `Raw`.
    ///
    /// Used by the round-trip corpus test in A; once producers migrate (B) they
    /// build the typed arm directly and this is a convenience for the read side.
    #[allow(dead_code)] // proven by the corpus test; the read-side consumer lands in plan-78-B/C
    pub(crate) fn parse(value: &str) -> Self {
        if let Some(id) = parse_vreg(value) {
            return Operand::vreg(RegClass::Int, id);
        }
        if let Some(id) = parse_fp_vreg(value) {
            return Operand::vreg(RegClass::Fp, id);
        }
        if let Ok(number) = value.parse::<i64>() {
            // Only classify as an immediate when the decimal is canonical, so
            // render reproduces the source string byte-for-byte.
            if number.to_string() == value {
                return Operand::imm(number);
            }
        }
        Operand::Raw(value.into())
    }
}

/// Render an operand to its string form. Lets the many string-shaped codegen
/// readers (arch token realization, `format!` diagnostics) keep spelling
/// `format!("{value}")` / `value.to_string()` through the plan-78-B flip; the
/// value they see is exactly the string that used to be stored. plan-78-C
/// replaces the string comparisons on the hot path with typed `Operand` matches.
impl std::fmt::Display for Operand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

/// Compare an operand to a string by its rendered spelling, so a reader that used
/// to test `value == "x30"` against the stored `String` keeps working verbatim
/// after the flip. (`&Operand == &str` follows from the std blanket
/// `impl PartialEq<&B> for &A where A: PartialEq<B>`.)
impl PartialEq<str> for Operand {
    fn eq(&self, other: &str) -> bool {
        self.render() == other
    }
}

impl PartialEq<&str> for Operand {
    fn eq(&self, other: &&str) -> bool {
        self.render() == *other
    }
}

/// An unmigrated `&str` operand argument becomes a verbatim `Raw` (so every
/// existing `.field(name, value)` call compiles unchanged and renders the same
/// string). Producers that know the kind pass a typed `Operand` instead.
impl From<&str> for Operand {
    fn from(value: &str) -> Self {
        Operand::Raw(value.into())
    }
}

/// `.field("imm", &imm.to_string())` and similar pass a `&String`; keep them
/// compiling verbatim as `Raw`. (`&String` does not coerce to `&str` under a
/// generic `impl Into<Operand>` bound the way it did for the old `&str` param.)
impl From<&String> for Operand {
    fn from(value: &String) -> Self {
        Operand::Raw(value.as_str().into())
    }
}

/// An owned `String` operand argument becomes a verbatim `Raw`.
impl From<String> for Operand {
    fn from(value: String) -> Self {
        Operand::Raw(value.into_boxed_str())
    }
}

/// The `ci(op, &[("dst", "x0"), …])` table-builder helpers (arch `select`/
/// encoder tests, and the production riscv64 `select`/`v128` builders) iterate a
/// `&[(&'static str, &str)]` slice, so each operand value arrives as `&&str`.
/// Keep those `.field(name, value)` calls compiling verbatim as `Raw`, the same
/// way the old `&str` parameter accepted them through deref coercion.
impl From<&&str> for Operand {
    fn from(value: &&str) -> Self {
        Operand::Raw((*value).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operand kinds `plan-78-A §2` enumerates. Every one must appear in the
    /// corpus so a missing kind fails the coverage assertion rather than passing
    /// vacuously.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
    enum Kind {
        VReg,
        Phys,
        Imm,
        Bool,
        Label,
        Symbol,
        TypeName,
        StackSentinel,
    }

    /// A round-trip corpus of real operand strings. The physical-register,
    /// immediate, boolean, symbol, type-name, and label rows were harvested from
    /// the `-ncode` dumps of `scripts/bench-probes/{trivial,one-regex}` on
    /// 2026-08-02 (`-ncode` is post-allocation, so it carries no `%v`/`%f`
    /// sentinels); the virtual-register rows use the real `vreg_name`/
    /// `fp_vreg_name` spellings, and the stack-offset sentinels are the
    /// pre-`finalize_frame` tokens `finalize_frame` rewrites. Every string must
    /// satisfy `parse(s).render() == s`, and together they must cover every
    /// `Kind`.
    fn corpus() -> Vec<(String, Kind)> {
        let mut rows: Vec<(String, Kind)> = vec![
            // Physical registers across all three ISAs — AArch64 `x*`/`sp`/`d*`/
            // `v*`, x86-64 `rax`/`xmm*`, riscv64 `zero`/`ft*`. All stay `Raw` in
            // A (their name is the render source of truth).
            ("x0".into(), Kind::Phys),
            ("x28".into(), Kind::Phys),
            ("sp".into(), Kind::Phys),
            ("d3".into(), Kind::Phys),
            ("d15".into(), Kind::Phys),
            ("v5".into(), Kind::Phys),
            ("rax".into(), Kind::Phys),
            ("xmm2".into(), Kind::Phys),
            ("zero".into(), Kind::Phys),
            ("ft0".into(), Kind::Phys),
            // Decimal immediates, including boundary values.
            ("0".into(), Kind::Imm),
            ("1".into(), Kind::Imm),
            ("42".into(), Kind::Imm),
            ("65535".into(), Kind::Imm),
            ("1000000".into(), Kind::Imm),
            (i64::MAX.to_string(), Kind::Imm),
            // Booleans — the immediate encoder maps these to 1/0, but the operand
            // string is the word, so they render verbatim as `Raw`.
            ("true".into(), Kind::Bool),
            ("false".into(), Kind::Bool),
            // Labels (the `name` field of a `label` op).
            (
                "_mfb_rt_io_io_print_buf_line_fits".into(),
                Kind::Label,
            ),
            // Symbols — a runtime helper and a libc import.
            ("_mfb_arena_alloc".into(), Kind::Symbol),
            ("_exit".into(), Kind::Symbol),
            // Type names (the `type` field).
            ("Integer".into(), Kind::TypeName),
            ("Boolean".into(), Kind::TypeName),
            ("UnionTag".into(), Kind::TypeName),
            // Stack-offset sentinels (pre-`finalize_frame`).
            ("incoming_args".into(), Kind::StackSentinel),
            ("outgoing_args".into(), Kind::StackSentinel),
        ];
        // Virtual registers, using the real render functions so the corpus tracks
        // the actual spelling (integer `%vN` and floating-point `%fN`), spanning
        // the id range the regex body reaches (~135k int vregs).
        for id in [0u32, 7, 42, 135_293] {
            rows.push((vreg_name(id), Kind::VReg));
            rows.push((fp_vreg_name(id), Kind::VReg));
        }
        rows
    }

    #[test]
    fn parse_render_round_trips_over_the_corpus() {
        for (value, _kind) in corpus() {
            let round_tripped = Operand::parse(&value).render();
            assert_eq!(
                round_tripped, value,
                "round-trip changed the operand string `{value}` -> `{round_tripped}`"
            );
        }
    }

    #[test]
    fn corpus_covers_every_operand_kind() {
        use std::collections::HashSet;
        let present: HashSet<Kind> = corpus().into_iter().map(|(_, kind)| kind).collect();
        for kind in [
            Kind::VReg,
            Kind::Phys,
            Kind::Imm,
            Kind::Bool,
            Kind::Label,
            Kind::Symbol,
            Kind::TypeName,
            Kind::StackSentinel,
        ] {
            assert!(
                present.contains(&kind),
                "corpus is missing operand kind {kind:?}; round-trip proof would pass vacuously for it"
            );
        }
    }

    #[test]
    fn every_from_conversion_renders_verbatim() {
        // All four `impl Into<Operand>` entry points the `field` funnel accepts —
        // `&str`, `&String`, `&&str`, and owned `String` — must produce a `Raw`
        // that renders back to the identical bytes (so an unmigrated `.field`
        // caller is a no-op through the funnel).
        let owned = String::from("_mfb_arena_alloc");
        let borrowed: &str = "x0";
        let double: &&str = &borrowed;
        assert_eq!(Operand::from("x0").render(), "x0");
        assert_eq!(Operand::from(&owned).render(), "_mfb_arena_alloc");
        assert_eq!(Operand::from(double).render(), "x0");
        assert_eq!(Operand::from(owned.clone()).render(), "_mfb_arena_alloc");
        // Each is the `Raw` arm, never a typed one.
        assert!(matches!(Operand::from("42"), Operand::Raw(_)));
        assert!(matches!(Operand::from(String::from("%v3")), Operand::Raw(_)));
    }

    #[test]
    fn virtual_registers_type_by_class() {
        // The two sentinel prefixes classify to the two classes, and the id is
        // preserved. This is the identity plan-78-C's typed reads depend on.
        assert_eq!(
            Operand::parse("%v17"),
            Operand::VReg {
                class: RegClass::Int,
                id: 17
            }
        );
        assert_eq!(
            Operand::parse("%f3"),
            Operand::VReg {
                class: RegClass::Fp,
                id: 3
            }
        );
    }

    #[test]
    fn code_instruction_stores_typed_operands() {
        // plan-78-B: `CodeInstruction.fields` stores a typed `Operand`, not a
        // rendered `String`. A `&str` producer yields `Raw`; a typed immediate
        // yields `Imm`; `operand()` returns the typed value and `get()` renders
        // both back to the identical string (byte-identity).
        use crate::target::shared::code::CodeInstruction;
        let inst = CodeInstruction::new("mov")
            .field("dst", "x0")
            .field("value", Operand::imm(42));
        assert_eq!(inst.operand("dst"), Some(&Operand::Raw("x0".into())));
        assert_eq!(inst.operand("value"), Some(&Operand::Imm(42)));
        assert_eq!(inst.operand("missing"), None);
        assert_eq!(inst.get("dst").as_deref(), Some("x0"));
        assert_eq!(inst.get("value").as_deref(), Some("42"));
    }

    #[test]
    fn non_canonical_decimals_stay_raw() {
        // A `u64` immediate past `i64::MAX`, a leading-zero form, and a signed
        // form must not be lifted to `Imm` (they would not render back exactly).
        for value in ["18446744073709551615", "007", "+5", "-0"] {
            assert_eq!(
                Operand::parse(value).render(),
                value,
                "non-canonical decimal `{value}` failed to round-trip"
            );
            assert!(
                matches!(Operand::parse(value), Operand::Raw(_)),
                "non-canonical decimal `{value}` should stay Raw"
            );
        }
        // A negative immediate that *is* canonical types as `Imm` and round-trips.
        assert_eq!(Operand::parse("-5"), Operand::Imm(-5));
        assert_eq!(Operand::parse("-5").render(), "-5");
    }
}
