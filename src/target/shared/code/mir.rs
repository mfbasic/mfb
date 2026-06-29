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

// The Phase-A MIR op set mirrors `CodeOp` exactly. The macro lists each variant
// once and derives the enum plus the bijective `CodeOp` <-> `MirOp` conversions,
// so the mapping is *total and exhaustive by construction* — a missing or
// misspelled variant is a compile error, which is what guarantees the
// byte-identical round trip. The display mnemonic delegates to `CodeOp` (1:1 in
// Phase A); later plans that rename a `MirOp` variant detach its mnemonic here.
macro_rules! mir_ops {
    ($($variant:ident),+ $(,)?) => {
        /// Neutral machine-IR opcode. Phase A: a 1:1 mirror of [`CodeOp`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum MirOp {
            $($variant),+
        }

        impl MirOp {
            /// Map a selected AArch64 [`CodeOp`] up to its neutral MIR op.
            fn from_code(op: CodeOp) -> Self {
                match op {
                    $(CodeOp::$variant => MirOp::$variant),+
                }
            }

            /// Lower a neutral MIR op down to the AArch64 [`CodeOp`] that
            /// instruction selection picks for it (the trivial inverse in
            /// Phase A).
            fn to_code(self) -> CodeOp {
                match self {
                    $(MirOp::$variant => CodeOp::$variant),+
                }
            }

            /// Display mnemonic for the `-mir` dump. Delegates to [`CodeOp`]
            /// while the op set is a 1:1 mirror.
            pub(crate) fn mnemonic(self) -> &'static str {
                self.to_code().mnemonic()
            }
        }
    };
}

mir_ops!(
    Label, Mov, MovImm, Add, Adds, Sub, Subs, And, Orr, Eor, Mvn, Mul, SMulH, UMulH, Adc, Rorv,
    RorvW, Lslv, Lsrv, Asrv, Clz, Rbit, RevW, RevX, SDiv, UDiv, MSub, LslImm, LsrImm, AsrImm,
    AddImm, SubImm, SubSp, AddSp, CmpImm, Cmp, BranchEq, BranchNe, BranchGe, BranchLt, BranchGt,
    BranchLe, BranchVc, BranchVs, BranchHi, BranchLo, BranchMi, BranchLs, Branch, BranchLink,
    BranchLinkRegister, BranchSelf, Svc, Ret, LdrU64, LdrU32, LdrU16, LdrU8, StrU64, StrU32, StrU8,
    LdrD, StrD, Adrp, AddPageOff, FMovXFromD, FMovDFromX, FMovDFromD, FAddD, FSubD, FMulD, FDivD,
    FNegD, FAbsD, FSqrtD, FCmpD, FCmpZeroD, SCvtfDFromX, FCvtzsXFromD, FCvtmsXFromD, FCvtpsXFromD,
    FCvtasXFromD, LdrQ, StrQ, FAddV, FSubV, FMulV, FDivV, FMlaV, FMlsV, FMinV, FMaxV, FCmGtV,
    FCmGeV, FCmEqV, FAbsV, FNegV, FSqrtV, FRintpV, FRintmV, FRintaV, FRintnV, FRintzV, FCvtzsV,
    FCvtasV, ScvtfV, FCmGtZeroV, FCmGeZeroV, FCmEqZeroV, FCmLtZeroV, FCmLeZeroV, AddV, SubV, CmGtV,
    CmGeV, CmEqV, SshlV, UshlV, NegV, AbsV, AndV, OrrV, EorV, BslV, BitV, ShlV, SshrV, UshrV,
    DupVFromX, UmovXFromV, FMaddD,
);

/// Raise an AArch64 instruction stream to the neutral MIR (`NIR → MIR`). In
/// Phase A this is a 1:1 op rename over the same field bag — the builder logic
/// that produced the [`CodeInstruction`]s *is* the lowering (`mir.md §3`).
pub(crate) fn lower_to_mir(instructions: &[CodeInstruction]) -> Vec<MirInstruction> {
    instructions
        .iter()
        .map(|instruction| MirInstruction {
            op: MirOp::from_code(instruction.op),
            fields: instruction.fields.clone(),
        })
        .collect()
}

/// AArch64 instruction selection (`MIR → machine ops`). In Phase A this is the
/// trivial inverse of [`lower_to_mir`]: each MIR op maps back to the one
/// [`CodeOp`] it mirrors, over the identical field bag. The resulting stream
/// feeds the existing register allocator / peephole / encoder unchanged.
pub(crate) fn select_aarch64(instructions: &[MirInstruction]) -> Vec<CodeInstruction> {
    instructions
        .iter()
        .map(|instruction| CodeInstruction {
            op: instruction.op.to_code(),
            fields: instruction.fields.clone(),
        })
        .collect()
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

    /// Every Phase-A `CodeOp` round-trips through `MirOp` unchanged — the
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
            assert_eq!(MirOp::from_code(op).to_code(), op);
            assert_eq!(MirOp::from_code(op).mnemonic(), op.mnemonic());
        }
    }

    /// `select_aarch64 ∘ lower_to_mir` is the identity on the instruction
    /// stream (op and the ordered field bag), so `-codegen mir` is
    /// byte-identical to `-codegen direct`.
    #[test]
    fn lower_then_select_is_identity() {
        let original = vec![
            CodeInstruction::new("mov").field("dst", "%v0").field("src", "x0"),
            CodeInstruction::new("add")
                .field("dst", "%v1")
                .field("lhs", "%v0")
                .field("rhs", "%v0"),
            CodeInstruction::new("ret"),
        ];
        let round_tripped = select_aarch64(&lower_to_mir(&original));
        assert_eq!(round_tripped.len(), original.len());
        for (after, before) in round_tripped.iter().zip(original.iter()) {
            assert_eq!(after.op, before.op);
            assert_eq!(after.fields, before.fields);
        }
    }
}
