use super::*;

impl CodeInstruction {
    pub(crate) fn new(op: &str) -> Self {
        Self {
            op: CodeOp::from_mnemonic(op).unwrap_or_else(|err| panic!("{err}")),
            fields: Vec::new(),
        }
    }

    pub(crate) fn field(mut self, name: &'static str, value: &str) -> Self {
        self.fields.push((name, value.to_string()));
        self
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
            | CodeOp::Adc
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
            | CodeOp::FDivD => &["dst", "lhs", "rhs"],
            CodeOp::Mvn | CodeOp::Clz | CodeOp::Rbit | CodeOp::RevW | CodeOp::RevX => {
                &["dst", "src"]
            }
            CodeOp::MSub => &["dst", "lhs", "rhs", "minuend"],
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
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::Branch
            | CodeOp::BranchLink => &["target"],
            CodeOp::BranchLinkRegister => &["register"],
            CodeOp::BranchSelf | CodeOp::Svc | CodeOp::Ret => &[],
            CodeOp::LdrU64 | CodeOp::LdrU32 | CodeOp::LdrU16 | CodeOp::LdrU8 => {
                &["dst", "base", "offset"]
            }
            CodeOp::StrU64 | CodeOp::StrU32 | CodeOp::StrU8 => &["src", "base", "offset"],
            CodeOp::Adrp | CodeOp::AddPageOff => &["dst", "symbol"],
            CodeOp::FMovXFromD
            | CodeOp::FMovDFromX
            | CodeOp::FNegD
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
            | CodeOp::FCmGtZeroV
            | CodeOp::FCmGeZeroV
            | CodeOp::FCmEqZeroV
            | CodeOp::FCmLtZeroV
            | CodeOp::FCmLeZeroV => &["dst", "src"],
            CodeOp::ShlV | CodeOp::SshrV | CodeOp::UshrV => &["dst", "src", "shift"],
            CodeOp::DupVFromX => &["dst", "src"],
            CodeOp::UmovXFromV => &["dst", "src", "index"],
            CodeOp::FMaddD => &["dst", "addend", "lhs", "rhs"],
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
                .map(|(name, value)| format!("\"{name}\": {}", json_string(value))),
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
        format!(
            "\n{}{{ \"from\": {}, \"to\": {}, \"kind\": {}, \"binding\": {}, \"library\": {} }}",
            pad,
            json_string(&self.from),
            json_string(&self.to),
            json_string(&self.kind),
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
