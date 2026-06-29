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

// The MIR op set is two groups (`mir.md §10`):
//
// - **mirror** ops carry over a single AArch64 [`CodeOp`] 1:1 (the plan-00-A
//   layer). The macro derives the `CodeOp` <-> `MirOp` conversions from this
//   list, so the mapping is *total and exhaustive by construction* — a missing
//   variant is a compile error, which is what keeps the byte-identical round
//   trip honest. Their display mnemonic delegates to `CodeOp`.
// - **fused** ops are the neutral, *flagless* control-flow ops (plan-00-B): a
//   compare-and-branch (`br_cc`/`fbr_cc`) or an explicit-overflow arithmetic
//   (`add_ovf`/`sub_ovf`) that each stand in for an AArch64 flag-setter + the
//   flag-reading branch that consumed it. They have no single `CodeOp` (they
//   expand to two), so they are excluded from `to_code` and carry an explicit
//   mnemonic. [`lower_to_mir`] produces them by fusing adjacent pairs;
//   [`select_aarch64`] expands them back to the exact `cmp; b.cc` / `adds; b.vc`
//   the backend emits today.
macro_rules! mir_ops {
    (
        mirror { $($mv:ident),+ $(,)? }
        fused { $($fv:ident => $fm:literal),+ $(,)? }
    ) => {
        /// Neutral machine-IR opcode (`mir.md §10`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum MirOp {
            $($mv,)+
            $($fv,)+
        }

        impl MirOp {
            /// Map a selected AArch64 [`CodeOp`] up to its mirror MIR op. Total
            /// over `CodeOp` (every `CodeOp` is a mirror variant).
            fn from_code(op: CodeOp) -> Self {
                match op {
                    $(CodeOp::$mv => MirOp::$mv),+
                }
            }

            /// The single AArch64 [`CodeOp`] a mirror op selects, or `None` for
            /// a fused control-flow op (which expands to two instructions — see
            /// [`select_aarch64`]).
            fn to_code(self) -> Option<CodeOp> {
                match self {
                    $(MirOp::$mv => Some(CodeOp::$mv),)+
                    $(MirOp::$fv => None,)+
                }
            }

            /// Display mnemonic for the `-mir` dump.
            pub(crate) fn mnemonic(self) -> &'static str {
                match self {
                    $(MirOp::$mv => CodeOp::$mv.mnemonic(),)+
                    $(MirOp::$fv => $fm,)+
                }
            }
        }
    };
}

mir_ops!(
    mirror {
        Label, Mov, MovImm, Add, Adds, Sub, Subs, And, Orr, Eor, Mvn, Mul, SMulH, UMulH, Adc, Rorv,
        RorvW, Lslv, Lsrv, Asrv, Clz, Rbit, RevW, RevX, SDiv, UDiv, MSub, LslImm, LsrImm, AsrImm,
        AddImm, SubImm, SubSp, AddSp, CmpImm, Cmp, BranchEq, BranchNe, BranchGe, BranchLt, BranchGt,
        BranchLe, BranchVc, BranchVs, BranchHi, BranchLo, BranchMi, BranchLs, Branch, BranchLink,
        BranchLinkRegister, BranchSelf, Svc, Ret, LdrU64, LdrU32, LdrU16, LdrU8, StrU64, StrU32,
        StrU8, LdrD, StrD, Adrp, AddPageOff, FMovXFromD, FMovDFromX, FMovDFromD, FAddD, FSubD,
        FMulD, FDivD, FNegD, FAbsD, FSqrtD, FCmpD, FCmpZeroD, SCvtfDFromX, FCvtzsXFromD,
        FCvtmsXFromD, FCvtpsXFromD, FCvtasXFromD, LdrQ, StrQ, FAddV, FSubV, FMulV, FDivV, FMlaV,
        FMlsV, FMinV, FMaxV, FCmGtV, FCmGeV, FCmEqV, FAbsV, FNegV, FSqrtV, FRintpV, FRintmV,
        FRintaV, FRintnV, FRintzV, FCvtzsV, FCvtasV, ScvtfV, FCmGtZeroV, FCmGeZeroV, FCmEqZeroV,
        FCmLtZeroV, FCmLeZeroV, AddV, SubV, CmGtV, CmGeV, CmEqV, SshlV, UshlV, NegV, AbsV, AndV,
        OrrV, EorV, BslV, BitV, ShlV, SshrV, UshrV, DupVFromX, UmovXFromV, FMaddD,
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

    let mut out = Vec::with_capacity(instructions.len());
    let mut i = 0;
    while i < instructions.len() {
        let setter = &instructions[i];
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

    /// Every mirror `CodeOp` round-trips through `MirOp` unchanged — the
    /// property the byte-identical gate rests on.
    #[test]
    fn code_op_round_trips_through_mir() {
        // A representative spread across the op families; `from_code`/`to_code`
        // are exhaustive matches, so a coverage gap is a compile error rather
        // than a test miss.
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
        ] {
            assert_eq!(MirOp::from_code(op).to_code(), Some(op));
            assert_eq!(MirOp::from_code(op).mnemonic(), op.mnemonic());
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
        assert_eq!(mir[1].op, MirOp::Adc);
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
}
