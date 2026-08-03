use super::*;

impl CodeInstruction {
    #[track_caller]
    pub(crate) fn new(op: &str) -> Self {
        Self {
            op: CodeOp::from_mnemonic(op).unwrap_or_else(|err| panic!("{err}")),
            fields: Vec::new(),
            // plan-71-C Phase 0: record where this instruction was built. With the
            // `abi::` emit helpers also `#[track_caller]`, this resolves to the
            // shared-builder line, not `abi.rs`. `self.emit()` refines it to the
            // builder's `emit` call site for the common path.
            source: Some(core::panic::Location::caller()),
        }
    }

    /// Append a named operand field. Accepts any `impl Into<Operand>` (a typed
    /// `Operand`, or a `&str`/`&String`/`String` that becomes `Operand::Raw`).
    /// plan-78-B stores the typed `Operand` directly; the rendered spelling is
    /// produced on demand by [`Self::get`] / the dump formatters.
    pub(crate) fn field(mut self, name: &'static str, value: impl Into<Operand>) -> Self {
        self.fields.push((name, value.into()));
        self
    }

    /// Rendered value of a named field, if present. Used by the peephole pass,
    /// `finalize_frame`, and the CFG builder to read register/offset/label
    /// operands without re-deriving them from the encoder.
    ///
    /// plan-78-B Phase 1: this returns an owned `String` (not the old borrowed
    /// `&str`) ahead of the storage flip. Once `fields` stores `Operand`
    /// (Phase 2) a rendered value cannot be borrowed from the store — it is
    /// produced on demand — so the owned return is forced; doing it now, while
    /// storage is still `String`, isolates the caller ripple (`… == Some("x0")`
    /// becomes `….as_deref() == Some("x0")`) from the flip itself. Callers that
    /// want the typed operand use [`Self::operand`] (added with the flip); this
    /// stays the string-comparison / re-parse convenience.
    pub(crate) fn get(&self, name: &str) -> Option<String> {
        self.fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.render())
    }

    /// The typed operand of a named field, if present. plan-78-C reads register
    /// identity (class + id/index) off this without the string re-parse `get()`
    /// + the analysis sniffs would otherwise do. The typed-storage accessor is the
    /// reason plan-78-B flipped `fields` to `Operand`; the hot-path consumer lands
    /// in plan-78-C, so it carries a targeted allow until then (a Phase-2 codegen
    /// test already exercises it, proving the flip stores a real typed value).
    #[allow(dead_code)] // consumed by plan-78-C's typed regalloc; proven live by the Phase-2 test
    pub(crate) fn operand(&self, name: &str) -> Option<&Operand> {
        self.fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        let required: &[&str] = match self.op {
            CodeOp::Label => &["name"],
            CodeOp::Mov => &["dst", "src"],
            CodeOp::MovImm => &["dst", "value"],
            CodeOp::Add
            | CodeOp::Adds
            | CodeOp::Sub
            | CodeOp::Subs
            | CodeOp::And
            | CodeOp::Orr
            | CodeOp::Eor
            | CodeOp::Mul
            | CodeOp::SMulH
            | CodeOp::UMulH
            | CodeOp::Rorv
            | CodeOp::RorvW
            | CodeOp::Lslv
            | CodeOp::Lsrv
            | CodeOp::Asrv
            | CodeOp::SDiv
            | CodeOp::UDiv
            | CodeOp::FAddD
            | CodeOp::FSubD
            | CodeOp::FMulD
            | CodeOp::FMinnmD
            | CodeOp::FMaxnmD
            | CodeOp::FDivD => &["dst", "lhs", "rhs"],
            CodeOp::Mvn
            | CodeOp::Clz
            | CodeOp::Rbit
            | CodeOp::RevW
            | CodeOp::RevX
            | CodeOp::Sxtw => &["dst", "src"],
            CodeOp::MSub => &["dst", "lhs", "rhs", "minuend"],
            // Explicit-carry add/sub (plan-00-G §4): two writes (`dst`,
            // `carry_out`/`borrow_out`) plus the carry-in/borrow-in value.
            CodeOp::AddCarry => &["dst", "carry_out", "lhs", "rhs", "carry_in"],
            CodeOp::SubBorrow => &["dst", "borrow_out", "lhs", "rhs", "borrow_in"],
            CodeOp::LslImm | CodeOp::LsrImm | CodeOp::AsrImm => &["dst", "src", "shift"],
            CodeOp::AddImm | CodeOp::SubImm => &["dst", "src", "imm"],
            CodeOp::SubSp | CodeOp::AddSp => &["imm"],
            CodeOp::CmpImm => &["lhs", "rhs"],
            CodeOp::Cmp => &["lhs", "rhs"],
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
            | CodeOp::X86Jae
            | CodeOp::X86Jp
            | CodeOp::X86Jnp
            | CodeOp::X86Ja
            | CodeOp::X86Jb
            | CodeOp::X86Jbe
            | CodeOp::X86Je
            | CodeOp::X86Jne
            | CodeOp::Branch
            | CodeOp::BranchLink => &["target"],
            CodeOp::BranchLinkRegister => &["register"],
            CodeOp::BranchSelf | CodeOp::Svc | CodeOp::Ret => &[],
            CodeOp::LdrU64 | CodeOp::LdrU32 | CodeOp::LdrU16 | CodeOp::LdrU8 => {
                &["dst", "base", "offset"]
            }
            CodeOp::StrU64 | CodeOp::StrU32 | CodeOp::StrU16 | CodeOp::StrU8 => {
                &["src", "base", "offset"]
            }
            CodeOp::LdrD => &["dst", "base", "offset"],
            CodeOp::StrD => &["src", "base", "offset"],
            CodeOp::Adrp | CodeOp::AddPageOff => &["dst", "symbol"],
            CodeOp::FMovXFromD
            | CodeOp::FMovDFromX
            | CodeOp::FMovDFromD
            | CodeOp::FNegD
            | CodeOp::FAbsD
            | CodeOp::FSqrtD
            | CodeOp::SCvtfDFromX
            | CodeOp::FCvtzsXFromD
            | CodeOp::FCvtmsXFromD
            | CodeOp::FCvtpsXFromD
            | CodeOp::FCvtasXFromD => &["dst", "src"],
            CodeOp::FCmpD => &["lhs", "rhs"],
            CodeOp::FCmpZeroD => &["src"],
            // NEON vector ops (plan-01-simd Phase 1).
            CodeOp::LdrQ => &["dst", "base", "offset"],
            CodeOp::StrQ => &["src", "base", "offset"],
            CodeOp::FAddV
            | CodeOp::FSubV
            | CodeOp::FMulV
            | CodeOp::FDivV
            | CodeOp::FMlaV
            | CodeOp::FMlsV
            | CodeOp::FMinV
            | CodeOp::FMaxV
            | CodeOp::FCmGtV
            | CodeOp::FCmGeV
            | CodeOp::FCmEqV
            | CodeOp::AddV
            | CodeOp::SubV
            | CodeOp::CmGtV
            | CodeOp::CmGeV
            | CodeOp::CmEqV
            | CodeOp::SshlV
            | CodeOp::UshlV
            | CodeOp::AndV
            | CodeOp::OrrV
            | CodeOp::EorV
            | CodeOp::BslV
            | CodeOp::BitV => &["dst", "lhs", "rhs"],
            CodeOp::FAbsV
            | CodeOp::FNegV
            | CodeOp::FSqrtV
            | CodeOp::FRintpV
            | CodeOp::FRintmV
            | CodeOp::FRintaV
            | CodeOp::FRintnV
            | CodeOp::FRintzV
            | CodeOp::FCvtzsV
            | CodeOp::FCvtasV
            | CodeOp::ScvtfV
            | CodeOp::NegV
            | CodeOp::AbsV
            | CodeOp::Cnt8bV
            | CodeOp::Addv8bV
            | CodeOp::FCmGtZeroV
            | CodeOp::FCmGeZeroV
            | CodeOp::FCmEqZeroV
            | CodeOp::FCmLtZeroV
            | CodeOp::FCmLeZeroV => &["dst", "src"],
            CodeOp::ShlV | CodeOp::SshrV | CodeOp::UshrV => &["dst", "src", "shift"],
            CodeOp::DupVFromX => &["dst", "src"],
            CodeOp::UmovXFromV => &["dst", "src", "index"],
            CodeOp::FMaddD | CodeOp::FMsubD | CodeOp::FNmsubD | CodeOp::FNmaddD => {
                &["dst", "addend", "lhs", "rhs"]
            }
            // rv64-only compare-branch / float-compare-to-GPR / set-less-than (plan-99).
            CodeOp::RvBr => &["lhs", "rhs", "cond", "target"],
            CodeOp::RvFcmp => &["dst", "lhs", "rhs", "cmp"],
            CodeOp::Slt | CodeOp::Sltu => &["dst", "lhs", "rhs"],
            // The RVV family (plan-32-B) is table-driven: the `vop` field selects
            // the mnemonic and the encoder enforces that op's specific operand
            // fields (which vary by format), so only `vop` is required here.
            CodeOp::RvVop => &["vop"],
        };
        for name in required {
            if !self.fields.iter().any(|(field, _)| field == name) {
                return Err(format!(
                    "native code instruction '{}' missing field '{}'",
                    self.op.mnemonic(),
                    name
                ));
            }
        }
        Ok(())
    }
}

pub(super) trait ToCodeJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToCodeJson for CodeFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"frame\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"stackSlots\": [{}\n{}  ],\n",
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
            self.frame.to_json(indent + 2),
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            join_json(&self.stack_slots, indent + 2),
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

impl CodeFrame {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"stackSize\": {}, \"calleeSaved\": [{}] }}",
            self.stack_size,
            json_string_list(&self.callee_saved)
        )
    }
}

impl ToCodeJson for CodeParam {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"location\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            json_string(&self.location)
        )
    }
}

impl ToCodeJson for CodeInstruction {
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

impl ToCodeJson for CodeRelocation {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let library = self
            .library
            .as_ref()
            .map(|library| json_string(library))
            .unwrap_or_else(|| "null".to_string());
        // `-ncode` is the concrete AArch64 backend dump, so the neutral intent is
        // serialized through the AArch64 intent→kind table (plan-00-D): the dump
        // still reads `branch26`/`page21`/`pageoff12`, byte-identical to before.
        // (The neutral intent name appears in the `-mir` dump instead.)
        format!(
            "\n{}{{ \"from\": {}, \"to\": {}, \"kind\": {}, \"binding\": {}, \"library\": {} }}",
            pad,
            json_string(&self.from),
            json_string(&self.to),
            json_string(crate::arch::aarch64::reloc::reloc_kind(self.kind)),
            json_string(&self.binding),
            library
        )
    }
}

impl ToCodeJson for CodeImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"library\": {}, \"symbol\": {} }}",
            pad,
            json_string(&self.library),
            json_string(&self.symbol)
        )
    }
}

impl ToCodeJson for CodeDataObject {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{ \"symbol\": {}, \"kind\": {}, \"layout\": {}, ",
                "\"align\": {}, \"size\": {}, \"value\": {} }}"
            ),
            pad,
            json_string(&self.symbol),
            json_string(&self.kind),
            json_string(&self.layout),
            self.align,
            self.size,
            json_string(&self.value)
        )
    }
}

impl ToCodeJson for CodeStackSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"offset\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            self.offset
        )
    }
}

/// Serialize a slice of `ToCodeJson` values into a comma-joined JSON fragment
/// (used by the `-code`/`-mir` array serializers). Merged here from the former
/// 17-line `serialization_utils` module (bug-334 B4): its only consumer is the
/// `ToCodeJson` machinery in this file and its two siblings.
pub(super) fn join_json<T: ToCodeJson>(values: &[T], indent: usize) -> String {
    values
        .iter()
        .map(|value| value.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

/// Serialize a slice of strings into a `", "`-joined list of JSON string
/// literals (the callee-saved register list).
pub(super) fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

impl NativeCodePlan {
    /// Serialize the whole plan to the `-code` JSON dump. Moved here from
    /// validation.rs (bug-334 B6) to sit with the other `ToCodeJson`
    /// serializers rather than beside `NativeCodePlan::validate`.
    pub(crate) fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-native-code-plan\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"buildMode\": {},\n",
                "  \"arch\": {},\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"dataObjects\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(self.build_mode.as_str()),
            json_string(&self.arch),
            json_string(&self.project),
            self.entry_symbol
                .as_ref()
                .map(|symbol| json_string(symbol))
                .unwrap_or_else(|| "null".to_string()),
            join_json(&self.imports, 2),
            join_json(&self.data_objects, 2),
            join_json(&self.functions, 2)
        )
    }
}
