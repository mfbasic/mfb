//! Target-neutral machine IR (MIR) scaffold — plan-00-A.
//!
//! This is the foundational *layer* of the MIR effort (`planning/mir.md`): the
//! seam between the NIR→builder lowering and the AArch64 backend through which
//! all code now flows (plan-00-G flipped MIR to the sole path and deleted the
//! `direct` backend), plus the `-mir` dump tooling.
//!
//! [`MirOp`] reuses the [`CodeOp`] variant set 1:1 (`mir.md §10`); op families
//! were neutralized one at a time (plans B–F) with the round trip
//! [`lower_to_mir`] → [`select_aarch64`](crate::arch::aarch64::select::select_aarch64) kept the identity, so each step was
//! proven byte-identical against the `direct` path before that path was
//! retired. The round trip is still the identity for builder ops, which is why
//! the AArch64 output was unperturbed by making MIR the sole path.
//!
//! The `-mir` dump (`mir.md §12a`) is the neutral counterpart to `-ncode`: it
//! serializes the MIR stream (neutral ops, virtual registers, *no* `target`/
//! `arch`) captured **before** register allocation and instruction selection.

// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::code_impl::join_json;
use crate::codegen::engine::builder::code_impl::ToCodeJson;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::json::json_string;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::target::shared::regmodel::RegisterModel;

/// One MIR instruction: a neutral op plus the same ordered string-field bag as
/// [`CodeInstruction`] (`mir.md §10`). The allocator's field-based liveness
/// keeps working unchanged because the shape is identical.
pub(crate) struct MirInstruction {
    pub(crate) op: MirOp,
    /// plan-79: typed `Operand` values (was `String`). Carrying the typed operand
    /// through the MIR boundary means a produced `VReg`/`Phys` survives
    /// production → MIR → select → regalloc → encode instead of being rendered to a
    /// `String` here and re-boxed as `Raw` in `select` — which is what capped
    /// plan-82's allocation win at 2.3% (plan-82-A §CORE-PREMISE FALSIFICATION).
    pub(crate) fields: Vec<(&'static str, Operand)>,
    /// plan-71-C Phase 0: carries the builder source `file:line` from the
    /// [`CodeInstruction`] this MIR op was lowered from, so `select_x86` can stamp
    /// it back onto the selected instruction and the `MFB_BUG387_AUDIT` cross-check
    /// can name the exact re-tokenization site. Never serialized; byte-identical.
    pub(crate) source: Option<&'static core::panic::Location<'static>>,
}

// The MIR op set is four groups (`mir.md §4`/§10):
//
// - **mirror** ops carry over a single AArch64 [`CodeOp`] 1:1 *with its mnemonic
//   unchanged* (the plan-00-A layer, plus the ops whose AArch64 mnemonic is
//   already ISA-neutral — `clz`/`rbit`/`msub`/the universal ALU/move/load
//   shapes). The macro derives the `CodeOp` <-> `MirOp` conversions from this
//   list, so the mapping is *total and exhaustive by construction* — a missing
//   variant is a compile error, which is what keeps the byte-identical round
//   trip honest. Their display mnemonic delegates to `CodeOp`.
// - **renamed** ops also carry a single AArch64 [`CodeOp`] 1:1 (plan-00-C), but
//   the MIR mnemonic is the **ISA-neutral semantic name** (`smulh`→`mulhi_s`,
//   `rorv`→`rotr`, `fcvtzs_x_from_d`→`f2i_trunc`, …) rather than the AArch64
//   instruction. The conversion stays 1:1 — selection maps the neutral MIR op
//   straight back to its one `CodeOp` and the encoder is untouched — so the
//   `.ncode`/binary remain byte-identical; only the `-mir` dump shows the
//   neutral name (`mir.md §4`, validation §5: no `smulh`/`rorv`/`adrp`).
// - **fused** ops are the neutral, *flagless* control-flow ops (plan-00-B): a
//   compare-and-branch (`br_cc`/`fbr_cc`) or an explicit-overflow arithmetic
//   (`add_ovf`/`sub_ovf`) that each stand in for an AArch64 flag-setter + the
//   flag-reading branch that consumed it. They have no single `CodeOp` (they
//   expand to two), so they are excluded from `to_code` and carry an explicit
//   mnemonic. [`lower_to_mir`] produces them by fusing adjacent pairs;
//   [`select_aarch64`](crate::arch::aarch64::select::select_aarch64) expands them back to the exact `cmp; b.cc` / `adds; b.vc`
//   the backend emits today.
// - **expand** ops are the neutral *structural* ops (plan-00-C): a single MIR op
//   that an ISA realizes with a short fixed instruction *sequence*. Today the
//   only one is `addr_of <sym>` — one PC-relative symbol-address op that AArch64
//   selects as the `adrp; add :lo12:` page pair (x86 `lea` RIP-rel, rv64
//   `auipc; addi` later). Like fused ops they have no single `CodeOp` (they
//   expand to two); [`lower_to_mir`] produces them by fusing the adjacent
//   `adrp; add_pageoff` pair and [`select_aarch64`](crate::arch::aarch64::select::select_aarch64) expands them back
//   byte-for-byte (`mir.md §4`).
macro_rules! mir_ops {
    (
        mirror { $($mv:ident),+ $(,)? }
        renamed { $($rc:ident => $rv:ident => $rm:literal),+ $(,)? }
        simd { $($sv:ident => $sm:literal),+ $(,)? }
        fused { $($fv:ident => $fm:literal),+ $(,)? }
        expand { $($ev:ident => $em:literal),+ $(,)? }
    ) => {
        /// Neutral machine-IR opcode (`mir.md §4`/§10).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum MirOp {
            $($mv,)+
            $($rv,)+
            $($sv,)+
            $($fv,)+
            $($ev,)+
        }

        impl MirOp {
            /// Map a selected AArch64 [`CodeOp`] up to its MIR op. Total over
            /// `CodeOp` (every `CodeOp` is a mirror, a renamed, or a `v128`
            /// variant; the fused/expand ops are produced by fusion in
            /// [`lower_to_mir`], never by `from_code`).
            fn from_code(op: CodeOp) -> Self {
                match op {
                    $(CodeOp::$mv => MirOp::$mv,)+
                    $(CodeOp::$rc => MirOp::$rv,)+
                    $(CodeOp::$sv => MirOp::$sv,)+
                }
            }

            /// The single [`CodeOp`] a mirror/renamed/`v128` op selects, or `None`
            /// for a fused control-flow op or a structural expand op (which expand
            /// to two instructions). Maps the neutral op back to its concrete
            /// CodeOp — including the renames (`call`→`bl`, `mulhi_u`→`umulh`, …) —
            /// so it is NOT a mnemonic round-trip; both backends' selection use it.
            pub(crate) fn to_code(self) -> Option<CodeOp> {
                match self {
                    $(MirOp::$mv => Some(CodeOp::$mv),)+
                    $(MirOp::$rv => Some(CodeOp::$rc),)+
                    $(MirOp::$sv => Some(CodeOp::$sv),)+
                    $(MirOp::$fv => None,)+
                    $(MirOp::$ev => None,)+
                }
            }

            /// Display mnemonic for the `-mir` dump.
            pub(crate) fn mnemonic(self) -> &'static str {
                match self {
                    $(MirOp::$mv => CodeOp::$mv.mnemonic(),)+
                    $(MirOp::$rv => $rm,)+
                    $(MirOp::$sv => $sm,)+
                    $(MirOp::$fv => $fm,)+
                    $(MirOp::$ev => $em,)+
                }
            }

            /// Whether this op is a `v128` SIMD op (the neutral NEON-tail
            /// vocabulary, plan-00-E). Used by the `-mir` neutrality checks.
            #[cfg(test)]
            pub(crate) fn is_v128(self) -> bool {
                matches!(self, $(MirOp::$sv)|+)
            }
        }
    };
}

mir_ops!(
    mirror {
        Label, Mov, MovImm, Add, Adds, Sub, Subs, And, Orr, Eor, Mvn, Mul, Lslv, Lsrv, Asrv, Clz,
        Rbit, Sxtw, SDiv, UDiv, MSub, LslImm, LsrImm, AsrImm, AddImm, SubImm, SubSp, AddSp, CmpImm, Cmp,
        BranchEq, BranchNe, BranchGe, BranchLt, BranchGt, BranchLe, BranchVc, BranchVs, BranchHi,
        BranchLo, BranchMi, BranchLs, Branch, BranchSelf, Ret,
        // x86-only float-compare branches, synthesized by `select_x86` *after*
        // MIR lowering (never produced by `from_code`); listed here only so the
        // `CodeOp`→`MirOp` map stays total.
        X86Jae, X86Jp, X86Jnp, X86Ja, X86Jb, X86Jbe, X86Je, X86Jne,
        // rv64-only compare-and-branch / float-compare-to-GPR / set-less-than,
        // synthesized by `select_riscv64` *after* MIR lowering (never produced by
        // `from_code`); listed here only so the `CodeOp`→`MirOp` map stays total
        // (plan-99).
        RvBr, RvFcmp, Slt, Sltu,
        // rv64-only RVV vector op (plan-32-B), synthesized by `select_riscv64`'s
        // v128 dual-path lowering *after* MIR lowering (never produced by
        // `from_code`); listed here only so the `CodeOp`→`MirOp` map stays total.
        RvVop,
        LdrU64, LdrU32, LdrU16, LdrU8, StrU64, StrU32, StrU16, StrU8, LdrD, StrD, Adrp, AddPageOff,
        // AArch64 load-acquire / store-release (bug-564): emitted only by the
        // macOS Network.framework trampolines and `tls::write`'s gate loads.
        StlrU64, LdarU64, LdarU32,
        FMovDFromD, FAddD, FSubD, FMulD, FDivD, FMinnmD, FMaxnmD, FNegD, FAbsD, FSqrtD, FCvtSFromD, FCvtDFromS, FCmpD,
        FCmpZeroD, FMaddD, FMsubD, FNmsubD, FNmaddD,
    }
    // Neutral semantic names for the AArch64-specific scalar shapes (plan-00-C
    // Phases 3 & 4). `CodeOp` (lhs) ⇒ `MirOp` (mid) ⇒ neutral mnemonic (rhs);
    // the conversion is still 1:1, so selection is byte-identical.
    renamed {
        // §3 "exotic" integer ops — not 1:1 across ISAs, named semantically here
        // so the backends can select natively or expand.
        SMulH => MulhiS => "mulhi_s",     // smulh: signed 64×64→high 64
        UMulH => MulhiU => "mulhi_u",     // umulh: unsigned 64×64→high 64
        // Explicit-carry add/sub (plan-00-G §4): carry-in/out are *values*, so a
        // 128-bit chain is allocation-safe (no implicit flag between limbs). The
        // AArch64 backend expands add to `adds; cset` (no carry-in) or `cmp;
        // adcs; cset`, and sub to `subs; sbcs; cset`.
        AddCarry => AddC => "addc",       // add_carry:  a + b + carry_in, → carry_out
        SubBorrow => SubB => "subc",      // sub_borrow: a - b - borrow_in, → borrow_out
        Rorv => Rotr => "rotr",           // rorv:  rotate-right (64-bit, variable)
        RorvW => RotrW => "rotr_w",       // rorv:  rotate-right (32-bit, variable)
        RevW => BswapW => "bswap_w",      // rev:   byte reverse (32-bit)
        RevX => Bswap => "bswap",         // rev:   byte reverse (64-bit)
        // §4 float↔int conversions (rounding-mode family) + bit reinterpret.
        FCvtzsXFromD => F2iTrunc => "f2i_trunc",     // fcvtzs: toward zero
        FCvtmsXFromD => F2iFloor => "f2i_floor",     // fcvtms: toward −inf
        FCvtpsXFromD => F2iCeil => "f2i_ceil",       // fcvtps: toward +inf
        FCvtasXFromD => F2iNearest => "f2i_nearest", // fcvtas: nearest, ties away
        SCvtfDFromX => I2f => "i2f",                 // scvtf:  signed int → f64
        FMovDFromX => FmovI2f => "fmov_i2f",         // fmov:   i64 bits → f64 (reinterpret)
        FMovXFromD => FmovF2i => "fmov_f2i",         // fmov:   f64 bits → i64 (reinterpret)
        // §7 machine-y ops (plan-00-F): the call/syscall vocabulary the runtime
        // helpers use, named semantically so the helper MIR is ISA-independent.
        // The ABI register placement (which GPRs carry args/nr/result) is a
        // per-ISA backend detail, not named here.
        BranchLink => Call => "call",            // bl:  direct call to a symbol
        BranchLinkRegister => CallIndirect => "call_indirect", // blr: call via a register
        Svc => Syscall => "syscall",             // svc: trap into the OS (x86 syscall, rv64 ecall)
    }
    // The fixed-width `v128` SIMD vocabulary (plan-00-E, `mir.md §6`): the whole
    // NEON tail of `CodeOp`, re-expressed as neutral lane ops. Each maps 1:1 to
    // its NEON `CodeOp` (the MirOp variant keeps the name; only the *mnemonic*
    // is neutralized — the `v128.` namespace), so selection is byte-identical
    // and only the `-mir` dump changes (no `fadd_v`/`fmla_v`/`frintn_v`/`bsl_v`
    // mnemonic survives). Lanes are `2×f64` / `2×i64` / `16×i8` as the op needs;
    // the lane semantics (NaN of `fmin`/`fmax`, `bsl`/`bit` mask polarity,
    // round-mode ties, lane-compare all-ones/zero masks) are the contract the
    // x86_64 (SSE2+FMA3+SSE4.1) and rv64 (scalarized) backends realize against
    // — pinned by the lane-semantics test matrix below (`mir.md §6`, §12.1).
    simd {
        // 128-bit vector load / store.
        LdrQ => "v128.load",
        StrQ => "v128.store",
        // FP three-same `.2d` (two f64 lanes).
        FAddV => "v128.fadd",
        FSubV => "v128.fsub",
        FMulV => "v128.fmul",
        FDivV => "v128.fdiv",
        FMlaV => "v128.fma",     // fused multiply-accumulate: dst += lhs*rhs
        FMlsV => "v128.fms",     // fused multiply-subtract:   dst -= lhs*rhs
        FMinV => "v128.fmin",    // NaN-propagating min (NEON `fmin`, not `fminnm`)
        FMaxV => "v128.fmax",    // NaN-propagating max
        // FP lane compares → per-lane all-ones (true) / all-zeros (false) mask.
        FCmGtV => "v128.fcmp_gt",
        FCmGeV => "v128.fcmp_ge",
        FCmEqV => "v128.fcmp_eq",
        // FP two-reg-misc `.2d`.
        FAbsV => "v128.fabs",
        FNegV => "v128.fneg",
        FSqrtV => "v128.fsqrt",
        // Round to integral f64, by mode (the `frint*` family).
        FRintpV => "v128.fround_ceil",     // toward +inf
        FRintmV => "v128.fround_floor",    // toward -inf
        FRintaV => "v128.fround_nearest",  // nearest, ties away from zero
        FRintnV => "v128.fround_even",     // nearest, ties to even
        FRintzV => "v128.fround_trunc",    // toward zero
        // Lane f64↔i64 conversions (mirror the scalar plan-00-C names).
        FCvtzsV => "v128.f2i_trunc",       // f64→i64 toward zero
        FCvtasV => "v128.f2i_nearest",     // f64→i64 nearest, ties away
        ScvtfV => "v128.i2f",              // i64→f64 signed
        // FP compare-against-zero `.2d` → lane mask.
        FCmGtZeroV => "v128.fcmp_gt_zero",
        FCmGeZeroV => "v128.fcmp_ge_zero",
        FCmEqZeroV => "v128.fcmp_eq_zero",
        FCmLtZeroV => "v128.fcmp_lt_zero",
        FCmLeZeroV => "v128.fcmp_le_zero",
        // Integer three-same `.2d` (two i64 lanes).
        AddV => "v128.add",
        SubV => "v128.sub",
        CmGtV => "v128.icmp_gt",   // signed lane compare → lane mask
        CmGeV => "v128.icmp_ge",
        CmEqV => "v128.icmp_eq",
        SshlV => "v128.sshl",      // signed variable lane shift (neg rhs = right)
        UshlV => "v128.ushl",      // unsigned variable lane shift
        // Integer two-reg-misc `.2d`.
        NegV => "v128.neg",
        AbsV => "v128.abs",
        Cnt8bV => "v128.cnt8b",
        Addv8bV => "v128.addv8b",
        // Bitwise three-same `.16b`.
        AndV => "v128.and",
        OrrV => "v128.or",
        EorV => "v128.xor",
        BslV => "v128.bsl",        // bit-select: mask in dst picks lhs vs rhs bits
        BitV => "v128.bit",        // bit-insert-if-true (mask in rhs)
        // Shifted-immediate `.2d`.
        ShlV => "v128.shl_imm",
        SshrV => "v128.sshr_imm",
        UshrV => "v128.ushr_imm",
        // Lane broadcast / extract (scalar GPR ↔ lane).
        DupVFromX => "v128.dup_from_gpr",
        UmovXFromV => "v128.umov_to_gpr",
    }
    fused {
        // Flagless compare-and-branch (`mir.md §5`): one neutral op for the
        // implicit `cmp; b.cc` / `fcmp; b.cc` pairing, operands carried (no
        // hidden NZCV dependency). `BrCc`/`FBrCc` take two register operands;
        // `BrCcImm` an immediate rhs; `FBrCcZero` a single operand (compare vs
        // zero). The condition is carried as a `cond` field.
        BrCc => "br_cc",
        BrCcImm => "br_cc_imm",
        FBrCc => "fbr_cc",
        FBrCcZero => "fbr_cc_zero",
        // Explicit-overflow arithmetic (`mir.md §5`): the value plus the
        // overflow-trap branch that read the V flag, fused into one op.
        AddOvf => "add_ovf",
        SubOvf => "sub_ovf",
        // Syscall-and-check (`mir.md §7`, plan-00-F): the macOS syscall error
        // idiom `svc; b.<carry>` — the trap plus the branch that reads the carry
        // flag the syscall sets on error — fused into one flagless op. This is
        // the last flag-reading branch in the helper MIR (plan-00-B deferred it
        // here as "the syscall neutralization"). The `cond` field carries the
        // carry condition; a backend realizes the check its own way (macOS sets
        // carry; Linux/rv64 return `-errno` and compare). [`select_aarch64`](crate::arch::aarch64::select::select_aarch64)
        // expands it back to the exact `svc; b.<carry>` byte-for-byte.
        SyscallBr => "syscall_br",
    }
    expand {
        // PC-relative symbol address (`mir.md §4`): one neutral op for the
        // AArch64 `adrp; add :lo12:` page pair. Fused from the adjacent pair in
        // [`lower_to_mir`] and expanded back byte-for-byte in [`select_aarch64`](crate::arch::aarch64::select::select_aarch64).
        AddrOf => "addr_of",
    }
);

/// The neutral MIR operand that names the arena-state base pointer (`mir.md
/// §7`, plan-00-D). MIR code that reaches the arena references `arena_base`
/// instead of a pinned physical register; the AArch64 backend *realizes* it as
/// the pinned `x19` (`Aarch64RegisterModel::arena_base`, reserved from
/// allocation), x86_64 will realize it as a TLS/memory load (plan-00-H). The
/// abstraction is the identity here: [`lower_to_mir`] renames the realization
/// register to `arena_base`, [`select_aarch64`](crate::arch::aarch64::select::select_aarch64) renames it back, so the codegen
/// stream is byte-identical while the `-mir` dump shows `arena_base`, not `x19`.
pub(crate) const ARENA_BASE: &str = "arena_base";

/// The physical register AArch64 realizes [`ARENA_BASE`] as — pinned `x19`. Since
/// plan-34-A this is the aarch64 backend's `regmodel::ARENA_BASE_REGISTER`, not
/// the shared operand token (`ARENA_STATE_REGISTER` now *is* [`ARENA_BASE`]).
/// Used only by the backend realization tests, which feed this physical name and
/// prove each selection maps it back to its ISA home (`x19`/`s11`/`r15`).
pub(crate) fn arena_base_realization() -> &'static str {
    crate::arch::aarch64::regmodel::ARENA_BASE_REGISTER
}

/// Rewrite every field value equal to `from` to `to` across an instruction's
/// field bag. The arena base register is pinned program-wide (reserved from
/// allocation), so a field value equal to it is unambiguously the arena base —
/// never an immediate (numbers), symbol, or label — which is what makes the
/// `x19`⇄`arena_base` rename total and reversible.
pub(crate) fn rename_field_values(fields: &mut [(&'static str, Operand)], from: &str, to: &str) {
    for (_, value) in fields.iter_mut() {
        // The arena-base realization (and every rename this serves) is a hardcoded
        // physical spelled as `Raw` — a `VReg`/`Phys`/`Imm` is never the arena base.
        // Matching `Raw` directly (no `render()`) keeps this per-field-per-
        // instruction pass in `lower_to_mir` allocation-free (plan-79).
        if let Operand::Raw(text) = value {
            if text.as_ref() == from {
                *value = Operand::from(to);
            }
        }
    }
}

/// The [`Operand`]-valued twin of [`rename_field_values`] for the arch backends'
/// selected [`CodeInstruction`] streams (plan-78-B flipped `CodeInstruction`'s
/// value type to `Operand`; `MirInstruction` stays `String`). Replaces any
/// operand rendering to `from` with `Operand::from(to)`.
pub(crate) fn rename_operand_field_values(
    fields: &mut [(&'static str, Operand)],
    from: &str,
    to: &str,
) {
    for (_, value) in fields.iter_mut() {
        // Same as `rename_field_values`: the rename target is a `Raw` physical
        // token, so match `Raw` directly and skip the per-field `render()` alloc
        // this pass would otherwise do over the whole selected stream (plan-79).
        if let Operand::Raw(text) = value {
            if text.as_ref() == from {
                *value = Operand::from(to);
            }
        }
    }
}

/// Carry a `CodeInstruction`'s typed operand fields into a `MirInstruction`'s
/// field bag (plan-79: both are now `Operand`-valued, so this is a clone — no
/// render). Byte-identical: the operand is carried verbatim.
pub(crate) fn mir_fields_from_code(
    fields: &[(&'static str, Operand)],
) -> Vec<(&'static str, Operand)> {
    fields.to_vec()
}

/// Carry a `MirInstruction`'s typed operand fields into the `CodeInstruction` a
/// backend `select` builds (plan-79: a clone — no re-`Raw` boxing). A `%vN`
/// therefore reaches regalloc as a typed `VReg`, and a colored physical stays a
/// typed `Phys`, instead of being rebuilt as `Operand::Raw`.
pub(crate) fn code_fields_from_mir(
    fields: &[(&'static str, Operand)],
) -> Vec<(&'static str, Operand)> {
    fields.to_vec()
}

/// The fused (flagless) MIR op a given AArch64 flag-setter folds *into* when it
/// is immediately followed by a flag-reading branch, or `None` if the op never
/// fuses. The setter's flags are otherwise invisible in the MIR.
fn fused_variant(op: CodeOp) -> Option<MirOp> {
    match op {
        CodeOp::Cmp => Some(MirOp::BrCc),
        CodeOp::CmpImm => Some(MirOp::BrCcImm),
        CodeOp::FCmpD => Some(MirOp::FBrCc),
        CodeOp::FCmpZeroD => Some(MirOp::FBrCcZero),
        CodeOp::Adds => Some(MirOp::AddOvf),
        CodeOp::Subs => Some(MirOp::SubOvf),
        // The syscall sets the carry flag (macOS error idiom); a following
        // carry-reading branch folds into the flagless `syscall_br` (plan-00-F).
        CodeOp::Svc => Some(MirOp::SyscallBr),
        _ => None,
    }
}

/// The AArch64 flag-setter [`CodeOp`] a fused MIR op expands back to (its first
/// of two instructions), or `None` for a non-fused op.
pub(crate) fn fused_setter_codeop(op: MirOp) -> Option<CodeOp> {
    match op {
        MirOp::BrCc => Some(CodeOp::Cmp),
        MirOp::BrCcImm => Some(CodeOp::CmpImm),
        MirOp::FBrCc => Some(CodeOp::FCmpD),
        MirOp::FBrCcZero => Some(CodeOp::FCmpZeroD),
        MirOp::AddOvf => Some(CodeOp::Adds),
        MirOp::SubOvf => Some(CodeOp::Subs),
        MirOp::SyscallBr => Some(CodeOp::Svc),
        _ => None,
    }
}

/// Whether a branch reads condition flags (so it pairs with a preceding
/// flag-setter). The unconditional `b`/`branch_self`/`ret` and the `call`/
/// `call_indirect` ops do not. A `svc; b.<carry>` syscall error check fuses
/// into the flagless `syscall_br` (plan-00-F): the syscall sets the carry flag
/// and is a fusable setter (see [`fused_variant`]), so the carry-reading branch
/// folds into it rather than surviving as a standalone flag branch — leaving the
/// helper MIR fully flagless (`mir.md §5`/§7).
fn is_flag_reading_branch(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::BranchEq
            | CodeOp::BranchNe
            | CodeOp::BranchGe
            | CodeOp::BranchLt
            | CodeOp::BranchGt
            | CodeOp::BranchLe
            | CodeOp::BranchVc
            | CodeOp::BranchVs
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::BranchMi
            | CodeOp::BranchLs
    )
}

/// Field key marking the boundary between a fused op's compare/arith operands
/// and its branch: its value is the branch mnemonic (e.g. `"b.lt"`). No
/// flag-setter or branch field is named `cond`, so the first such field is
/// unambiguously the split point for [`select_aarch64`](crate::arch::aarch64::select::select_aarch64).
pub(crate) const FUSED_COND_FIELD: &str = "cond";

/// Field key marking a compare-and-branch that **reuses** the immediately
/// preceding fused op's comparison instead of computing its own (a second/third
/// branch on one AArch64 `cmp`, e.g. the 3-way `cmp; b.lo; b.hi` string
/// ordering). The op still carries its operands (so it is self-contained and an
/// ISA without flag reuse can re-emit the compare), but the AArch64 selector
/// emits only the branch — reproducing the single shared `cmp` byte-for-byte.
pub(crate) const FUSED_SHARE_FIELD: &str = "share";

/// Raise an AArch64 instruction stream to the neutral MIR (`NIR → MIR`). Mirror
/// ops carry over 1:1 (plan-00-A); a flag-setter immediately followed by the
/// flag-reading branch that consumes it is *fused* into one flagless op
/// (plan-00-B): the compare/arith operands and the branch condition are carried
/// explicitly, so the MIR has no `cmp; b.cc` pair with a hidden NZCV
/// dependency. The fused op's fields are `[<setter fields>, cond=<branch
/// mnemonic>, <branch fields>]`.
///
/// Fusion is pairwise and local: a setter not directly followed by a
/// flag-reading branch (e.g. an `adds; adc` carry chain), and a flag-reading
/// branch not directly preceded by a fusable setter (e.g. a second branch on
/// one compare, or a `svc; b.lo` syscall carry check — both hand-written-helper
/// patterns), stay as mirror ops. The builder-emitted control flow this seam
/// neutralizes always pairs one setter with one branch, so it fuses fully.
pub(crate) fn lower_to_mir(instructions: &[CodeInstruction]) -> Vec<MirInstruction> {
    // Borrowing entry: the rare `-mir` capture path and the unit tests reuse the
    // pre-lowering stream, so they clone it into the by-value lowering. The hot
    // pipeline callers instead move their (dropped-after) stream through
    // [`lower_to_mir_owned`] to avoid re-cloning every `fields` Vec (plan-84).
    // `CodeInstruction` is not `Clone`; rebuild each element from its parts.
    lower_to_mir_owned(
        instructions
            .iter()
            .map(|instruction| CodeInstruction {
                op: instruction.op,
                fields: instruction.fields.clone(),
                source: instruction.source,
            })
            .collect(),
    )
}

/// By-value [`lower_to_mir`]: consumes the `CodeInstruction` stream so each
/// non-fused instruction's `fields` Vec is **moved** into the produced
/// `MirInstruction` rather than `to_vec`-cloned (plan-84 Phase 2). The fused /
/// `addr_of` / shared-branch cases still clone — they slice-borrow or reuse the
/// setter's fields across multiple outputs — but they are the minority. Every
/// produced field bag is byte-identical to the borrowing path; only the carrier
/// (move vs clone) changes.
pub(crate) fn lower_to_mir_owned(mut instructions: Vec<CodeInstruction>) -> Vec<MirInstruction> {
    // Build a fused op from a flag-setter's operands + a branch. `shared` marks
    // a branch that reuses the preceding comparison (see [`FUSED_SHARE_FIELD`]).
    fn fuse(
        op: MirOp,
        setter_fields: &[(&'static str, Operand)],
        branch: &CodeInstruction,
        shared: bool,
    ) -> MirInstruction {
        let mut fields = mir_fields_from_code(setter_fields);
        fields.push((FUSED_COND_FIELD, branch.op.mnemonic().into()));
        fields.extend(mir_fields_from_code(&branch.fields));
        if shared {
            fields.push((FUSED_SHARE_FIELD, "true".into()));
        }
        // Fused ops (cmp+branch) are never a Family-1a re-tokenization target, so a
        // precise source is unneeded; carry the branch's for completeness.
        MirInstruction {
            op,
            fields,
            source: branch.source,
        }
    }

    // Fuse an `adrp; add_pageoff` page pair into one `addr_of`, or `None` if the
    // two do not form a single same-register symbol-address sequence.
    fn fuse_addr_of(adrp: &CodeInstruction, add: &CodeInstruction) -> Option<MirInstruction> {
        if add.op != CodeOp::AddPageOff {
            return None;
        }
        let dst = adrp.get("dst")?;
        let symbol = adrp.get("symbol")?;
        if add.get("dst").as_deref() == Some(dst.as_str())
            && add.get("src").as_deref() == Some(dst.as_str())
            && add.get("symbol").as_deref() == Some(symbol.as_str())
        {
            // Carry the `adrp` field bag (`[dst, symbol]`); the expansion
            // reconstructs `add_pageoff`'s `src == dst` from it.
            Some(MirInstruction {
                op: MirOp::AddrOf,
                fields: mir_fields_from_code(&adrp.fields),
                source: adrp.source,
            })
        } else {
            None
        }
    }

    let mut out = Vec::with_capacity(instructions.len());
    let mut i = 0;
    while i < instructions.len() {
        // Read the op by copy so the common case below can `mem::take` this
        // instruction's `fields` without holding an outstanding borrow (plan-84).
        let setter_op = instructions[i].op;
        // `addr_of` fusion (plan-00-C): a symbol-address `adrp <dst>, <sym>;
        // add_pageoff <dst>, <dst>, <sym>` page pair is one neutral PC-relative
        // address op. The two are always emitted adjacently with `src == dst`
        // (every builder/helper site goes through `abi::load_page_address` +
        // `abi::add_page_offset` on the same register), so the fused op carries
        // just `[dst, symbol]` and `select_aarch64` rebuilds the pair exactly.
        if setter_op == CodeOp::Adrp {
            if i + 1 < instructions.len() {
                if let Some(addr_of) = fuse_addr_of(&instructions[i], &instructions[i + 1]) {
                    out.push(addr_of);
                    i += 2;
                    continue;
                }
            }
        }
        if let Some(fused_op) = fused_variant(setter_op) {
            if instructions
                .get(i + 1)
                .is_some_and(|next| is_flag_reading_branch(next.op))
            {
                // The first branch owns the comparison. The setter's fields are
                // reused across every shared branch below, so this path clones
                // (borrow + multi-use); it is the minority.
                let setter_idx = i;
                out.push(fuse(
                    fused_op,
                    &instructions[setter_idx].fields,
                    &instructions[setter_idx + 1],
                    false,
                ));
                i += 2;
                // Any further consecutive flag-reading branches read the *same*
                // flags (nothing reset them), so they share this comparison —
                // the 3-way `cmp; b.lo; b.hi` ordering, etc.
                while instructions
                    .get(i)
                    .is_some_and(|next| is_flag_reading_branch(next.op))
                {
                    out.push(fuse(
                        fused_op,
                        &instructions[setter_idx].fields,
                        &instructions[i],
                        true,
                    ));
                    i += 1;
                }
                continue;
            }
        }
        // Common (non-fused) case: MOVE the fields Vec into the MIR instruction
        // instead of `mir_fields_from_code`'s `to_vec` clone (plan-84 Phase 2).
        let source = instructions[i].source;
        let fields = std::mem::take(&mut instructions[i].fields);
        out.push(MirInstruction {
            op: MirOp::from_code(setter_op),
            fields,
            source,
        });
        i += 1;
    }
    // `arena_base` abstraction (plan-00-D §2): the pinned arena register is an
    // AArch64 realization detail, so the neutral MIR names `arena_base` instead.
    let realization = arena_base_realization();
    for instruction in &mut out {
        rename_field_values(&mut instruction.fields, realization, ARENA_BASE);
    }
    out
}

/// Route a finished function's instruction stream through the neutral MIR and
/// back (`select_aarch64 ∘ lower_to_mir`, the identity) — plan-00-F/G. This is
/// where the **hand-written runtime helpers** enter the MIR
/// pipeline: unlike the builder-emitted functions they never pass through the
/// pre-allocation seam in `run_register_allocation`, so without this their
/// streams would skip the MIR entirely. Routing them here brings the entry
/// sequence, the arena allocator, the error path, the PCG64 RNG, the math
/// kernels, and the thread trampoline through the neutral MIR like every builder
/// function (plan-00-G: MIR is now the sole code path). Builder functions
/// already round-tripped pre-allocation; routing their final (post-allocation,
/// post-frame) stream again is a second identity pass over the frame/peephole
/// output.
pub(crate) fn route_function_through_mir(function: &mut CodeFunction) {
    // Move the stream through the boundary (plan-84 Phase 2): it is overwritten
    // by the selection result below, so ownership is free.
    let neutral = lower_to_mir_owned(std::mem::take(&mut function.instructions));
    function.instructions = active_backend().select(neutral);
}

// --- Backend dispatch ---------------------------------------------------------

/// A code-generation backend: the per-ISA tail of the pipeline that consumes
/// neutral MIR. The shared lowering produces [`MirInstruction`]s, then asks the
/// **active** backend to (1) `select` them into that ISA's machine ops and (2)
/// supply the [`RegisterModel`] the shared allocator colors vregs against.
/// AArch64 implements it via [`select_aarch64`](crate::arch::aarch64::select::select_aarch64) + `Aarch64RegisterModel`; a new
/// ISA adds its own `impl Backend` under `src/arch/<isa>/` plus a
/// `CodegenPlatform` that returns it — with no shared-code edit at the
/// selection / allocation sites, which is what makes a new backend additive
/// (plan-00-H/I).
pub(crate) trait Backend: Sync {
    /// Select neutral MIR into this ISA's machine instructions. Consumes the MIR
    /// stream by value so each non-fused instruction's `fields` Vec is **moved**
    /// into the produced `CodeInstruction` instead of `code_fields_from_mir`'s
    /// `to_vec` clone (plan-84 Phase 2). The stream is dropped after selection at
    /// every call site, so ownership costs nothing.
    fn select(&self, neutral: Vec<MirInstruction>) -> Vec<CodeInstruction>;
    /// The register model the shared allocator colors vregs against.
    fn register_model(&self) -> &'static dyn RegisterModel;
    /// Whether this backend is AArch64 — lets shared codegen pick a NEON path
    /// (e.g. `bits::popCount` via `CNT`+`ADDV`, plan-39 K2) that other ISAs can't
    /// express cheaply, while they keep the portable integer sequence. Default
    /// `false`; only the AArch64 backend overrides it.
    fn is_aarch64(&self) -> bool {
        false
    }
    /// Extra bytes a *called* function must add to its 16-byte-aligned frame so
    /// the stack pointer is 16-byte aligned at its own call sites. On x86-64 the
    /// `call` instruction pushes the 8-byte return address, so a frame that is a
    /// multiple of 16 leaves rsp misaligned by 8 at the next call — libc's
    /// variadic `movaps` register-save then faults. Returns 8 for x86-64, 0 for
    /// AArch64 (the link register is a register, nothing is pushed).
    fn frame_call_padding(&self) -> usize {
        0
    }
    /// Bytes a caller must reserve at the bottom of its outgoing-argument region,
    /// below the first stack argument, that a callee is entitled to use — the
    /// Win64 32-byte "shadow"/"home" space, where a callee may spill its four
    /// register arguments `rcx`/`rdx`/`r8`/`r9` into the *caller's* frame (plan-47-B
    /// §4.3). Default 0: SysV and AAPCS64 have no such reservation, so every
    /// existing backend keeps `finalize_frame`'s current `outgoing_bytes`
    /// arithmetic and emits byte-identical frames. The Win64 backend returns 32.
    fn shadow_space_bytes(&self) -> usize {
        0
    }
    /// The offset added to every *outgoing* stack-argument slot when the frame's
    /// stack-arg sentinels are resolved — the shadow space sits below the first
    /// stack argument, so on Win64 outgoing arg 0 lives at `[rsp+32]`, not `[rsp+0]`
    /// (plan-47-B §4.3). Default 0: existing backends place outgoing arg 0 at the
    /// frame bottom, unchanged. The Win64 backend returns 32.
    fn outgoing_args_base_offset(&self) -> usize {
        0
    }
    /// The page granularity at which this target grows a thread stack on demand,
    /// or 0 if the whole stack is always addressable without probing.
    ///
    /// Windows reserves a thread's stack but commits only part of it, and extends
    /// it by *one page at a time* when an access faults on the single guard page
    /// below the committed region. A frame larger than one page that moves `rsp`
    /// down and then writes therefore steps clean over the guard page into
    /// reserved-but-uncommitted memory, and the OS raises `STATUS_ACCESS_VIOLATION`
    /// instead of extending the stack — which is why every Windows compiler emits
    /// a `__chkstk` probe for such frames. Returning a non-zero page here makes
    /// [`finalize_frame`](crate::codegen::engine::util::finalize_frame) touch each
    /// page as it allocates, so the guard is hit once per page and the stack grows
    /// normally. SysV/AAPCS64 targets grow the stack without this cooperation, so
    /// they keep the single `sub sp` and stay byte-identical.
    fn stack_probe_page_bytes(&self) -> usize {
        0
    }
}

thread_local! {
    /// The backend the current lowering thread dispatches selection + register
    /// allocation through. Installed by [`set_backend`] at each lowering entry
    /// point from `platform.backend()`; `&'static` because every backend is a
    /// zero-sized singleton.
    static ACTIVE_BACKEND: Cell<Option<&'static dyn Backend>> = const { Cell::new(None) };
}

/// Install the active backend for this lowering thread. Called from
/// `lower_module_for_platform` / `lower_module_mir_for_platform`.
pub(crate) fn set_backend(backend: &'static dyn Backend) {
    ACTIVE_BACKEND.with(|cell| cell.set(Some(backend)));
}

/// The active backend. Panics if lowering ran without [`set_backend`] — every
/// real entry point installs it; the unit tests that exercise the round trip
/// call [`select_aarch64`](crate::arch::aarch64::select::select_aarch64) directly instead of going through dispatch.
pub(crate) fn active_backend() -> &'static dyn Backend {
    ACTIVE_BACKEND
        .with(|cell| cell.get())
        .expect("active backend not set; call mir::set_backend at the lowering entry point")
}

// --- MIR capture for the `-mir` dump ------------------------------------------

thread_local! {
    /// Per-function MIR captured during a `-mir` build, keyed by function
    /// symbol, in lowering order. `None` when no `-mir` dump is in progress, so
    /// ordinary builds pay only a cheap `is_some` check per function.
    static CAPTURE: RefCell<Option<Vec<(String, Vec<MirInstruction>)>>> = const { RefCell::new(None) };
}

/// Arm MIR capture for the duration of a `-mir` lowering. Cleared by
/// [`take_capture`].
pub(crate) fn begin_capture() {
    CAPTURE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
}

/// Whether MIR capture is currently armed.
pub(crate) fn capture_enabled() -> bool {
    CAPTURE.with(|cell| cell.borrow().is_some())
}

/// Record one function's pre-allocation MIR. No-op when capture is disarmed.
pub(crate) fn capture_function(symbol: &str, instructions: Vec<MirInstruction>) {
    CAPTURE.with(|cell| {
        if let Some(captured) = cell.borrow_mut().as_mut() {
            captured.push((symbol.to_string(), instructions));
        }
    });
}

/// Disarm capture and return everything recorded since [`begin_capture`].
pub(crate) fn take_capture() -> Vec<(String, Vec<MirInstruction>)> {
    CAPTURE.with(|cell| cell.borrow_mut().take().unwrap_or_default())
}

// --- The `-mir` plan (neutral counterpart to `NativeCodePlan`) ----------------

/// One neutral relocation in the `-mir` dump (plan-00-D §1). Mirrors a
/// [`CodeRelocation`] but serializes the **neutral intent name** (`call`,
/// `data_addr_hi`, `got_load_lo`) rather than the AArch64 reloc kind — diffing a
/// `-mir` dump across targets is identical, where the `-ncode` reloc kind is not.
pub(crate) struct MirRelocation {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) intent: RelocIntent,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
}

/// One MIR function: the program/runtime metadata, the neutral op stream, and
/// the neutral relocations (intents, not AArch64 kinds). Frame data stays a
/// post-selection backend concern (the `-mir` dump is *before* selection and
/// allocation), but relocations carry their neutral *intent* and so belong in
/// the neutral view (`mir.md §8`).
pub(crate) struct MirFunction {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) returns: String,
    pub(crate) params: Vec<CodeParam>,
    pub(crate) instructions: Vec<MirInstruction>,
    pub(crate) relocations: Vec<MirRelocation>,
}

/// The whole-module MIR — the `-mir` output. Deliberately ISA-independent:
/// no `target`/`arch` (diff it across targets and it is identical, unlike
/// `-ncode`).
pub(crate) struct MirPlan {
    pub(crate) project: String,
    pub(crate) entry_symbol: Option<String>,
    pub(crate) functions: Vec<MirFunction>,
}

impl MirPlan {
    pub(crate) fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-mir\",\n",
                "  \"version\": 1,\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.project),
            self.entry_symbol
                .as_ref()
                .map(|symbol| json_string(symbol))
                .unwrap_or_else(|| "null".to_string()),
            join_json(&self.functions, 2)
        )
    }
}

/// Assemble the [`MirPlan`] from a finished [`NativeCodePlan`] and the per-
/// function MIR captured during its lowering. Functions that went through the
/// virtual-register builder appear with their **pre-allocation** MIR (virtual
/// registers, `%vN`/`%fN`); functions not yet ported to the MIR path — the
/// hand-written AArch64 runtime helpers (`mir.md §9`, ported in plan-00-B) —
/// are shown as their 1:1 MIR over the final (physical-register) stream, so the
/// dump is complete and honest about what is neutral versus not.
pub(crate) fn build_mir_plan(
    plan: &NativeCodePlan,
    captured: Vec<(String, Vec<MirInstruction>)>,
) -> MirPlan {
    let mut by_symbol: HashMap<String, Vec<MirInstruction>> = captured.into_iter().collect();
    let functions = plan
        .functions
        .iter()
        .map(|function| {
            let instructions = by_symbol
                .remove(&function.symbol)
                .unwrap_or_else(|| lower_to_mir(&function.instructions));
            MirFunction {
                name: function.name.clone(),
                symbol: function.symbol.clone(),
                returns: function.returns.clone(),
                params: function
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.clone(),
                        type_: param.type_.clone(),
                        location: param.location.clone(),
                    })
                    .collect(),
                instructions,
                relocations: function
                    .relocations
                    .iter()
                    .map(|relocation| MirRelocation {
                        from: relocation.from.clone(),
                        to: relocation.to.clone(),
                        intent: relocation.kind,
                        binding: relocation.binding.clone(),
                        library: relocation.library.clone(),
                    })
                    .collect(),
            }
        })
        .collect();
    MirPlan {
        project: plan.project.clone(),
        entry_symbol: plan.entry_symbol.clone(),
        functions,
    }
}

impl ToCodeJson for MirInstruction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut fields = vec![format!("\"op\": {}", json_string(self.op.mnemonic()))];
        fields.extend(
            self.fields
                .iter()
                .map(|(name, value)| format!("\"{name}\": {}", json_string(&value.render()))),
        );
        format!("\n{}{{ {} }}", pad, fields.join(", "))
    }
}

impl ToCodeJson for MirFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"instructions\": [{}\n{}  ],\n",
                "{}  \"relocations\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            json_string(&self.returns),
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            join_json(&self.instructions, indent + 2),
            pad,
            pad,
            join_json(&self.relocations, indent + 2),
            pad,
            pad
        )
    }
}

impl ToCodeJson for MirRelocation {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let library = self
            .library
            .as_ref()
            .map(|library| json_string(library))
            .unwrap_or_else(|| "null".to_string());
        // Neutral intent name — never an AArch64 reloc kind (validation §5).
        format!(
            "\n{}{{ \"from\": {}, \"to\": {}, \"intent\": {}, \"binding\": {}, \"library\": {} }}",
            pad,
            json_string(&self.from),
            json_string(&self.to),
            json_string(self.intent.name()),
            json_string(&self.binding),
            library
        )
    }
}

#[cfg(test)]
mod tests;
