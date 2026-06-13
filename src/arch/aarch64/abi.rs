use crate::target::shared::code::CodeInstruction;

pub(crate) const RETURN_REGISTER: &str = "x0";
pub(crate) const IO_PRINT_CLOBBERS: &[&str] = &["x0", "x1", "x2", "x9", "x16"];

pub(crate) fn argument_register(index: usize) -> Result<String, String> {
    if index < 8 {
        Ok(format!("x{index}"))
    } else {
        Err(format!(
            "aarch64 code plan cannot pass argument {index}; stack arguments are not implemented"
        ))
    }
}

pub(crate) fn temporary_register(allocation: usize) -> Result<String, String> {
    let register = match allocation {
        8..=17 => format!("x{allocation}"),
        18 => "x19".to_string(),
        19 => "x20".to_string(),
        20 => "x21".to_string(),
        21 => "x22".to_string(),
        22 => "x23".to_string(),
        23 => "x24".to_string(),
        24 => "x25".to_string(),
        25 => "x26".to_string(),
        26 => "x27".to_string(),
        27 => "x28".to_string(),
        other => {
            return Err(format!(
                "aarch64 code plan exhausted physical registers at allocation {other}"
            ));
        }
    };
    Ok(register)
}

pub(crate) fn return_register() -> &'static str {
    RETURN_REGISTER
}

pub(crate) fn link_register() -> &'static str {
    "x30"
}

pub(crate) fn stack_pointer() -> &'static str {
    "sp"
}

pub(crate) fn syscall_register() -> &'static str {
    "x8"
}

pub(crate) fn string_length_register() -> &'static str {
    "x2"
}

pub(crate) fn string_data_register() -> &'static str {
    "x1"
}

pub(crate) fn newline_scratch_register() -> &'static str {
    "x9"
}

pub(crate) fn is_callee_saved(register: &str) -> bool {
    matches!(
        register,
        "x19" | "x20" | "x21" | "x22" | "x23" | "x24" | "x25" | "x26" | "x27" | "x28"
    )
}

pub(crate) fn is_stack_pointer(register: &str) -> bool {
    register == stack_pointer()
}

pub(crate) fn label(name: &str) -> CodeInstruction {
    CodeInstruction::new("label").field("name", name)
}

pub(crate) fn move_register(dst: &str, src: &str) -> CodeInstruction {
    CodeInstruction::new("mov")
        .field("dst", dst)
        .field("src", src)
}

pub(crate) fn move_immediate(dst: &str, type_: &str, value: &str) -> CodeInstruction {
    CodeInstruction::new("mov_imm")
        .field("dst", dst)
        .field("type", type_)
        .field("value", value)
}

pub(crate) fn add_immediate(dst: &str, src: &str, imm: usize) -> CodeInstruction {
    CodeInstruction::new("add_imm")
        .field("dst", dst)
        .field("src", src)
        .field("imm", &imm.to_string())
}

pub(crate) fn add_registers(dst: &str, lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("add")
        .field("dst", dst)
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn subtract_stack(imm: usize) -> CodeInstruction {
    CodeInstruction::new("sub_sp").field("imm", &imm.to_string())
}

pub(crate) fn add_stack(imm: usize) -> CodeInstruction {
    CodeInstruction::new("add_sp").field("imm", &imm.to_string())
}

pub(crate) fn compare_immediate(lhs: &str, rhs: &str) -> CodeInstruction {
    CodeInstruction::new("cmp_imm")
        .field("lhs", lhs)
        .field("rhs", rhs)
}

pub(crate) fn branch_eq(target: &str) -> CodeInstruction {
    CodeInstruction::new("b.eq").field("target", target)
}

pub(crate) fn branch(target: &str) -> CodeInstruction {
    CodeInstruction::new("b").field("target", target)
}

pub(crate) fn branch_link(target: &str) -> CodeInstruction {
    CodeInstruction::new("bl").field("target", target)
}

pub(crate) fn branch_self() -> CodeInstruction {
    CodeInstruction::new("branch_self")
}

pub(crate) fn syscall() -> CodeInstruction {
    CodeInstruction::new("svc")
}

pub(crate) fn return_() -> CodeInstruction {
    CodeInstruction::new("ret")
}

pub(crate) fn load_u64(dst: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("ldr_u64")
        .field("dst", dst)
        .field("base", base)
        .field("offset", &offset.to_string())
}

pub(crate) fn store_u64(src: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("str_u64")
        .field("src", src)
        .field("base", base)
        .field("offset", &offset.to_string())
}

pub(crate) fn load_page_address(dst: &str, symbol: &str) -> CodeInstruction {
    CodeInstruction::new("adrp")
        .field("dst", dst)
        .field("symbol", symbol)
}

pub(crate) fn add_page_offset(dst: &str, src: &str, symbol: &str) -> CodeInstruction {
    CodeInstruction::new("add_pageoff")
        .field("dst", dst)
        .field("src", src)
        .field("symbol", symbol)
}
