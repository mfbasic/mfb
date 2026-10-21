use super::*;
use crate::arch::riscv64::regmodel::Riscv64RegisterModel;
use crate::arch::riscv64::select::remap_register;
use crate::codegen::engine::operand::{abi_token, AbiConvention, AbiRole};
use crate::target::shared::abi::realize_abi_positional;
use crate::target::shared::regmodel::RegisterModel;

/// The kernel's `user_regs_struct` is 256 bytes, its `__riscv_d_ext_state`
/// 264 (`f[32]`, `fcsr`, padding): `GETREGSET` answered those sizes on both
/// riscv64 boxes.
#[test]
fn the_layouts_are_the_kernel_s() {
    assert_eq!(std::mem::size_of::<General>(), 256);
    assert_eq!(std::mem::size_of::<Float>(), 264);
}

/// The roles `list reg` prints are the compiler's own: each convention's last
/// argument and result register carries that role, as the riscv64 selection
/// places it, and the pinned registers are the ones the register model
/// reserves.
#[test]
fn roles_name_the_registers_the_compiler_uses() {
    let riscv = |index: u8| {
        remap_register(realize_abi_positional(index)).expect("an AArch64 register maps")
    };
    let last = |convention, abi_role, index: u8, label: &str| {
        // The token exists at `index`, so the convention reaches that far.
        abi_token(convention, abi_role, index);
        let register = riscv(index);
        assert!(
            role(&register).is_some_and(|roles| roles.split(", ").any(|r| r == label)),
            "{register} lacks '{label}'"
        );
        // The register after it by number (`a7`, `x17`, is followed by `s2`).
        let number = X
            .iter()
            .position(|name| *name == register)
            .expect("an ABI name");
        let next = X[number + 1];
        assert!(
            !role(&next).is_some_and(|roles| roles.split(", ").any(|r| r == label)),
            "{next} has '{label}'"
        );
    };
    last(AbiConvention::Mfb, AbiRole::Arg, 7, "MFB arg");
    last(AbiConvention::Mfb, AbiRole::Ret, 3, "MFB result");
    last(AbiConvention::C, AbiRole::Arg, 7, "C arg");
    last(AbiConvention::C, AbiRole::Ret, 1, "C result");
    let model = Riscv64RegisterModel;
    assert_eq!(role(model.arena_base()), Some("Arena"));
    assert_eq!(role(model.current_thread()), Some("Current Thread"));
    assert_eq!(role(model.closure_env()), Some("Closure"));
    let named = registers(&[0; 32], &Float::zeroed())
        .iter()
        .filter(|register| register.role.is_some())
        .map(|register| register.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "s2", "s10", "s11"]
    );
}

#[test]
fn registers_list_in_group_order() {
    let mut general = [0u64; 32];
    for (index, word) in general.iter_mut().enumerate() {
        *word = index as u64;
    }
    let mut float = Float::zeroed();
    float.f[10] = 0x4000_0000_0000_0000;
    float.fcsr = 0x21;
    let listed = registers(&general, &float);
    let names = |group| {
        listed
            .iter()
            .filter(|register| register.group == group)
            .map(|register| register.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_eq!(
        names(RegisterGroup::General),
        "ra,gp,tp,t0,t1,t2,s0,s1,a0,a1,a2,a3,a4,a5,a6,a7,s2,s3,s4,s5,s6,s7,s8,s9,s10,s11,\
         t3,t4,t5,t6"
    );
    assert_eq!(names(RegisterGroup::System), "sp,pc,fcsr");
    assert_eq!(
        names(RegisterGroup::Simd),
        "ft0,ft1,ft2,ft3,ft4,ft5,ft6,ft7,fs0,fs1,fa0,fa1,fa2,fa3,fa4,fa5,fa6,fa7,fs2,fs3,fs4,\
         fs5,fs6,fs7,fs8,fs9,fs10,fs11,ft8,ft9,ft10,ft11"
    );
    let value = |name: &str| {
        listed
            .iter()
            .find(|register| register.name == name)
            .map(|register| register.value)
    };
    // `a0` is `x10`, `pc` word 0, `sp` word 2; `fa0` is `f10`.
    assert_eq!(value("a0"), Some(RegisterValue::Bits64(10)));
    assert_eq!(value("pc"), Some(RegisterValue::Bits64(0)));
    assert_eq!(value("sp"), Some(RegisterValue::Bits64(2)));
    assert_eq!(value("fcsr"), Some(RegisterValue::Bits64(0x21)));
    assert_eq!(
        value("fa0"),
        Some(RegisterValue::Bits64(0x4000_0000_0000_0000))
    );
    assert_eq!(value("zero"), None);
}

#[test]
fn assign_writes_one_register() {
    let mut general = [0u64; 32];
    let mut float = None;
    let reads = std::cell::Cell::new(0);
    let mut read = || {
        reads.set(reads.get() + 1);
        Ok(Float::zeroed())
    };
    for (name, value) in [
        ("a3", 7),
        ("ra", 0x40),
        ("sp", 0x1000),
        ("pc", 0x2000),
        ("fcsr", 1),
    ] {
        assign(&mut general, &mut float, &mut read, name, value).unwrap();
    }
    assert_eq!((general[13], general[1]), (7, 0x40));
    assert_eq!((sp(&general), pc(&general)), (0x1000, 0x2000));
    assert_eq!(float.map(|state| state.fcsr), Some(1));
    assert_eq!(reads.get(), 1);
    for (name, error) in [
        ("zero", "no register 'zero' to set"),
        ("x10", "no register 'x10' to set"),
        ("fa0", "no register 'fa0' to set"),
    ] {
        assert_eq!(
            assign(&mut general, &mut float, &mut read, name, 0).err(),
            Some(error.to_string())
        );
    }
    assert_eq!(
        assign(&mut general, &mut float, &mut read, "fcsr", 1 << 32).err(),
        Some("fcsr holds 32 bits, not 0x0000000100000000".to_string())
    );
}
