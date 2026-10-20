//! plan-176-V: the Vulkan call plumbing — C-struct building, create/device calls,
//! graphics-state loads and stores, integer and address arguments, library loading and `dlsym`.
//! The shared constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// One member of a Vulkan create-info struct.
///
/// Vulkan's structs are the bulk of this backend, and writing each as a run of
/// `move_immediate`/`store_u64` would make every offset a bare number inside a wall
/// of emitter calls — unreadable, and unauditable in a file where nothing checks the
/// offsets and a wrong one hands garbage to a driver. So each struct is a *table*,
/// one row per member, naming the member beside its offset.
pub(super) enum Field<'a> {
    /// A 32-bit value: an `sType`, an enum, a flags word, a count.
    U32(&'a str),
    /// The address of another stack slot — `sp + slot`.
    Addr(usize),
}

/// Zero `size` bytes at `sp + base`, then write each named member.
///
/// The zeroing is not tidiness. Vulkan reads `pNext` and `flags` on every struct, and
/// a reserved field left holding stack garbage is not ignored — so zeroing the whole
/// range and writing only the members the table names is what makes each table
/// *complete* rather than merely representative.
pub(super) fn emit_struct(
    builder: &mut CodeBuilder,
    base: usize,
    size: usize,
    fields: &[(usize, Field)],
) {
    emit_zero_range(builder, base, size);
    for (offset, field) in fields {
        match field {
            Field::U32(value) => {
                builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
                builder.emit(abi::store_u32(
                    abi::SCRATCH[0],
                    abi::stack_pointer(),
                    base + offset,
                ));
            }
            Field::Addr(slot) => {
                builder.emit(abi::add_immediate(
                    abi::SCRATCH[0],
                    abi::stack_pointer(),
                    *slot,
                ));
                builder.emit(abi::store_u64(
                    abi::SCRATCH[0],
                    abi::stack_pointer(),
                    base + offset,
                ));
            }
        }
    }
}

/// `vkCreateX(device, &info, NULL, &out)` then store `out` into the state block.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_create_call(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    name: &str,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_info: usize,
    off_out: usize,
    state_slot: usize,
    unavailable: &str,
) -> Result<(), String> {
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        name,
        off_handle,
        off_fn,
        unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        off_out,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(unavailable));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    emit_state_store(builder, off_state, state_slot, abi::SCRATCH[0]);
    Ok(())
}

/// `vkX(device)` — the one-argument device calls.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_device_call_1(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    name: &str,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    unavailable: &str,
) -> Result<(), String> {
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        name,
        off_handle,
        off_fn,
        unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    emit_call_fn(builder, off_fn);
    Ok(())
}

/// Park the graphics-state block's address in `slot`, once, so every later access is
/// a load rather than another `adrp`/`add` pair.
pub(super) fn state_base_into(builder: &mut CodeBuilder, slot: usize) {
    let symbol = builder.current_symbol.clone();
    push_symbol_address(
        &symbol,
        GRAPHICS_STATE_SYMBOL,
        abi::SCRATCH[0],
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), slot));
}

pub(super) fn emit_state_store(
    builder: &mut CodeBuilder,
    state_slot: usize,
    offset: usize,
    value: impl Into<abi::Reg>,
) {
    let value: abi::Reg = value.into();
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        state_slot,
    ));
    builder.emit(abi::store_u64(value, abi::SCRATCH[3], offset));
}

pub(super) fn emit_state_load(
    builder: &mut CodeBuilder,
    state_slot: usize,
    offset: usize,
    dst: impl Into<abi::Reg>,
) {
    // No temporary at all: the state base goes into `dst` itself, which the second load
    // overwrites anyway. Both kinds of temporary have broken this helper on x86-64:
    //
    // * A fixed `SCRATCH[k]`. The x86 scratch pool aliases the C argument bank
    //   (`map_scratch_register`): `SCRATCH[3]` is `r8`, which is `c_arg(4)`. This helper
    //   is called *between* argument stagings, so a fixed scratch overwrote an argument
    //   already in place — `vkCmdBindDescriptorSets` received the graphics-state pointer
    //   as its `descriptorSetCount` and walked a one-element array for a dozen entries.
    // * A `builder.temporary_vreg()`. The allocator does not see the hand-assigned
    //   `SCRATCH` values the emitters keep live, so it may colour the vreg onto one of
    //   them: bug-688's band index held its table offset in `SCRATCH[5]` (`r10`) across
    //   this call, the vreg came out as `r10`, and the table pointer became the state
    //   block plus 4 MiB — an access violation on Windows, and nothing on AArch64, where
    //   `x9`–`x17` are outside the allocatable file.
    let dst: abi::Reg = dst.into();
    builder.emit(abi::load_u64(dst, abi::stack_pointer(), state_slot));
    builder.emit(abi::load_u64(dst, dst, offset));
}

/// Store `value` (a whole number in a GPR) as an IEEE-754 **single** at
/// `sp + offset`.
///
/// `VkViewport` is the one place this backend needs a real `float` — everything else
/// crosses as 16.16 fixed point or a whole number. The assembler has no double→single
/// convert and no 32-bit floating-point store, which is the same gap that made the
/// item block fixed point, so the bit pattern is assembled with integer arithmetic:
/// convert to a double (which the assembler *does* have), read its bits out with
/// `fmov`, then re-lay the sign, the rebiased exponent and the top 23 mantissa bits.
///
/// Exact for the values it is given. A viewport dimension is a positive integer well
/// under 2^24, so it is representable in `float` with no rounding at all and the
/// mantissa bits being dropped are all zero. It is **not** a general
/// double-to-float — it has no rounding, no denormals, and no zero case — which is
/// why it is spelled as a private helper for this one use rather than an `abi::`
/// primitive that would invite the general one.
pub(super) fn emit_store_f32_from_integer(
    builder: &mut CodeBuilder,
    value: impl Into<abi::Reg>,
    offset: usize,
) {
    let bits = abi::SCRATCH[8];
    let sign = abi::SCRATCH[9];
    let exponent = abi::SCRATCH[10];
    let mantissa = abi::SCRATCH[11];

    builder.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[7], value));
    builder.emit(abi::float_move_x_from_d(bits, abi::FP_SCRATCH[7]));

    // sign = bits >> 63, in place at bit 31.
    builder.emit(abi::shift_right_immediate(sign, bits, 63));
    builder.emit(abi::shift_left_immediate(sign, sign, 31));

    // exponent = ((bits >> 52) & 0x7FF) - 1023 + 127, at bit 23.
    builder.emit(abi::shift_right_immediate(exponent, bits, 52));
    builder.emit(abi::move_immediate(mantissa, "Integer", "2047"));
    builder.emit(abi::and_registers(exponent, exponent, mantissa));
    builder.emit(abi::subtract_immediate(exponent, exponent, 896));
    builder.emit(abi::shift_left_immediate(exponent, exponent, 23));

    // mantissa = (bits >> 29) & 0x7FFFFF — the top 23 of the double's 52.
    builder.emit(abi::shift_right_immediate(mantissa, bits, 29));
    builder.emit(abi::move_immediate(bits, "Integer", "8388607"));
    builder.emit(abi::and_registers(mantissa, mantissa, bits));

    builder.emit(abi::or_registers(sign, sign, exponent));
    builder.emit(abi::or_registers(sign, sign, mantissa));
    builder.emit(abi::store_u32(sign, abi::stack_pointer(), offset));
}

/// Call the function pointer parked at `off_fn`.
///
/// Through a **fresh vreg** — see the module doc. Never a fixed `abi::SCRATCH[k]`.
/// Stage a zero as the `n`-th integer C argument, wherever this target passes it.
///
/// **`c_arg(n)` is not "argument n" — it is slot n of the aligned call bank**, and
/// the bank is longer than the ABI's register argument list. On x86-64 SysV the bank
/// runs `[rdi, rsi, rdx, rcx, r8, r9, rax, rbp]` but only the first six carry
/// arguments, so `c_arg(6)` writes `rax` and `c_arg(7)` writes **`rbp`** — the frame
/// pointer — while the callee reads two stack slots nothing wrote. Staging into them
/// and spilling afterwards does not help: the damage is the write itself. AArch64 and
/// riscv64 pass eight in registers, so there the same indices are exactly right, which
/// is why this is invisible on the development host.
///
/// It cost a blank frame to find. `vkCmdBindDescriptorSets` is this backend's only
/// eight-argument call; validation reported a `descriptorSetCount` in the dozens and
/// a dozen "invalid VkDescriptorSet" handles read past the end of a one-element array,
/// all of it downstream of a zeroed `rbp`.
///
/// Routed through the register model rather than a hardcoded six, so on a target whose
/// registers cover `n` the emitted bytes are the plain `c_arg(n)` staging.
pub(super) fn emit_int_arg_zero(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    n: usize,
) {
    emit_int_arg(builder, platform, n, "0");
}

/// Stage integer immediate `value` as C argument `n`.
///
/// **This is not optional past argument four on Win64.** `CALL_ARGS_WIN64` is
/// `[rcx, rdx, r8, r9, rdi, rsi, ...]`, so `c_arg(4)`/`c_arg(5)` realize to `rdi`/`rsi`
/// — registers Win64 does not pass arguments in at all. The 5th argument onward travels
/// on the stack above the shadow space, which is what `outgoing_stack_arg_store` writes.
/// SysV covers six in registers, so there the emitted bytes are the plain `c_arg(n)`
/// staging and nothing changes.
///
/// plan-98-F Phase 3 found this the hard way: `vkMapMemory` takes six arguments, and its
/// `ppData` out-param arrived in `rsi`, which lavapipe never reads — so the ICD wrote the
/// mapped pointer through whatever `ppData` happened to hold and faulted at
/// `vulkan_lvp!vk_icdGetInstanceProcAddr+0x818`, `mov [r8],rax` with `r8=8`.
pub(super) fn emit_int_arg(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    n: usize,
    value: &str,
) {
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    if n < register_args {
        builder.emit(abi::move_immediate(abi::c_arg(n), "Integer", value));
        return;
    }
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    builder.emit(abi::outgoing_stack_arg_store(
        abi::SCRATCH[0],
        n - register_args,
    ));
}

/// Stage the integer **stored at** stack offset `offset` as C argument `n`.
///
/// The load-from-memory counterpart of [`emit_int_arg`], and the reason it exists is
/// the hazard in that function's own doc turned around: an argument whose value is
/// computed rather than constant cannot wait in a scratch register while the other
/// arguments are staged, because on x86-64 the scratch pool aliases the argument bank.
/// Parking it on the stack and loading it straight into its argument register makes
/// the staging order irrelevant. Same register-model rule as [`emit_int_arg`].
pub(super) fn emit_int_arg_slot(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    n: usize,
    offset: usize,
) {
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    if n < register_args {
        builder.emit(abi::load_u64(abi::c_arg(n), abi::stack_pointer(), offset));
        return;
    }
    builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), offset));
    builder.emit(abi::outgoing_stack_arg_store(
        abi::SCRATCH[0],
        n - register_args,
    ));
}

/// Stage the address of the stack object at `offset` as C argument `n` — the
/// `&out` / `&structure` shape every Vulkan out-parameter uses. Same register-model
/// rule as [`emit_int_arg`].
pub(super) fn emit_addr_arg(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    n: usize,
    offset: usize,
) {
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    if n < register_args {
        builder.emit(abi::add_immediate(
            abi::c_arg(n),
            abi::stack_pointer(),
            offset,
        ));
        return;
    }
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        offset,
    ));
    builder.emit(abi::outgoing_stack_arg_store(
        abi::SCRATCH[0],
        n - register_args,
    ));
}

pub(super) fn emit_call_fn(builder: &mut CodeBuilder, off_fn: usize) {
    let callee = builder.temporary_vreg();
    builder.emit(abi::load_u64(&callee, abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(&callee));
}

/// `fn = dlsym(handle, "<name>")` into `off_fn`; branch away if null.
///
/// Every entry point is resolved from the loader library directly rather than
/// bootstrapped through `vkGetInstanceProcAddr`. The spec only *guarantees* that
/// `vkGetInstanceProcAddr` is exported, so the bootstrap is the textbook route — but
/// measured on box 2228 (loader 1.4.309), `vkGetInstanceProcAddr(NULL,
/// "vkCreateInstance")` returned NULL where `dlsym(handle, "vkCreateInstance")`
/// returned a working pointer, and `libvulkan.so.1` exports every core entry point by
/// name.
///
/// It is also the simpler shape: `dlsym` goes through `emit_external_call`, the path
/// the `audio` backend has proven on both libcs, instead of a hand-staged call
/// through a resolved pointer. The one thing this must not become is a silent
/// fallback — a name that is genuinely absent still returns FALSE from the probe,
/// which is what "no Vulkan here" is supposed to mean.
#[allow(clippy::too_many_arguments)]
/// Open the Vulkan loader library, leaving the handle in `c_return(0)` (0 on failure).
///
/// POSIX takes `dlopen(name, RTLD_NOW)`. Windows takes `LoadLibraryExA(name, NULL, 0)`:
/// a three-argument call, `hFile` reserved and required to be NULL, and flags 0 for the
/// **default search order** — this is a system library found in `System32`, not one of
/// the exe-relative `vendor/` DLLs that `emit_lib_open` builds an absolute path for.
///
/// Deliberately NOT routed through the `emit_lib_open` platform hook the `LINK` loader
/// uses. That hook answers in `return_register()`, the aligned MFB bank, while every
/// check and store in this backend reads `c_return(0)` — and those are *different
/// registers on Win64* (`rcx` vs `rax`) and the same one on AArch64, which is exactly
/// the class of defect bug-478/479 spent eight fixes on. Answering in `c_return(0)`
/// here keeps this file's single convention.
pub(super) fn emit_open_library(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    let windows = matches!(platform.family(), PlatformFamily::Windows);
    emit_data_address(
        &symbol,
        abi::c_arg(0),
        &soname_symbol(),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    if windows {
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0")); // hFile, reserved
        builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0")); // default search
    } else {
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    }
    platform.emit_external_call(
        if windows { "LoadLibraryExA" } else { "dlopen" },
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )
}

pub(super) fn emit_dlsym(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    name: &str,
    off_handle: usize,
    off_fn: usize,
    unavailable: &str,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_handle,
    ));
    emit_data_address(
        &symbol,
        abi::c_arg(1),
        &symbol_name_symbol(name),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    // `GetProcAddress(hModule, lpProcName)` is `dlsym`'s exact shape — two arguments in
    // the same order, the address in the C result, NULL when the name is absent — so
    // only the callee name changes.
    platform.emit_external_call(
        if matches!(platform.family(), PlatformFamily::Windows) {
            "GetProcAddress"
        } else {
            "dlsym"
        },
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(unavailable));
    builder.emit(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_fn,
    ));
    Ok(())
}

/// Zero `length` bytes at `sp + base`, eight at a time.
///
/// Vulkan's create-info structs are mostly zero, and a *reserved* field left holding
/// stack garbage is not ignored — `flags` and `pNext` in particular are read. Zeroing
/// the whole range and then writing only the named members is what makes the field
/// tables above complete rather than merely representative.
pub(super) fn emit_zero_range(builder: &mut CodeBuilder, base: usize, length: usize) {
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for offset in (base..base + length).step_by(8) {
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            offset,
        ));
    }
}
