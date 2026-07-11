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

use std::cell::{Cell, RefCell};

use crate::target::shared::regmodel::RegisterModel;

use super::*;

/// One MIR instruction: a neutral op plus the same ordered string-field bag as
/// [`CodeInstruction`] (`mir.md §10`). The allocator's field-based liveness
/// keeps working unchanged because the shape is identical.
pub(crate) struct MirInstruction {
    pub(crate) op: MirOp,
    pub(crate) fields: Vec<(&'static str, String)>,
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
        LdrU64, LdrU32, LdrU16, LdrU8, StrU64, StrU32, StrU8, LdrD, StrD, Adrp, AddPageOff,
        FMovDFromD, FAddD, FSubD, FMulD, FDivD, FMinnmD, FMaxnmD, FNegD, FAbsD, FSqrtD, FCmpD,
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
pub(crate) fn rename_field_values(fields: &mut [(&'static str, String)], from: &str, to: &str) {
    for (_, value) in fields.iter_mut() {
        if value == from {
            *value = to.to_string();
        }
    }
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
    // Build a fused op from a flag-setter's operands + a branch. `shared` marks
    // a branch that reuses the preceding comparison (see [`FUSED_SHARE_FIELD`]).
    fn fuse(
        op: MirOp,
        setter_fields: &[(&'static str, String)],
        branch: &CodeInstruction,
        shared: bool,
    ) -> MirInstruction {
        let mut fields = setter_fields.to_vec();
        fields.push((FUSED_COND_FIELD, branch.op.mnemonic().to_string()));
        fields.extend(branch.fields.iter().cloned());
        if shared {
            fields.push((FUSED_SHARE_FIELD, "true".to_string()));
        }
        MirInstruction { op, fields }
    }

    // Fuse an `adrp; add_pageoff` page pair into one `addr_of`, or `None` if the
    // two do not form a single same-register symbol-address sequence.
    fn fuse_addr_of(adrp: &CodeInstruction, add: &CodeInstruction) -> Option<MirInstruction> {
        if add.op != CodeOp::AddPageOff {
            return None;
        }
        let dst = adrp.get("dst")?;
        let symbol = adrp.get("symbol")?;
        if add.get("dst") == Some(dst)
            && add.get("src") == Some(dst)
            && add.get("symbol") == Some(symbol)
        {
            // Carry the `adrp` field bag (`[dst, symbol]`); the expansion
            // reconstructs `add_pageoff`'s `src == dst` from it.
            Some(MirInstruction {
                op: MirOp::AddrOf,
                fields: adrp.fields.clone(),
            })
        } else {
            None
        }
    }

    let mut out = Vec::with_capacity(instructions.len());
    let mut i = 0;
    while i < instructions.len() {
        let setter = &instructions[i];
        // `addr_of` fusion (plan-00-C): a symbol-address `adrp <dst>, <sym>;
        // add_pageoff <dst>, <dst>, <sym>` page pair is one neutral PC-relative
        // address op. The two are always emitted adjacently with `src == dst`
        // (every builder/helper site goes through `abi::load_page_address` +
        // `abi::add_page_offset` on the same register), so the fused op carries
        // just `[dst, symbol]` and `select_aarch64` rebuilds the pair exactly.
        if setter.op == CodeOp::Adrp {
            if let Some(add) = instructions.get(i + 1) {
                if let Some(addr_of) = fuse_addr_of(setter, add) {
                    out.push(addr_of);
                    i += 2;
                    continue;
                }
            }
        }
        if let Some(fused_op) = fused_variant(setter.op) {
            if instructions
                .get(i + 1)
                .is_some_and(|next| is_flag_reading_branch(next.op))
            {
                // The first branch owns the comparison.
                out.push(fuse(fused_op, &setter.fields, &instructions[i + 1], false));
                i += 2;
                // Any further consecutive flag-reading branches read the *same*
                // flags (nothing reset them), so they share this comparison —
                // the 3-way `cmp; b.lo; b.hi` ordering, etc.
                while instructions
                    .get(i)
                    .is_some_and(|next| is_flag_reading_branch(next.op))
                {
                    out.push(fuse(fused_op, &setter.fields, &instructions[i], true));
                    i += 1;
                }
                continue;
            }
        }
        out.push(MirInstruction {
            op: MirOp::from_code(setter.op),
            fields: setter.fields.clone(),
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
    let neutral = lower_to_mir(&function.instructions);
    function.instructions = active_backend().select(&neutral);
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
    /// Select neutral MIR into this ISA's machine instructions.
    fn select(&self, neutral: &[MirInstruction]) -> Vec<CodeInstruction>;
    /// The register model the shared allocator colors vregs against.
    fn register_model(&self) -> &'static dyn RegisterModel;
    /// Extra bytes a *called* function must add to its 16-byte-aligned frame so
    /// the stack pointer is 16-byte aligned at its own call sites. On x86-64 the
    /// `call` instruction pushes the 8-byte return address, so a frame that is a
    /// multiple of 16 leaves rsp misaligned by 8 at the next call — libc's
    /// variadic `movaps` register-save then faults. Returns 8 for x86-64, 0 for
    /// AArch64 (the link register is a register, nothing is pushed).
    fn frame_call_padding(&self) -> usize {
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
                .map(|(name, value)| format!("\"{name}\": {}", json_string(value))),
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
mod tests {
    use super::*;

    fn assert_round_trips(original: &[CodeInstruction]) {
        let round_tripped = crate::arch::aarch64::select::select_aarch64(&lower_to_mir(original));
        assert_eq!(
            round_tripped.len(),
            original.len(),
            "expand∘fuse changed instruction count"
        );
        for (after, before) in round_tripped.iter().zip(original.iter()) {
            assert_eq!(after.op, before.op);
            assert_eq!(after.fields, before.fields);
        }
    }

    /// Every `CodeOp` round-trips through `MirOp` unchanged — the property the
    /// byte-identical gate rests on. `from_code`/`to_code` are exhaustive
    /// matches, so a coverage gap is a compile error rather than a test miss.
    #[test]
    fn code_op_round_trips_through_mir() {
        // Mirror ops: the MIR op carries the same mnemonic as its `CodeOp`.
        for op in [
            CodeOp::Label,
            CodeOp::Mov,
            CodeOp::Add,
            CodeOp::BranchVs,
            CodeOp::BranchMi,
            CodeOp::Ret,
            CodeOp::LdrD,
            CodeOp::FMaddD, // scalar fused multiply-add — stays a mirror op
            CodeOp::Clz,    // already-neutral exotic int op — stays a mirror
            CodeOp::Rbit,   // (clz/rbit/msub keep their semantic AArch64 name)
            CodeOp::MSub,
        ] {
            assert_eq!(MirOp::from_code(op).to_code(), Some(op));
            assert_eq!(MirOp::from_code(op).mnemonic(), op.mnemonic());
        }
    }

    /// Renamed ops (plan-00-C Phases 3 & 4) still convert 1:1 to their `CodeOp`
    /// — so selection/encoding stay byte-identical — but the MIR mnemonic is the
    /// ISA-neutral semantic name, *not* the AArch64 instruction. This is the
    /// `mir.md §4` / validation §5 requirement: no `smulh`/`umulh`/`rorv`/`adc`/
    /// `rev`/`fcvt*`/`scvtf` mnemonics in the MIR.
    #[test]
    fn renamed_ops_are_neutral_but_select_back_identically() {
        for (code, neutral) in [
            (CodeOp::SMulH, "mulhi_s"),
            (CodeOp::UMulH, "mulhi_u"),
            (CodeOp::AddCarry, "addc"),
            (CodeOp::SubBorrow, "subc"),
            (CodeOp::Rorv, "rotr"),
            (CodeOp::RorvW, "rotr_w"),
            (CodeOp::RevW, "bswap_w"),
            (CodeOp::RevX, "bswap"),
            (CodeOp::FCvtzsXFromD, "f2i_trunc"),
            (CodeOp::FCvtmsXFromD, "f2i_floor"),
            (CodeOp::FCvtpsXFromD, "f2i_ceil"),
            (CodeOp::FCvtasXFromD, "f2i_nearest"),
            (CodeOp::SCvtfDFromX, "i2f"),
            (CodeOp::FMovDFromX, "fmov_i2f"),
            (CodeOp::FMovXFromD, "fmov_f2i"),
            // §7 machine-y call/syscall vocabulary (plan-00-F): the helpers'
            // `bl`/`blr`/`svc` neutralize so the helper MIR has no AArch64 op.
            (CodeOp::BranchLink, "call"),
            (CodeOp::BranchLinkRegister, "call_indirect"),
            (CodeOp::Svc, "syscall"),
        ] {
            let mir = MirOp::from_code(code);
            // 1:1 selection back to the same CodeOp — byte-identical encoding.
            assert_eq!(mir.to_code(), Some(code));
            // Neutral MIR mnemonic, and it no longer names the AArch64 op.
            assert_eq!(mir.mnemonic(), neutral);
            assert_ne!(mir.mnemonic(), code.mnemonic());
        }
    }

    /// The whole `v128` SIMD vocabulary (plan-00-E): each NEON `CodeOp` maps 1:1
    /// to its `MirOp` (so selection/encoding stay byte-identical) but carries a
    /// neutral `v128.*` mnemonic — no `*_v` NEON mnemonic survives in the MIR
    /// (`mir.md §6`, validation §5). Covers every renamed NEON op exhaustively.
    #[test]
    fn v128_ops_are_neutral_but_select_back_identically() {
        let cases = [
            (CodeOp::LdrQ, "v128.load"),
            (CodeOp::StrQ, "v128.store"),
            (CodeOp::FAddV, "v128.fadd"),
            (CodeOp::FSubV, "v128.fsub"),
            (CodeOp::FMulV, "v128.fmul"),
            (CodeOp::FDivV, "v128.fdiv"),
            (CodeOp::FMlaV, "v128.fma"),
            (CodeOp::FMlsV, "v128.fms"),
            (CodeOp::FMinV, "v128.fmin"),
            (CodeOp::FMaxV, "v128.fmax"),
            (CodeOp::FCmGtV, "v128.fcmp_gt"),
            (CodeOp::FCmGeV, "v128.fcmp_ge"),
            (CodeOp::FCmEqV, "v128.fcmp_eq"),
            (CodeOp::FAbsV, "v128.fabs"),
            (CodeOp::FNegV, "v128.fneg"),
            (CodeOp::FSqrtV, "v128.fsqrt"),
            (CodeOp::FRintpV, "v128.fround_ceil"),
            (CodeOp::FRintmV, "v128.fround_floor"),
            (CodeOp::FRintaV, "v128.fround_nearest"),
            (CodeOp::FRintnV, "v128.fround_even"),
            (CodeOp::FRintzV, "v128.fround_trunc"),
            (CodeOp::FCvtzsV, "v128.f2i_trunc"),
            (CodeOp::FCvtasV, "v128.f2i_nearest"),
            (CodeOp::ScvtfV, "v128.i2f"),
            (CodeOp::FCmGtZeroV, "v128.fcmp_gt_zero"),
            (CodeOp::FCmGeZeroV, "v128.fcmp_ge_zero"),
            (CodeOp::FCmEqZeroV, "v128.fcmp_eq_zero"),
            (CodeOp::FCmLtZeroV, "v128.fcmp_lt_zero"),
            (CodeOp::FCmLeZeroV, "v128.fcmp_le_zero"),
            (CodeOp::AddV, "v128.add"),
            (CodeOp::SubV, "v128.sub"),
            (CodeOp::CmGtV, "v128.icmp_gt"),
            (CodeOp::CmGeV, "v128.icmp_ge"),
            (CodeOp::CmEqV, "v128.icmp_eq"),
            (CodeOp::SshlV, "v128.sshl"),
            (CodeOp::UshlV, "v128.ushl"),
            (CodeOp::NegV, "v128.neg"),
            (CodeOp::AbsV, "v128.abs"),
            (CodeOp::AndV, "v128.and"),
            (CodeOp::OrrV, "v128.or"),
            (CodeOp::EorV, "v128.xor"),
            (CodeOp::BslV, "v128.bsl"),
            (CodeOp::BitV, "v128.bit"),
            (CodeOp::ShlV, "v128.shl_imm"),
            (CodeOp::SshrV, "v128.sshr_imm"),
            (CodeOp::UshrV, "v128.ushr_imm"),
            (CodeOp::DupVFromX, "v128.dup_from_gpr"),
            (CodeOp::UmovXFromV, "v128.umov_to_gpr"),
        ];
        for (code, neutral) in cases {
            let mir = MirOp::from_code(code);
            assert_eq!(mir.to_code(), Some(code), "v128 op must select 1:1");
            assert_eq!(mir.mnemonic(), neutral);
            assert!(mir.is_v128(), "{neutral} should be classed as a v128 op");
            // Neutral name: in the `v128.` namespace, never the AArch64 `*_v`.
            assert!(mir.mnemonic().starts_with("v128."));
            assert_ne!(mir.mnemonic(), code.mnemonic());
            assert!(!mir.mnemonic().ends_with("_v"));
        }
    }

    /// **The `v128` lane-semantics contract** (plan-00-E Phase 4, `mir.md §6`).
    ///
    /// This is the contract the x86_64 (SSE2+FMA3+SSE4.1) and rv64 (scalarized)
    /// backends are judged against — the silent-bug surface where a wrong lane
    /// op breaks the ≤1-ULP kernels without a crash. On AArch64 it is realized
    /// by the (unchanged) NEON encoder; pinning it here makes it an executable
    /// acceptance the new backends must reproduce. Each entry fixes the op's
    /// **lane shape** and its **edge-case semantics** (the parts that genuinely
    /// differ across ISAs); the executable golden vectors are the runtime ULP
    /// harness (`tools/math-kernels/runtime_ulp.py`, the transcendental kernels)
    /// and the `vector::` / `math::`-array acceptance fixtures, which exercise
    /// these ops observably and round-trip unchanged through the MIR.
    #[test]
    fn v128_lane_semantics_contract() {
        // (op, lane shape, the cross-ISA-significant semantic to reproduce)
        let contract = [
            (CodeOp::LdrQ, "mem128", "load 16 bytes, no lane interpretation"),
            (CodeOp::StrQ, "mem128", "store 16 bytes, no lane interpretation"),
            (CodeOp::FAddV, "2xf64", "IEEE add per lane"),
            (CodeOp::FSubV, "2xf64", "IEEE sub per lane"),
            (CodeOp::FMulV, "2xf64", "IEEE mul per lane"),
            (CodeOp::FDivV, "2xf64", "IEEE div per lane"),
            (CodeOp::FMlaV, "2xf64", "fused dst += lhs*rhs, single rounding (needs x86 FMA3)"),
            (CodeOp::FMlsV, "2xf64", "fused dst -= lhs*rhs, single rounding (needs x86 FMA3)"),
            (CodeOp::FMinV, "2xf64", "NaN-PROPAGATING min (NEON fmin); x86 must match minpd+NaN fixup, NOT bare minpd"),
            (CodeOp::FMaxV, "2xf64", "NaN-PROPAGATING max; x86 maxpd+NaN fixup"),
            (CodeOp::FCmGtV, "2xf64", "lane mask: all-ones if a>b (ordered, NaN→0), else all-zeros"),
            (CodeOp::FCmGeV, "2xf64", "lane mask: all-ones if a>=b (ordered)"),
            (CodeOp::FCmEqV, "2xf64", "lane mask: all-ones if a==b (ordered)"),
            (CodeOp::FAbsV, "2xf64", "clear sign bit per lane"),
            (CodeOp::FNegV, "2xf64", "flip sign bit per lane"),
            (CodeOp::FSqrtV, "2xf64", "IEEE sqrt per lane"),
            (CodeOp::FRintpV, "2xf64", "round to integral, toward +inf"),
            (CodeOp::FRintmV, "2xf64", "round to integral, toward -inf"),
            (CodeOp::FRintaV, "2xf64", "round to integral, nearest, ties AWAY from zero"),
            (CodeOp::FRintnV, "2xf64", "round to integral, nearest, ties to EVEN (x86 roundpd mode 0)"),
            (CodeOp::FRintzV, "2xf64", "round to integral, toward zero (truncate)"),
            (CodeOp::FCvtzsV, "f64->i64 x2", "convert toward zero, saturating to i64"),
            (CodeOp::FCvtasV, "f64->i64 x2", "convert nearest ties-away, saturating to i64"),
            (CodeOp::ScvtfV, "i64->f64 x2", "signed i64 → f64 per lane"),
            (CodeOp::FCmGtZeroV, "2xf64", "lane mask vs +0.0: all-ones if a>0"),
            (CodeOp::FCmGeZeroV, "2xf64", "lane mask vs +0.0: all-ones if a>=0"),
            (CodeOp::FCmEqZeroV, "2xf64", "lane mask vs +0.0: all-ones if a==0 (±0 both match)"),
            (CodeOp::FCmLtZeroV, "2xf64", "lane mask vs +0.0: all-ones if a<0"),
            (CodeOp::FCmLeZeroV, "2xf64", "lane mask vs +0.0: all-ones if a<=0"),
            (CodeOp::AddV, "2xi64", "wrapping add per lane"),
            (CodeOp::SubV, "2xi64", "wrapping sub per lane"),
            (CodeOp::CmGtV, "2xi64", "lane mask: all-ones if signed a>b"),
            (CodeOp::CmGeV, "2xi64", "lane mask: all-ones if signed a>=b"),
            (CodeOp::CmEqV, "2xi64", "lane mask: all-ones if a==b"),
            (CodeOp::SshlV, "2xi64", "signed variable shift: rhs>=0 left, rhs<0 arithmetic right"),
            (CodeOp::UshlV, "2xi64", "unsigned variable shift: rhs>=0 left, rhs<0 logical right"),
            (CodeOp::NegV, "2xi64", "two's-complement negate per lane"),
            (CodeOp::AbsV, "2xi64", "absolute value per lane"),
            (CodeOp::AndV, "16xi8", "bitwise and over all 128 bits"),
            (CodeOp::OrrV, "16xi8", "bitwise or over all 128 bits"),
            (CodeOp::EorV, "16xi8", "bitwise xor over all 128 bits"),
            (CodeOp::BslV, "16xi8", "bit-select: result = (dst & a) | (~dst & b); MASK lives in dst (≠ x86 blendv sign-bit)"),
            (CodeOp::BitV, "16xi8", "bit-insert-if-true: dst = (dst & ~mask) | (lhs & mask), mask in rhs"),
            (CodeOp::ShlV, "2xi64", "shift-left by immediate per lane"),
            (CodeOp::SshrV, "2xi64", "arithmetic shift-right by immediate per lane"),
            (CodeOp::UshrV, "2xi64", "logical shift-right by immediate per lane"),
            (CodeOp::DupVFromX, "i64->2xi64", "broadcast a GPR into both lanes"),
            (CodeOp::UmovXFromV, "lane->i64", "extract one lane into a GPR (zero-extend)"),
        ];
        // Completeness: exactly one contract row per `v128` op, and every row is
        // a v128 op with a `2xf64`/`2xi64`/`16xi8`/conversion/mem lane shape. The
        // count must match the neutrality test's 48-op coverage, so a newly added
        // v128 op cannot slip in without a pinned lane-semantics contract.
        assert_eq!(
            contract.len(),
            48,
            "every v128 op needs a lane-semantics contract row"
        );
        let valid_shapes = [
            "mem128",
            "2xf64",
            "2xi64",
            "16xi8",
            "f64->i64 x2",
            "i64->f64 x2",
            "i64->2xi64",
            "lane->i64",
        ];
        for (op, shape, semantics) in contract {
            let mir = MirOp::from_code(op);
            assert!(mir.is_v128(), "{} must be a v128 op", mir.mnemonic());
            assert!(
                valid_shapes.contains(&shape),
                "{}: unknown lane shape {shape}",
                mir.mnemonic()
            );
            assert!(!semantics.is_empty(), "{}: empty contract", mir.mnemonic());
        }
        // No two rows name the same op (the table is a function, total over v128).
        let mut seen = std::collections::HashSet::new();
        for (op, _, _) in contract {
            assert!(
                seen.insert(op.mnemonic()),
                "duplicate contract row for {}",
                op.mnemonic()
            );
        }
    }

    /// `select_aarch64 ∘ lower_to_mir` is the identity on a non-fusing stream.
    #[test]
    fn lower_then_select_is_identity() {
        assert_round_trips(&[
            CodeInstruction::new("mov")
                .field("dst", "%v0")
                .field("src", "x0"),
            CodeInstruction::new("add")
                .field("dst", "%v1")
                .field("lhs", "%v0")
                .field("rhs", "%v0"),
            CodeInstruction::new("ret"),
        ]);
    }

    /// A flag-setter + flag-reading branch fuses to one flagless op and expands
    /// back byte-for-byte: integer compare-and-branch (incl. an extra `reason`
    /// field), float compare-and-branch, compare-vs-zero, and overflow arith.
    #[test]
    fn flag_pairs_fuse_and_expand_identically() {
        // cmp_imm; b.eq (carries the debug `reason` field)
        assert_round_trips(&[
            CodeInstruction::new("cmp_imm")
                .field("lhs", "x0")
                .field("rhs", "2"),
            CodeInstruction::new("b.eq")
                .field("target", "if_else_0")
                .field("reason", "ifFalse"),
        ]);
        // cmp; b.lt (register compare)
        assert_round_trips(&[
            CodeInstruction::new("cmp")
                .field("lhs", "%v0")
                .field("rhs", "%v1"),
            CodeInstruction::new("b.lt").field("target", "L0"),
        ]);
        // adds; b.vc (integer add overflow trap)
        assert_round_trips(&[
            CodeInstruction::new("adds")
                .field("dst", "%v6")
                .field("lhs", "%v4")
                .field("rhs", "%v5"),
            CodeInstruction::new("b.vc").field("target", "overflow_ok_2"),
        ]);
        // fcmp_d; b.mi (IEEE float `<`, plan-17) and fcmp_d; b.vs (finiteness)
        assert_round_trips(&[
            CodeInstruction::new("fcmp_d")
                .field("lhs", "%f0")
                .field("rhs", "%f1"),
            CodeInstruction::new("b.mi").field("target", "Lt"),
        ]);
        assert_round_trips(&[
            CodeInstruction::new("fcmp_d")
                .field("lhs", "%f0")
                .field("rhs", "%f1"),
            CodeInstruction::new("b.vs").field("target", "Lnan"),
        ]);
        // fcmp_zero_d; b.ge (compare vs zero)
        assert_round_trips(&[
            CodeInstruction::new("fcmp_zero_d").field("src", "%f2"),
            CodeInstruction::new("b.ge").field("target", "Lge"),
        ]);
    }

    /// The fused op is genuinely flagless: no `cmp`/`b.cc` remains in the MIR.
    #[test]
    fn fused_op_carries_operands_and_condition() {
        let mir = lower_to_mir(&[
            CodeInstruction::new("cmp_imm")
                .field("lhs", "x0")
                .field("rhs", "255"),
            CodeInstruction::new("b.hi").field("target", "range_err"),
        ]);
        assert_eq!(mir.len(), 1);
        assert_eq!(mir[0].op, MirOp::BrCcImm);
        assert_eq!(mir[0].op.mnemonic(), "br_cc_imm");
        let get = |k: &str| {
            mir[0]
                .fields
                .iter()
                .find(|(f, _)| *f == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("lhs"), Some("x0"));
        assert_eq!(get("rhs"), Some("255"));
        assert_eq!(get("cond"), Some("b.hi"));
        assert_eq!(get("target"), Some("range_err"));
    }

    /// The macOS syscall error idiom `svc; b.<carry>` fuses into the flagless
    /// `syscall_br` and expands back byte-for-byte (plan-00-F) — the last
    /// flag-reading branch in the helper MIR. A `svc` *not* followed by a flag
    /// branch (e.g. the exit syscall before `branch_self`) stays the neutral
    /// `syscall` op.
    #[test]
    fn syscall_carry_check_fuses_and_expands_identically() {
        // svc; b.lo (carry-clear → error path) fuses to one flagless op.
        let checked = [
            CodeInstruction::new("svc"),
            CodeInstruction::new("b.lo").field("target", "encoding_error"),
        ];
        let mir = lower_to_mir(&checked);
        assert_eq!(mir.len(), 1);
        assert_eq!(mir[0].op, MirOp::SyscallBr);
        assert_eq!(mir[0].op.mnemonic(), "syscall_br");
        let get = |k: &str| {
            mir[0]
                .fields
                .iter()
                .find(|(f, _)| *f == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("cond"), Some("b.lo"));
        assert_eq!(get("target"), Some("encoding_error"));
        // No `svc`/`b.lo` mnemonic survives — the MIR is flagless.
        assert!(!mir
            .iter()
            .any(|m| matches!(m.op, MirOp::Syscall | MirOp::BranchLo)));
        assert_round_trips(&checked);

        // A bare syscall (no following flag branch) stays the neutral `syscall`.
        let bare = [
            CodeInstruction::new("svc"),
            CodeInstruction::new("branch_self"),
        ];
        let mir = lower_to_mir(&bare);
        assert_eq!(mir.len(), 2);
        assert_eq!(mir[0].op, MirOp::Syscall);
        assert_eq!(mir[0].op.mnemonic(), "syscall");
        assert_round_trips(&bare);
    }

    /// The explicit-carry 128-bit add chain (plan-00-G §4): each limb is a single
    /// `add_carry` op whose carry-in/out are *values*, so nothing fuses and the
    /// carry survives register allocation. The op neutral-renames to `addc` and
    /// round-trips 1:1. A plain `adds` not followed by a flag-branch stays a
    /// mirror op (it is not mistaken for an overflow fusion).
    #[test]
    fn explicit_carry_chain_round_trips() {
        let original = [
            // lo = a_lo + b_lo, carry → x15
            CodeInstruction::new("add_carry")
                .field("dst", "x9")
                .field("carry_out", "x15")
                .field("lhs", "x13")
                .field("rhs", "x11")
                .field("carry_in", "xzr"),
            // hi = a_hi + b_hi + x15, carry-out discarded
            CodeInstruction::new("add_carry")
                .field("dst", "x10")
                .field("carry_out", "xzr")
                .field("lhs", "x14")
                .field("rhs", "x12")
                .field("carry_in", "x15"),
        ];
        let mir = lower_to_mir(&original);
        assert_eq!(mir.len(), 2);
        assert_eq!(mir[0].op, MirOp::AddC);
        assert_eq!(mir[0].op.mnemonic(), "addc");
        assert_round_trips(&original);

        // A lone `adds` (no following flag-branch) stays a mirror op.
        let lone_adds = [
            CodeInstruction::new("adds")
                .field("dst", "x9")
                .field("lhs", "x9")
                .field("rhs", "x1"),
            CodeInstruction::new("mov")
                .field("dst", "x10")
                .field("src", "x9"),
        ];
        let mir = lower_to_mir(&lone_adds);
        assert_eq!(mir[0].op, MirOp::Adds);
        assert_round_trips(&lone_adds);
    }

    /// A 3-way `cmp; b.lo; b.hi` (the string-ordering pattern) becomes two
    /// flagless ops: the first owns the compare, the second shares it. Both are
    /// self-contained (carry operands), and they expand back to the single
    /// shared `cmp` byte-for-byte.
    #[test]
    fn multi_branch_compare_fuses_with_share() {
        let original = [
            CodeInstruction::new("cmp")
                .field("lhs", "x16")
                .field("rhs", "x11"),
            CodeInstruction::new("b.lo").field("target", "less"),
            CodeInstruction::new("b.hi").field("target", "greater"),
        ];
        let mir = lower_to_mir(&original);
        assert_eq!(mir.len(), 2);
        assert_eq!(mir[0].op, MirOp::BrCc); // owns the compare
        assert_eq!(mir[1].op, MirOp::BrCc); // shares it — still flagless
        let shared = |m: &MirInstruction| m.fields.iter().any(|(k, _)| *k == "share");
        assert!(!shared(&mir[0]));
        assert!(shared(&mir[1]));
        // the shared branch still carries the compare operands (self-contained)
        fn get<'a>(m: &'a MirInstruction, k: &str) -> Option<&'a str> {
            m.fields
                .iter()
                .find(|(f, _)| *f == k)
                .map(|(_, v)| v.as_str())
        }
        assert_eq!(get(&mir[1], "lhs"), Some("x16"));
        assert_eq!(get(&mir[1], "rhs"), Some("x11"));
        assert_eq!(get(&mir[1], "cond"), Some("b.hi"));
        assert_round_trips(&original);
    }

    /// An `adrp; add_pageoff` page pair fuses into one neutral `addr_of` and
    /// expands back to the exact same two instructions (plan-00-C Phase 1). The
    /// fused op carries only `[dst, symbol]`; the expansion reconstructs
    /// `add_pageoff`'s `src == dst`.
    #[test]
    fn addr_of_fuses_and_expands_identically() {
        let original = [
            CodeInstruction::new("adrp")
                .field("dst", "x9")
                .field("symbol", "str.0"),
            CodeInstruction::new("add_pageoff")
                .field("dst", "x9")
                .field("src", "x9")
                .field("symbol", "str.0"),
        ];
        let mir = lower_to_mir(&original);
        assert_eq!(mir.len(), 1);
        assert_eq!(mir[0].op, MirOp::AddrOf);
        assert_eq!(mir[0].op.mnemonic(), "addr_of");
        let get = |k: &str| {
            mir[0]
                .fields
                .iter()
                .find(|(f, _)| *f == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("dst"), Some("x9"));
        assert_eq!(get("symbol"), Some("str.0"));
        // No `adrp`/`add_pageoff` mnemonic survives in the MIR (validation §5).
        assert!(!mir
            .iter()
            .any(|m| matches!(m.op, MirOp::Adrp | MirOp::AddPageOff)));
        assert_round_trips(&original);
    }

    /// A lone `adrp` (no following `add_pageoff`), or a pair whose registers do
    /// not line up, is *not* fused — it stays a mirror op so the stream still
    /// round-trips. Guards against an over-eager fusion mistaking an unrelated
    /// page load for the address-of sequence.
    #[test]
    fn unpaired_or_mismatched_adrp_is_not_fused() {
        // Lone adrp followed by an unrelated op.
        let lone = [
            CodeInstruction::new("adrp")
                .field("dst", "x9")
                .field("symbol", "g"),
            CodeInstruction::new("ret"),
        ];
        let mir = lower_to_mir(&lone);
        assert_eq!(mir[0].op, MirOp::Adrp);
        assert_round_trips(&lone);
        // add_pageoff on a *different* register than the adrp — not an addr_of.
        let mismatch = [
            CodeInstruction::new("adrp")
                .field("dst", "x9")
                .field("symbol", "g"),
            CodeInstruction::new("add_pageoff")
                .field("dst", "x10")
                .field("src", "x10")
                .field("symbol", "g"),
        ];
        let mir = lower_to_mir(&mismatch);
        assert_eq!(mir.len(), 2);
        assert_eq!(mir[0].op, MirOp::Adrp);
        assert_eq!(mir[1].op, MirOp::AddPageOff);
        assert_round_trips(&mismatch);
    }

    /// The round-trip identity, in miniature: a 39-fixture sweep with at least
    /// one instruction from **every** builder op family — moves & immediates,
    /// the universal ALU, the neutral-renamed exotic integer ops, the structural
    /// `addr_of` pair, every load/store width (one against the abstract
    /// `arena_base`, plan-00-D), scalar float arith + the renamed float↔int
    /// conversions & bit-reinterprets, the NEON `v128` ops, the machine-y
    /// call/syscall vocabulary (plan-00-F), and the fused flagless control-flow
    /// ops. `select_aarch64 ∘ lower_to_mir` must be the identity on the whole
    /// stream — the property that lets the MIR be the sole code path (plan-00-G)
    /// without perturbing the AArch64 output.
    #[test]
    fn round_trip_sweep_over_every_op_family() {
        let fixtures: [CodeInstruction; 39] = [
            // — moves & immediates —
            CodeInstruction::new("mov")
                .field("dst", "%v0")
                .field("src", "x1"),
            CodeInstruction::new("mov_imm")
                .field("dst", "%v1")
                .field("value", "4294967296"),
            // — universal ALU (incl. the immediate forms that keep small imms) —
            CodeInstruction::new("add")
                .field("dst", "%v2")
                .field("lhs", "%v0")
                .field("rhs", "%v1"),
            CodeInstruction::new("sub")
                .field("dst", "%v3")
                .field("lhs", "%v2")
                .field("rhs", "%v0"),
            CodeInstruction::new("mul")
                .field("dst", "%v4")
                .field("lhs", "%v2")
                .field("rhs", "%v3"),
            CodeInstruction::new("and")
                .field("dst", "%v5")
                .field("lhs", "%v4")
                .field("rhs", "%v0"),
            CodeInstruction::new("orr")
                .field("dst", "%v6")
                .field("lhs", "%v5")
                .field("rhs", "%v1"),
            CodeInstruction::new("eor")
                .field("dst", "%v7")
                .field("lhs", "%v6")
                .field("rhs", "%v2"),
            CodeInstruction::new("mvn")
                .field("dst", "%v8")
                .field("src", "%v7"),
            CodeInstruction::new("sdiv")
                .field("dst", "%v9")
                .field("lhs", "%v8")
                .field("rhs", "%v0"),
            CodeInstruction::new("udiv")
                .field("dst", "%v10")
                .field("lhs", "%v9")
                .field("rhs", "%v1"),
            CodeInstruction::new("lslv")
                .field("dst", "%v11")
                .field("lhs", "%v10")
                .field("rhs", "%v0"),
            CodeInstruction::new("add_imm")
                .field("dst", "%v12")
                .field("src", "%v11")
                .field("imm", "8"),
            CodeInstruction::new("lsl_imm")
                .field("dst", "%v13")
                .field("src", "%v12")
                .field("shift", "3"),
            // — neutral-renamed "exotic" integer ops (Phase 3) —
            CodeInstruction::new("smulh")
                .field("dst", "%v14")
                .field("lhs", "%v0")
                .field("rhs", "%v1"),
            CodeInstruction::new("umulh")
                .field("dst", "%v15")
                .field("lhs", "%v0")
                .field("rhs", "%v1"),
            CodeInstruction::new("add_carry")
                .field("dst", "%v16")
                .field("carry_out", "%v26")
                .field("lhs", "%v14")
                .field("rhs", "%v15")
                .field("carry_in", "xzr"),
            CodeInstruction::new("rorv")
                .field("dst", "%v17")
                .field("lhs", "%v16")
                .field("rhs", "%v0"),
            CodeInstruction::new("rev_x")
                .field("dst", "%v18")
                .field("src", "%v17"),
            CodeInstruction::new("clz")
                .field("dst", "%v19")
                .field("src", "%v18"),
            CodeInstruction::new("msub")
                .field("dst", "%v20")
                .field("lhs", "%v0")
                .field("rhs", "%v1")
                .field("minuend", "%v2"),
            // — structural addr_of page pair (Phase 1): fuses to one op —
            CodeInstruction::new("adrp")
                .field("dst", "%v21")
                .field("symbol", "pool"),
            CodeInstruction::new("add_pageoff")
                .field("dst", "%v21")
                .field("src", "%v21")
                .field("symbol", "pool"),
            // — loads/stores, every width; the u8 store addresses the abstract
            //   arena base (the pinned x19), exercising the plan-00-D rename —
            CodeInstruction::new("ldr_u64")
                .field("dst", "%v22")
                .field("base", "%v21")
                .field("offset", "0"),
            CodeInstruction::new("str_u8")
                .field("src", "%v22")
                .field("base", crate::arch::aarch64::regmodel::ARENA_BASE_REGISTER)
                .field("offset", "16"),
            CodeInstruction::new("ldr_d")
                .field("dst", "%f0")
                .field("base", "%v21")
                .field("offset", "24"),
            // — scalar float arith + renamed conversions / bit-reinterprets (Phase 4) —
            CodeInstruction::new("fadd_d")
                .field("dst", "%f1")
                .field("lhs", "%f0")
                .field("rhs", "%f0"),
            CodeInstruction::new("fmadd_d")
                .field("dst", "%f2")
                .field("lhs", "%f1")
                .field("rhs", "%f0")
                .field("acc", "%f1"),
            CodeInstruction::new("scvtf_d_from_x")
                .field("dst", "%f3")
                .field("src", "%v0"),
            CodeInstruction::new("fcvtzs_x_from_d")
                .field("dst", "%v23")
                .field("src", "%f3"),
            CodeInstruction::new("fcvtms_x_from_d")
                .field("dst", "%v24")
                .field("src", "%f3"),
            CodeInstruction::new("fmov_d_from_x")
                .field("dst", "%f4")
                .field("src", "%v0"),
            CodeInstruction::new("fmov_x_from_d")
                .field("dst", "%v25")
                .field("src", "%f4"),
            // — NEON `v128` op (plan-00-E): the `fmla_v` MAC neutralizes to
            //   `v128.fma`; no `*_v` NEON mnemonic survives —
            CodeInstruction::new("fmla_v")
                .field("dst", "%f5")
                .field("lhs", "%f3")
                .field("rhs", "%f4"),
            // — machine-y call/syscall vocabulary (plan-00-F): the helpers'
            //   `bl`/`blr`/`svc` neutralize to `call`/`call_indirect`/`syscall` —
            CodeInstruction::new("bl").field("target", "_mfb_arena_alloc"),
            CodeInstruction::new("blr").field("register", "x16"),
            CodeInstruction::new("svc"),
            // — fused flagless control flow (Phases B/C neighbours) —
            CodeInstruction::new("cmp")
                .field("lhs", "%v0")
                .field("rhs", "%v1"),
            CodeInstruction::new("b.lt").field("target", "Lbody"),
        ];
        assert_round_trips(&fixtures);

        // And the resulting MIR names nothing AArch64-specific from the
        // neutralized families (validation §5): the exotic ints, the scalar
        // float↔int conversions, the `adrp` page pair, and the NEON `*_v` tail.
        let mir = lower_to_mir(&fixtures);
        let banned = [
            "adrp",
            "add_pageoff",
            "smulh",
            "umulh",
            "add_carry",
            "sub_borrow",
            "rorv",
            "rorv_w",
            "rev_w",
            "rev_x",
            "fcvtzs_x_from_d",
            "fcvtms_x_from_d",
            "fcvtps_x_from_d",
            "fcvtas_x_from_d",
            "scvtf_d_from_x",
            "fmov_d_from_x",
            "fmov_x_from_d",
            // NEON-tail mnemonics — none may appear (now `v128.*`, plan-00-E).
            "ldr_q",
            "str_q",
            "fadd_v",
            "fmla_v",
            "fcmgt_v",
            "frintn_v",
            "fcvtzs_v",
            "scvtf_v",
            "add_v",
            "sshl_v",
            "shl_v",
            "bsl_v",
            "bit_v",
            "dup_v_from_x",
            "umov_x_from_v",
            // Machine-y mnemonics — now `call`/`call_indirect`/`syscall` (plan-00-F).
            "bl",
            "blr",
            "svc",
        ];
        let mut saw_v128 = false;
        let mut saw_call = false;
        let mut saw_syscall = false;
        for instruction in &mir {
            let mnemonic = instruction.op.mnemonic();
            assert!(
                !banned.contains(&mnemonic),
                "MIR still names an AArch64-specific op: {mnemonic}"
            );
            if mnemonic.starts_with("v128.") {
                saw_v128 = true;
            }
            saw_call |= mnemonic == "call" || mnemonic == "call_indirect";
            saw_syscall |= mnemonic == "syscall";
        }
        assert!(saw_v128, "the NEON fixture did not lower to a `v128.*` op");
        assert!(
            saw_call,
            "the bl/blr fixtures did not lower to `call`/`call_indirect`"
        );
        assert!(saw_syscall, "the svc fixture did not lower to `syscall`");

        // The pinned arena register never appears in the MIR: the `str_u8`
        // fixture's `base` is the abstract `arena_base`, not `x19` (plan-00-D
        // §2). Selection realizes it back to `x19` byte-for-byte (proved by the
        // `assert_round_trips` above).
        let realization = arena_base_realization();
        let mut saw_arena_base = false;
        for instruction in &mir {
            for (_, value) in &instruction.fields {
                assert_ne!(
                    value, realization,
                    "MIR still names the pinned arena register {realization}"
                );
                if value == ARENA_BASE {
                    saw_arena_base = true;
                }
            }
        }
        assert!(
            saw_arena_base,
            "the arena-base fixture did not lower to `arena_base`"
        );
    }

    /// `arena_base` is the identity through the MIR: an arena load/store names
    /// `arena_base` in the neutral stream and selects back to the pinned `x19`
    /// byte-for-byte (plan-00-D §2). Non-arena register operands are untouched.
    #[test]
    fn arena_base_renames_and_realizes_identically() {
        let realization = arena_base_realization();
        let original = [
            // load the free-list head from the arena, store it back via a vreg.
            CodeInstruction::new("ldr_u64")
                .field("dst", "%v0")
                .field("base", realization)
                .field("offset", "48"),
            CodeInstruction::new("str_u64")
                .field("src", "%v0")
                .field("base", realization)
                .field("offset", "48"),
        ];
        let mir = lower_to_mir(&original);
        // Both base fields are renamed to the neutral `arena_base`; the vreg is not.
        let base = |i: usize| {
            mir[i]
                .fields
                .iter()
                .find(|(k, _)| *k == "base")
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(base(0), Some(ARENA_BASE));
        assert_eq!(base(1), Some(ARENA_BASE));
        assert!(!mir
            .iter()
            .any(|m| m.fields.iter().any(|(_, v)| v == realization)));
        // …and selection restores the pinned register exactly.
        assert_round_trips(&original);
    }

    /// plan-34-A: the three invariant registers — zero, the link register, and the
    /// arena base — are named by neutral tokens in shared lowering, never by an
    /// AArch64 register number. This guards the rename against reintroduction of a
    /// physical `"x19"`/`"x30"`/`"x31"` (the seed of plan-34-C's stream invariant):
    /// a stray physical name that reached `x86_64/select.rs::map_scratch_register`
    /// would be realized as the wrong register (`x19` → `rbp`, not the arena base
    /// `r15`) — a silent miscompile.
    #[test]
    fn invariant_registers_are_neutral_tokens() {
        use crate::target::shared::abi;
        // The tokens are neutral, never an AArch64 register number.
        assert_eq!(abi::ZERO, "xzr");
        assert_eq!(abi::LR, "lr");
        assert_eq!(abi::ARENA, ARENA_BASE);
        for token in [abi::ZERO, abi::LR, abi::ARENA] {
            assert!(
                !matches!(token, "x19" | "x30" | "x31"),
                "invariant-register token must not be an AArch64 register number: {token}"
            );
        }
        // A shared stream that names zero (store source + negate), the link
        // register (frame save), and the arena base (address base) with the abi
        // helpers carries no physical x19/x30/x31 into the MIR — `arena_base` is
        // only realized to `x19` later, in `select_aarch64`.
        let stream = [
            abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, 0),
            abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
            abi::subtract_registers("x9", abi::ZERO, "x9"),
        ];
        let mir = lower_to_mir(&stream);
        for inst in &mir {
            for (_, value) in &inst.fields {
                assert!(
                    !matches!(value.as_str(), "x19" | "x30" | "x31"),
                    "MIR field leaked a physical invariant register: {value}"
                );
            }
        }
    }

    /// plan-34-C Phase 5 — the invariant that makes the hand-picked-scratch bug
    /// (`bug-56`) *unrepresentable*: no shared lowering source may name a physical
    /// AArch64 scratch register (`x9`–`x18`, `x20`–`x28`). Every scratch value is a
    /// virtual register (`%vN`); the call boundary is role tokens (plan-34-B); the
    /// invariant registers are neutral tokens (plan-34-A, guarded above). A
    /// `format!("x{base}")` or a stray `"x13"` cannot pass this test.
    ///
    /// The allowlist is the machine-floor code the plan's §2.6 / Open Decisions
    /// sanction as documented physical-by-design (each entry carries its reason);
    /// `#[cfg(test)]` fixtures (register-literal test inputs) are skipped by
    /// scanning only the code above each file's test module.
    #[test]
    fn shared_lowering_names_no_physical_scratch_register() {
        use std::path::Path;
        // Documented physical-by-design (NOT vreg-able) — plan-34-C:
        //   entry_and_arena.rs — the process entry stub + panic-path integer
        //     formatter run before/around the arena with no allocator frame (§2.6);
        //   runtime_helpers.rs / runtime_helpers_thread.rs — the thread trampoline
        //     and thread ops pin `x20` as the current-thread control-block register
        //     (like `arena_base`) that a worker's `is_cancelled` reads directly, and
        //     the trampoline is machine-floor (its own frame, program-invariant
        //     register save/restore).
        const ALLOWLIST: &[&str] = &[
            "entry_and_arena.rs",
            "runtime_helpers.rs",
            "runtime_helpers_thread.rs",
        ];
        // The forbidden AArch64 scratch registers: x9–x18 and x20–x28. (x0–x8 are
        // the call/syscall boundary → role tokens; x19 is the arena base and x30/x31
        // are lr/zero → invariant tokens, all guarded separately.)
        let forbidden: Vec<String> = (9..=18).chain(20..=28).map(|n| format!("\"x{n}\"")).collect();

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/target/shared/code");
        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read shared/code dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let name = path.file_name().unwrap().to_str().unwrap().to_string();
                if ALLOWLIST.contains(&name.as_str()) {
                    continue;
                }
                // Pure test-module files (`tests.rs`, `test_support.rs`) carry
                // register-literal fixtures, not lowering.
                if name.contains("test") {
                    continue;
                }
                let src = std::fs::read_to_string(&path).expect("read source");
                // Scan only above the test module (register-literal test fixtures).
                let code = match src.find("#[cfg(test)]").or_else(|| src.find("mod tests")) {
                    Some(i) => &src[..i],
                    None => &src,
                };
                for (line_no, line) in code.lines().enumerate() {
                    for reg in &forbidden {
                        if line.contains(reg.as_str()) {
                            offenders.push(format!(
                                "{}:{} names {reg}",
                                path.strip_prefix(&root).unwrap().display(),
                                line_no + 1
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "shared lowering must name no physical scratch register (plan-34-C Phase 5); \
             offenders (vreg them, or add a justified allowlist entry):\n{}",
            offenders.join("\n")
        );
    }

    /// Relocation intents are neutral: a `CodeRelocation` carries a
    /// [`RelocIntent`], its `-mir` name never an AArch64 reloc kind, and the
    /// AArch64 table realizes it back to today's `branch26`/`page21`/`pageoff12`
    /// (plan-00-D §1).
    #[test]
    fn reloc_intents_are_neutral_and_realize_to_aarch64_kinds() {
        use crate::arch::aarch64::reloc::reloc_kind;
        for (intent, neutral, concrete) in [
            (RelocIntent::Call, "call", "branch26"),
            (RelocIntent::DataAddrHi, "data_addr_hi", "page21"),
            (RelocIntent::DataAddrLo, "data_addr_lo", "pageoff12"),
            (RelocIntent::GotLoadHi, "got_load_hi", "page21"),
            (RelocIntent::GotLoadLo, "got_load_lo", "pageoff12"),
        ] {
            assert_eq!(intent.name(), neutral);
            assert_ne!(
                intent.name(),
                concrete,
                "the -mir name must not be an AArch64 kind"
            );
            assert_eq!(reloc_kind(intent), concrete);
        }
    }
}
