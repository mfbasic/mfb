//! Target-neutral machine IR (MIR) scaffold — plan-00-A.
//!
//! This is the foundational *layer* of the MIR effort (`planning/mir.md`): a
//! seam between the NIR→builder lowering and the AArch64 backend, plus the
//! tooling (`-codegen mir`, `-mir`) that lets every later neutralization plan
//! be proven byte-identical against the path it replaces.
//!
//! In Phase A the MIR is a near-1:1 mirror of today's AArch64 instruction
//! stream: [`MirOp`] reuses the [`CodeOp`] variant set 1:1 (renamed later, as
//! the backend is neutralized one op-family at a time), and the MIR keeps the
//! same `op + string-field bag` shape as [`CodeInstruction`] (`mir.md §10`).
//! Because the round trip [`lower_to_mir`] → [`select_aarch64`] is the identity
//! in this phase, routing the backend through the MIR (`-codegen mir`) produces
//! byte-identical `.ncode`/`.nobj`/binaries to the direct path (`mir.md §12.7`),
//! which is the de-risking gate for plans B–G.
//!
//! The `-mir` dump (`mir.md §12a`) is the neutral counterpart to `-ncode`: it
//! serializes the MIR stream (neutral ops, virtual registers, *no* `target`/
//! `arch`) captured **before** register allocation and instruction selection.

use std::cell::RefCell;
use std::sync::OnceLock;

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
//   [`select_aarch64`] expands them back to the exact `cmp; b.cc` / `adds; b.vc`
//   the backend emits today.
// - **expand** ops are the neutral *structural* ops (plan-00-C): a single MIR op
//   that an ISA realizes with a short fixed instruction *sequence*. Today the
//   only one is `addr_of <sym>` — one PC-relative symbol-address op that AArch64
//   selects as the `adrp; add :lo12:` page pair (x86 `lea` RIP-rel, rv64
//   `auipc; addi` later). Like fused ops they have no single `CodeOp` (they
//   expand to two); [`lower_to_mir`] produces them by fusing the adjacent
//   `adrp; add_pageoff` pair and [`select_aarch64`] expands them back
//   byte-for-byte (`mir.md §4`).
macro_rules! mir_ops {
    (
        mirror { $($mv:ident),+ $(,)? }
        renamed { $($rc:ident => $rv:ident => $rm:literal),+ $(,)? }
        fused { $($fv:ident => $fm:literal),+ $(,)? }
        expand { $($ev:ident => $em:literal),+ $(,)? }
    ) => {
        /// Neutral machine-IR opcode (`mir.md §4`/§10).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum MirOp {
            $($mv,)+
            $($rv,)+
            $($fv,)+
            $($ev,)+
        }

        impl MirOp {
            /// Map a selected AArch64 [`CodeOp`] up to its MIR op. Total over
            /// `CodeOp` (every `CodeOp` is a mirror *or* a renamed variant; the
            /// fused/expand ops are produced by fusion in [`lower_to_mir`], never
            /// by `from_code`).
            fn from_code(op: CodeOp) -> Self {
                match op {
                    $(CodeOp::$mv => MirOp::$mv,)+
                    $(CodeOp::$rc => MirOp::$rv,)+
                }
            }

            /// The single AArch64 [`CodeOp`] a mirror/renamed op selects, or
            /// `None` for a fused control-flow op or a structural expand op
            /// (which expand to two instructions — see [`select_aarch64`]).
            fn to_code(self) -> Option<CodeOp> {
                match self {
                    $(MirOp::$mv => Some(CodeOp::$mv),)+
                    $(MirOp::$rv => Some(CodeOp::$rc),)+
                    $(MirOp::$fv => None,)+
                    $(MirOp::$ev => None,)+
                }
            }

            /// Display mnemonic for the `-mir` dump.
            pub(crate) fn mnemonic(self) -> &'static str {
                match self {
                    $(MirOp::$mv => CodeOp::$mv.mnemonic(),)+
                    $(MirOp::$rv => $rm,)+
                    $(MirOp::$fv => $fm,)+
                    $(MirOp::$ev => $em,)+
                }
            }
        }
    };
}

mir_ops!(
    mirror {
        Label, Mov, MovImm, Add, Adds, Sub, Subs, And, Orr, Eor, Mvn, Mul, Lslv, Lsrv, Asrv, Clz,
        Rbit, SDiv, UDiv, MSub, LslImm, LsrImm, AsrImm, AddImm, SubImm, SubSp, AddSp, CmpImm, Cmp,
        BranchEq, BranchNe, BranchGe, BranchLt, BranchGt, BranchLe, BranchVc, BranchVs, BranchHi,
        BranchLo, BranchMi, BranchLs, Branch, BranchLink, BranchLinkRegister, BranchSelf, Svc, Ret,
        LdrU64, LdrU32, LdrU16, LdrU8, StrU64, StrU32, StrU8, LdrD, StrD, Adrp, AddPageOff,
        FMovDFromD, FAddD, FSubD, FMulD, FDivD, FNegD, FAbsD, FSqrtD, FCmpD, FCmpZeroD, LdrQ, StrQ,
        FAddV, FSubV, FMulV, FDivV, FMlaV, FMlsV, FMinV, FMaxV, FCmGtV, FCmGeV, FCmEqV, FAbsV,
        FNegV, FSqrtV, FRintpV, FRintmV, FRintaV, FRintnV, FRintzV, FCvtzsV, FCvtasV, ScvtfV,
        FCmGtZeroV, FCmGeZeroV, FCmEqZeroV, FCmLtZeroV, FCmLeZeroV, AddV, SubV, CmGtV, CmGeV, CmEqV,
        SshlV, UshlV, NegV, AbsV, AndV, OrrV, EorV, BslV, BitV, ShlV, SshrV, UshrV, DupVFromX,
        UmovXFromV, FMaddD,
    }
    // Neutral semantic names for the AArch64-specific scalar shapes (plan-00-C
    // Phases 3 & 4). `CodeOp` (lhs) ⇒ `MirOp` (mid) ⇒ neutral mnemonic (rhs);
    // the conversion is still 1:1, so selection is byte-identical.
    renamed {
        // §3 "exotic" integer ops — not 1:1 across ISAs, named semantically here
        // so the backends can select natively or expand.
        SMulH => MulhiS => "mulhi_s",     // smulh: signed 64×64→high 64
        UMulH => MulhiU => "mulhi_u",     // umulh: unsigned 64×64→high 64
        Adc => AddC => "addc",            // adc:   add with carry-in/out
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
    }
    expand {
        // PC-relative symbol address (`mir.md §4`): one neutral op for the
        // AArch64 `adrp; add :lo12:` page pair. Fused from the adjacent pair in
        // [`lower_to_mir`] and expanded back byte-for-byte in [`select_aarch64`].
        AddrOf => "addr_of",
    }
);

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
        _ => None,
    }
}

/// The AArch64 flag-setter [`CodeOp`] a fused MIR op expands back to (its first
/// of two instructions), or `None` for a non-fused op.
fn fused_setter_codeop(op: MirOp) -> Option<CodeOp> {
    match op {
        MirOp::BrCc => Some(CodeOp::Cmp),
        MirOp::BrCcImm => Some(CodeOp::CmpImm),
        MirOp::FBrCc => Some(CodeOp::FCmpD),
        MirOp::FBrCcZero => Some(CodeOp::FCmpZeroD),
        MirOp::AddOvf => Some(CodeOp::Adds),
        MirOp::SubOvf => Some(CodeOp::Subs),
        _ => None,
    }
}

/// Whether a branch reads condition flags (so it pairs with a preceding
/// flag-setter). The unconditional `b`, `bl`, `blr`, `branch_self`, `svc`, and
/// `ret` do not. A `svc; b.lo` carry check (a hand-written syscall helper, not
/// builder-emitted) is therefore left un-fused — its flag source is the
/// syscall, not a compare; that is the separate `syscall` neutralization
/// (`mir.md §7`).
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
/// unambiguously the split point for [`select_aarch64`].
const FUSED_COND_FIELD: &str = "cond";

/// Field key marking a compare-and-branch that **reuses** the immediately
/// preceding fused op's comparison instead of computing its own (a second/third
/// branch on one AArch64 `cmp`, e.g. the 3-way `cmp; b.lo; b.hi` string
/// ordering). The op still carries its operands (so it is self-contained and an
/// ISA without flag reuse can re-emit the compare), but the AArch64 selector
/// emits only the branch — reproducing the single shared `cmp` byte-for-byte.
const FUSED_SHARE_FIELD: &str = "share";

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
        if add.get("dst") == Some(dst) && add.get("src") == Some(dst) && add.get("symbol") == Some(symbol)
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
    out
}

/// AArch64 instruction selection (`MIR → machine ops`). Mirror ops map back to
/// their one [`CodeOp`] over the identical field bag; a fused flagless op
/// expands back to the exact two instructions it folded — the flag-setter
/// (`cmp`/`fcmp`/`adds`/`subs`) and the flag-reading branch — reproducing the
/// stream the backend emits today **byte-for-byte**. The result feeds the
/// existing register allocator / peephole / encoder unchanged.
pub(crate) fn select_aarch64(instructions: &[MirInstruction]) -> Vec<CodeInstruction> {
    let mut out = Vec::with_capacity(instructions.len());
    for instruction in instructions {
        if instruction.op == MirOp::AddrOf {
            // Structural expand (plan-00-C): `addr_of <dst>, <sym>` → the exact
            // `adrp <dst>, <sym>; add_pageoff <dst>, <dst>, <sym>` pair the
            // builders emit today (`abi::load_page_address` + `add_page_offset`).
            let dst = instruction
                .fields
                .iter()
                .find(|(key, _)| *key == "dst")
                .map(|(_, value)| value.clone())
                .expect("addr_of carries a dst field");
            let symbol = instruction
                .fields
                .iter()
                .find(|(key, _)| *key == "symbol")
                .map(|(_, value)| value.clone())
                .expect("addr_of carries a symbol field");
            out.push(abi::load_page_address(&dst, &symbol));
            out.push(abi::add_page_offset(&dst, &dst, &symbol));
            continue;
        }
        if let Some(setter_op) = fused_setter_codeop(instruction.op) {
            // Split the field bag at the `cond` marker: everything before it is
            // the flag-setter's operands; its value is the branch mnemonic;
            // everything after is the branch's operands (plus an optional
            // `share` marker).
            let split = instruction
                .fields
                .iter()
                .position(|(key, _)| *key == FUSED_COND_FIELD)
                .expect("fused MIR op carries a cond field");
            let setter_fields = instruction.fields[..split].to_vec();
            let branch_op = CodeOp::from_mnemonic(&instruction.fields[split].1)
                .expect("fused MIR op carries a valid branch mnemonic");
            let mut branch_fields = Vec::new();
            let mut shared = false;
            for (key, value) in &instruction.fields[split + 1..] {
                if *key == FUSED_SHARE_FIELD {
                    shared = true;
                } else {
                    branch_fields.push((*key, value.clone()));
                }
            }
            // A shared branch reuses the comparison the previous fused op
            // already emitted, so emit only its branch.
            if !shared {
                out.push(CodeInstruction {
                    op: setter_op,
                    fields: setter_fields,
                });
            }
            out.push(CodeInstruction {
                op: branch_op,
                fields: branch_fields,
            });
        } else {
            out.push(CodeInstruction {
                op: instruction
                    .op
                    .to_code()
                    .expect("non-fused MIR op maps to a single CodeOp"),
                fields: instruction.fields.clone(),
            });
        }
    }
    out
}

// --- `-codegen <direct|mir>` selection (the plan-03 `-regalloc` pattern) ------

/// Which code-generation path the backend runs. Selected by `-codegen <name>`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CodegenKind {
    /// Today's path: builders emit the AArch64 stream directly, no MIR layer.
    /// The shipping default until plan-00-G flips it.
    Direct,
    /// Route the lowered stream through the neutral MIR and back
    /// ([`lower_to_mir`] → [`select_aarch64`]) before register allocation.
    /// Byte-identical to [`Direct`] in Phase A — the self-diff oracle.
    Mir,
}

impl CodegenKind {
    #[allow(dead_code)]
    pub(crate) fn name(self) -> &'static str {
        match self {
            CodegenKind::Direct => "direct",
            CodegenKind::Mir => "mir",
        }
    }
}

/// Names accepted by `-codegen`, for the error message on an unknown value.
pub(crate) fn available_codegens() -> &'static [&'static str] {
    &["direct", "mir"]
}

/// Parse a `-codegen` value, listing the available paths on an unknown name.
pub(crate) fn parse_codegen(value: &str) -> Result<CodegenKind, String> {
    match value {
        "direct" => Ok(CodegenKind::Direct),
        "mir" => Ok(CodegenKind::Mir),
        other => Err(format!(
            "unknown -codegen path `{other}` (available: {})",
            available_codegens().join(", ")
        )),
    }
}

static SELECTED_CODEGEN: OnceLock<CodegenKind> = OnceLock::new();

/// Record the process-wide code-generation path chosen on the command line. May
/// be called at most once per process; ignored if already set.
pub(crate) fn set_codegen(kind: CodegenKind) {
    let _ = SELECTED_CODEGEN.set(kind);
}

/// The active code-generation path, defaulting to [`CodegenKind::Direct`] —
/// today's no-MIR AArch64 backend, which stays the shipping default until the
/// MIR path is proven and flipped (plan-00-G).
pub(crate) fn active_codegen() -> CodegenKind {
    *SELECTED_CODEGEN.get().unwrap_or(&CodegenKind::Direct)
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

/// One MIR function: the program/runtime metadata plus the neutral op stream.
/// Carries no frame/relocation data — those are post-selection backend concerns
/// (the `-mir` dump is *before* selection and allocation).
pub(crate) struct MirFunction {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) returns: String,
    pub(crate) params: Vec<CodeParam>,
    pub(crate) instructions: Vec<MirInstruction>,
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
                "{}  \"instructions\": [{}\n{}  ]\n",
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
            pad
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_round_trips(original: &[CodeInstruction]) {
        let round_tripped = select_aarch64(&lower_to_mir(original));
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
            CodeOp::FMaddD,
            CodeOp::DupVFromX,
            CodeOp::UmovXFromV,
            CodeOp::Clz,  // already-neutral exotic int op — stays a mirror
            CodeOp::Rbit, // (clz/rbit/msub keep their semantic AArch64 name)
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
            (CodeOp::Adc, "addc"),
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
        ] {
            let mir = MirOp::from_code(code);
            // 1:1 selection back to the same CodeOp — byte-identical encoding.
            assert_eq!(mir.to_code(), Some(code));
            // Neutral MIR mnemonic, and it no longer names the AArch64 op.
            assert_eq!(mir.mnemonic(), neutral);
            assert_ne!(mir.mnemonic(), code.mnemonic());
        }
    }

    /// `select_aarch64 ∘ lower_to_mir` is the identity on a non-fusing stream.
    #[test]
    fn lower_then_select_is_identity() {
        assert_round_trips(&[
            CodeInstruction::new("mov").field("dst", "%v0").field("src", "x0"),
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
            CodeInstruction::new("cmp_imm").field("lhs", "x0").field("rhs", "2"),
            CodeInstruction::new("b.eq")
                .field("target", "if_else_0")
                .field("reason", "ifFalse"),
        ]);
        // cmp; b.lt (register compare)
        assert_round_trips(&[
            CodeInstruction::new("cmp").field("lhs", "%v0").field("rhs", "%v1"),
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
            CodeInstruction::new("fcmp_d").field("lhs", "%f0").field("rhs", "%f1"),
            CodeInstruction::new("b.mi").field("target", "Lt"),
        ]);
        assert_round_trips(&[
            CodeInstruction::new("fcmp_d").field("lhs", "%f0").field("rhs", "%f1"),
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
            CodeInstruction::new("cmp_imm").field("lhs", "x0").field("rhs", "255"),
            CodeInstruction::new("b.hi").field("target", "range_err"),
        ]);
        assert_eq!(mir.len(), 1);
        assert_eq!(mir[0].op, MirOp::BrCcImm);
        assert_eq!(mir[0].op.mnemonic(), "br_cc_imm");
        let get = |k: &str| mir[0].fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.as_str());
        assert_eq!(get("lhs"), Some("x0"));
        assert_eq!(get("rhs"), Some("255"));
        assert_eq!(get("cond"), Some("b.hi"));
        assert_eq!(get("target"), Some("range_err"));
    }

    /// A flag-setter NOT followed by a flag-reading branch stays a mirror op —
    /// the `adds; adc` 128-bit carry chain must not be mistaken for overflow.
    #[test]
    fn carry_chain_is_not_fused() {
        let original = [
            CodeInstruction::new("adds")
                .field("dst", "x9")
                .field("lhs", "x9")
                .field("rhs", "x1"),
            CodeInstruction::new("adc")
                .field("dst", "x10")
                .field("lhs", "x10")
                .field("rhs", "xzr"),
        ];
        let mir = lower_to_mir(&original);
        assert_eq!(mir.len(), 2);
        assert_eq!(mir[0].op, MirOp::Adds);
        assert_eq!(mir[1].op, MirOp::AddC); // neutral-renamed `adc` → `addc`
        assert_eq!(mir[1].op.mnemonic(), "addc");
        assert_round_trips(&original);
    }

    /// A 3-way `cmp; b.lo; b.hi` (the string-ordering pattern) becomes two
    /// flagless ops: the first owns the compare, the second shares it. Both are
    /// self-contained (carry operands), and they expand back to the single
    /// shared `cmp` byte-for-byte.
    #[test]
    fn multi_branch_compare_fuses_with_share() {
        let original = [
            CodeInstruction::new("cmp").field("lhs", "x16").field("rhs", "x11"),
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
            m.fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.as_str())
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
            CodeInstruction::new("adrp").field("dst", "x9").field("symbol", "str.0"),
            CodeInstruction::new("add_pageoff")
                .field("dst", "x9")
                .field("src", "x9")
                .field("symbol", "str.0"),
        ];
        let mir = lower_to_mir(&original);
        assert_eq!(mir.len(), 1);
        assert_eq!(mir[0].op, MirOp::AddrOf);
        assert_eq!(mir[0].op.mnemonic(), "addr_of");
        let get = |k: &str| mir[0].fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.as_str());
        assert_eq!(get("dst"), Some("x9"));
        assert_eq!(get("symbol"), Some("str.0"));
        // No `adrp`/`add_pageoff` mnemonic survives in the MIR (validation §5).
        assert!(!mir.iter().any(|m| matches!(m.op, MirOp::Adrp | MirOp::AddPageOff)));
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
            CodeInstruction::new("adrp").field("dst", "x9").field("symbol", "g"),
            CodeInstruction::new("ret"),
        ];
        let mir = lower_to_mir(&lone);
        assert_eq!(mir[0].op, MirOp::Adrp);
        assert_round_trips(&lone);
        // add_pageoff on a *different* register than the adrp — not an addr_of.
        let mismatch = [
            CodeInstruction::new("adrp").field("dst", "x9").field("symbol", "g"),
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

    /// The byte-identical gate, in miniature: a 36-fixture sweep with at least
    /// one instruction from **every** builder op family — moves & immediates,
    /// the universal ALU, the neutral-renamed exotic integer ops, the structural
    /// `addr_of` pair, every load/store width, scalar float arith + the renamed
    /// float↔int conversions & bit-reinterprets, the NEON `v128` ops, and the
    /// fused flagless control-flow ops. `select_aarch64 ∘ lower_to_mir` must be
    /// the identity on the whole stream (the property the `.ncode`/binary
    /// self-diff oracle, `scripts/codegen-selfdiff.sh`, proves end-to-end).
    #[test]
    fn round_trip_sweep_over_every_op_family() {
        let fixtures: [CodeInstruction; 36] = [
            // — moves & immediates —
            CodeInstruction::new("mov").field("dst", "%v0").field("src", "x1"),
            CodeInstruction::new("mov_imm").field("dst", "%v1").field("value", "4294967296"),
            // — universal ALU (incl. the immediate forms that keep small imms) —
            CodeInstruction::new("add").field("dst", "%v2").field("lhs", "%v0").field("rhs", "%v1"),
            CodeInstruction::new("sub").field("dst", "%v3").field("lhs", "%v2").field("rhs", "%v0"),
            CodeInstruction::new("mul").field("dst", "%v4").field("lhs", "%v2").field("rhs", "%v3"),
            CodeInstruction::new("and").field("dst", "%v5").field("lhs", "%v4").field("rhs", "%v0"),
            CodeInstruction::new("orr").field("dst", "%v6").field("lhs", "%v5").field("rhs", "%v1"),
            CodeInstruction::new("eor").field("dst", "%v7").field("lhs", "%v6").field("rhs", "%v2"),
            CodeInstruction::new("mvn").field("dst", "%v8").field("src", "%v7"),
            CodeInstruction::new("sdiv").field("dst", "%v9").field("lhs", "%v8").field("rhs", "%v0"),
            CodeInstruction::new("udiv").field("dst", "%v10").field("lhs", "%v9").field("rhs", "%v1"),
            CodeInstruction::new("lslv").field("dst", "%v11").field("lhs", "%v10").field("rhs", "%v0"),
            CodeInstruction::new("add_imm").field("dst", "%v12").field("src", "%v11").field("imm", "8"),
            CodeInstruction::new("lsl_imm").field("dst", "%v13").field("src", "%v12").field("shift", "3"),
            // — neutral-renamed "exotic" integer ops (Phase 3) —
            CodeInstruction::new("smulh").field("dst", "%v14").field("lhs", "%v0").field("rhs", "%v1"),
            CodeInstruction::new("umulh").field("dst", "%v15").field("lhs", "%v0").field("rhs", "%v1"),
            CodeInstruction::new("adc").field("dst", "%v16").field("lhs", "%v14").field("rhs", "%v15"),
            CodeInstruction::new("rorv").field("dst", "%v17").field("lhs", "%v16").field("rhs", "%v0"),
            CodeInstruction::new("rev_x").field("dst", "%v18").field("src", "%v17"),
            CodeInstruction::new("clz").field("dst", "%v19").field("src", "%v18"),
            CodeInstruction::new("msub")
                .field("dst", "%v20")
                .field("lhs", "%v0")
                .field("rhs", "%v1")
                .field("minuend", "%v2"),
            // — structural addr_of page pair (Phase 1): fuses to one op —
            CodeInstruction::new("adrp").field("dst", "%v21").field("symbol", "pool"),
            CodeInstruction::new("add_pageoff")
                .field("dst", "%v21")
                .field("src", "%v21")
                .field("symbol", "pool"),
            // — loads/stores, every width —
            CodeInstruction::new("ldr_u64").field("dst", "%v22").field("base", "%v21").field("offset", "0"),
            CodeInstruction::new("str_u8").field("src", "%v22").field("base", "%v21").field("offset", "16"),
            CodeInstruction::new("ldr_d").field("dst", "%f0").field("base", "%v21").field("offset", "24"),
            // — scalar float arith + renamed conversions / bit-reinterprets (Phase 4) —
            CodeInstruction::new("fadd_d").field("dst", "%f1").field("lhs", "%f0").field("rhs", "%f0"),
            CodeInstruction::new("fmadd_d")
                .field("dst", "%f2")
                .field("lhs", "%f1")
                .field("rhs", "%f0")
                .field("acc", "%f1"),
            CodeInstruction::new("scvtf_d_from_x").field("dst", "%f3").field("src", "%v0"),
            CodeInstruction::new("fcvtzs_x_from_d").field("dst", "%v23").field("src", "%f3"),
            CodeInstruction::new("fcvtms_x_from_d").field("dst", "%v24").field("src", "%f3"),
            CodeInstruction::new("fmov_d_from_x").field("dst", "%f4").field("src", "%v0"),
            CodeInstruction::new("fmov_x_from_d").field("dst", "%v25").field("src", "%f4"),
            // — NEON v128 op —
            CodeInstruction::new("fmla_v").field("dst", "%f5").field("lhs", "%f3").field("rhs", "%f4"),
            // — fused flagless control flow (Phases B/C neighbours) —
            CodeInstruction::new("cmp").field("lhs", "%v0").field("rhs", "%v1"),
            CodeInstruction::new("b.lt").field("target", "Lbody"),
        ];
        assert_round_trips(&fixtures);

        // And the resulting MIR names nothing AArch64-specific from the
        // neutralized families (validation §5).
        let mir = lower_to_mir(&fixtures);
        let banned = [
            "adrp", "add_pageoff", "smulh", "umulh", "adc", "rorv", "rorv_w", "rev_w", "rev_x",
            "fcvtzs_x_from_d", "fcvtms_x_from_d", "fcvtps_x_from_d", "fcvtas_x_from_d",
            "scvtf_d_from_x", "fmov_d_from_x", "fmov_x_from_d",
        ];
        for instruction in &mir {
            let mnemonic = instruction.op.mnemonic();
            assert!(
                !banned.contains(&mnemonic),
                "MIR still names an AArch64-specific op: {mnemonic}"
            );
        }
    }
}
