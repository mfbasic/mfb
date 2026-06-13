#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeOp {
    Label,
    Mov,
    MovImm,
    Add,
    AddImm,
    SubSp,
    AddSp,
    CmpImm,
    BranchEq,
    Branch,
    BranchLink,
    BranchSelf,
    Svc,
    Ret,
    LdrU64,
    StrU64,
    Adrp,
    AddPageOff,
}

impl CodeOp {
    pub(crate) fn mnemonic(self) -> &'static str {
        match self {
            CodeOp::Label => "label",
            CodeOp::Mov => "mov",
            CodeOp::MovImm => "mov_imm",
            CodeOp::Add => "add",
            CodeOp::AddImm => "add_imm",
            CodeOp::SubSp => "sub_sp",
            CodeOp::AddSp => "add_sp",
            CodeOp::CmpImm => "cmp_imm",
            CodeOp::BranchEq => "b.eq",
            CodeOp::Branch => "b",
            CodeOp::BranchLink => "bl",
            CodeOp::BranchSelf => "branch_self",
            CodeOp::Svc => "svc",
            CodeOp::Ret => "ret",
            CodeOp::LdrU64 => "ldr_u64",
            CodeOp::StrU64 => "str_u64",
            CodeOp::Adrp => "adrp",
            CodeOp::AddPageOff => "add_pageoff",
        }
    }

    pub(crate) fn from_mnemonic(op: &str) -> Result<Self, String> {
        match op {
            "label" => Ok(CodeOp::Label),
            "mov" => Ok(CodeOp::Mov),
            "mov_imm" => Ok(CodeOp::MovImm),
            "add" => Ok(CodeOp::Add),
            "add_imm" => Ok(CodeOp::AddImm),
            "sub_sp" => Ok(CodeOp::SubSp),
            "add_sp" => Ok(CodeOp::AddSp),
            "cmp_imm" => Ok(CodeOp::CmpImm),
            "b.eq" => Ok(CodeOp::BranchEq),
            "b" => Ok(CodeOp::Branch),
            "bl" => Ok(CodeOp::BranchLink),
            "branch_self" => Ok(CodeOp::BranchSelf),
            "svc" => Ok(CodeOp::Svc),
            "ret" => Ok(CodeOp::Ret),
            "ldr_u64" => Ok(CodeOp::LdrU64),
            "str_u64" => Ok(CodeOp::StrU64),
            "adrp" => Ok(CodeOp::Adrp),
            "add_pageoff" => Ok(CodeOp::AddPageOff),
            other => Err(format!("aarch64 code op '{other}' is not encodable")),
        }
    }
}
