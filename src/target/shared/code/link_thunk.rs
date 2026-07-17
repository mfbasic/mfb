//! Native code generation for user `LINK` bindings (plan-linker.md §12).
//!
//! Each `LINK "lib" AS alias` block contributes:
//!
//! - a per-program load-time initializer (`_mfb_linker_init`) that `dlopen`s each
//!   distinct library and `dlsym`s every declared symbol into a per-function
//!   global pointer slot (§12.1), aborting before `main` on any failure;
//! - one MFB↔C marshaling thunk per `FUNC` (`_mfb_linker_<alias>_<name>`, §12.2)
//!   that marshals arguments per §12.3, calls through the resolved pointer, then
//!   marshals the native return and any `OUT` slots back, applying `SUCCESS_ON`
//!   and `RESULT`.
//!
//! The resolved function pointers live in the program's writable global region
//! (addressed `x19 + ENTRY_GLOBALS_OFFSET + slot*8`), reserved immediately after
//! the program's own globals. Read-only C strings (library filenames and symbol
//! names) live in the constant data section.

use std::collections::HashMap;

use super::link_locator::LinkLibraries;
use super::*;
use crate::ir::{IrLinkExpr, IrLinkFunction};
use crate::target::shared::abi;
use crate::target::shared::nir::{self, link_thunk_symbol};

/// The generated functions and data objects backing the program's `LINK`
/// bindings.
pub(super) struct LinkSupport {
    pub(super) functions: Vec<CodeFunction>,
    pub(super) data_objects: Vec<CodeDataObject>,
}

/// A read-only NUL-terminated C string constant.
fn cstring_object(symbol: &str, text: &str) -> CodeDataObject {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "c-string { u8 bytes[]; u8 nul }".to_string(),
        align: 1,
        size: bytes.len(),
        value,
    }
}

fn lib_symbol(index: usize) -> String {
    format!("_mfb_linker_lib_{index}")
}

fn sym_symbol(index: usize) -> String {
    format!("_mfb_linker_sym_{index}")
}

/// Read-only constant naming the `k`-th `FREE` deallocator symbol (e.g.
/// `sqlite3_free`), resolved by `dlsym` into a slot past the per-function slots.
fn free_sym_symbol(k: usize) -> String {
    format!("_mfb_linker_free_{k}")
}

/// The writable global slot (relative to `x19`) holding the resolved pointer for
/// the `index`-th `LINK` function. Reserved after the program's `globals_base`
/// own global slots.
fn slot_offset(globals_base: usize, index: usize) -> usize {
    ENTRY_GLOBALS_OFFSET + (globals_base + index) * 8
}

fn internal_reloc(from: &str, to: &str) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }
}

/// Emit `adrp`/`add` to materialize the address of data `symbol` into `dst`.
fn emit_data_address(
    from: &str,
    dst: &str,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::load_page_address(dst, symbol));
    instructions.push(abi::add_page_offset(dst, dst, symbol));
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// Emit `bl _mfb_arena_alloc` (size in `x0`, align in `x1`); on success the block
/// pointer is in `x1`. Branches to `fail` on allocation failure.
fn emit_alloc(
    from: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    fail: &str,
) {
    instructions.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    relocations.push(internal_reloc(from, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(fail),
    ]);
}

/// Build the full `LINK` support: the load-time initializer, one thunk per
/// function, and the backing data objects.
pub(super) fn emit_link_support(
    link_functions: &[IrLinkFunction],
    globals_base: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    libraries: &LinkLibraries,
) -> Result<LinkSupport, String> {
    let mut data_objects = Vec::new();

    // Distinct libraries in declaration order, each mapped to a constant symbol.
    let mut library_index: Vec<String> = Vec::new();
    for function in link_functions {
        if !library_index.iter().any(|lib| lib == &function.library) {
            library_index.push(function.library.clone());
        }
    }
    // plan-46-C: the `dlopen` filename is the binding author's declared `source`
    // for this build's exact (os, arch, libc) — resolved from the imported
    // binding's section-10 table, never synthesized. The old `library_filename`
    // guess (`lib{logical}.so.0` / `lib{logical}.dylib`) is gone: it never
    // consulted the manifest and missed every unversioned `.so`, `.so.3`,
    // non-`lib`-prefixed, or per-arch/libc variant.
    for (index, library) in library_index.iter().enumerate() {
        let resolved = libraries.get(library)?;
        data_objects.push(cstring_object(&lib_symbol(index), &resolved.dlopen_name));
    }
    // One symbol-name constant per function (indexed by position).
    for (index, function) in link_functions.iter().enumerate() {
        data_objects.push(cstring_object(&sym_symbol(index), &function.symbol));
    }

    // Each `FREE` block resolves its deallocator into a slot reserved past the
    // per-function slots. `free_index_of[i]` is the function's deallocator index
    // `k` (so its slot is `link_count + k`), or `None` when it has no FREE.
    let link_count = link_functions.len();
    let mut free_index_of: Vec<Option<usize>> = Vec::with_capacity(link_count);
    let mut free_count = 0usize;
    for function in link_functions {
        if let Some(free) = &function.free {
            data_objects.push(cstring_object(&free_sym_symbol(free_count), &free.symbol));
            free_index_of.push(Some(free_count));
            free_count += 1;
        } else {
            free_index_of.push(None);
        }
    }

    let initializer = lower_link_initializer(
        link_functions,
        &library_index,
        globals_base,
        link_count,
        &free_index_of,
        platform_imports,
        platform,
    )?;
    let mut functions = vec![initializer];
    for (index, function) in link_functions.iter().enumerate() {
        let free_slot = free_index_of[index].map(|k| link_count + k);
        functions.push(lower_link_thunk(function, index, globals_base, free_slot)?);
    }

    Ok(LinkSupport {
        functions,
        data_objects,
    })
}

/// Emit `_mfb_linker_init`: `dlopen` each library, `dlsym` each symbol into its
/// global slot. Returns the standard `(tag, value, message)` result so the
/// program entry handles a load failure exactly like a global-initializer error.
fn lower_link_initializer(
    link_functions: &[IrLinkFunction],
    library_index: &[String],
    globals_base: usize,
    link_count: usize,
    free_index_of: &[Option<usize>],
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2). The only value held across the
    // `dlopen`/`dlsym` libc calls is the library `handle`; as a vreg the allocator
    // keeps it in a callee-saved register across the calls (the calls are libc =
    // PCS, so callee-saved survives) instead of the old manual stack slot. `x19`
    // (arena_base, where the resolved slots land) and the libc ABI registers
    // (x0/x1) stay physical.
    let symbol = nir::LINK_INIT_SYMBOL;
    let fail = format!("{symbol}_fail");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let handle = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    for (lib_idx, library) in library_index.iter().enumerate() {
        // handle = dlopen(filename, RTLD_NOW)
        emit_data_address(
            symbol,
            abi::return_register(),
            &lib_symbol(lib_idx),
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::move_immediate(abi::ARG[1], "Integer", "2")); // RTLD_NOW
        platform.emit_libc_call(
            "dlopen",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&fail),
            abi::move_register(&handle, abi::return_register()),
        ]);
        for (fn_idx, function) in link_functions.iter().enumerate() {
            if &function.library != library {
                continue;
            }
            // slot = dlsym(handle, symbolName)
            instructions.push(abi::move_register(abi::return_register(), &handle));
            emit_data_address(
                symbol,
                abi::ARG[1],
                &sym_symbol(fn_idx),
                &mut instructions,
                &mut relocations,
            );
            platform.emit_libc_call(
                "dlsym",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_eq(&fail),
                abi::store_u64(
                    abi::return_register(),
                    ARENA_STATE_REGISTER,
                    slot_offset(globals_base, fn_idx),
                ),
            ]);
            // A FREE deallocator lives in the same library; resolve it into its
            // own slot (reserved past the per-function slots).
            if let Some(k) = free_index_of[fn_idx] {
                instructions.push(abi::move_register(abi::return_register(), &handle));
                emit_data_address(
                    symbol,
                    abi::ARG[1],
                    &free_sym_symbol(k),
                    &mut instructions,
                    &mut relocations,
                );
                platform.emit_libc_call(
                    "dlsym",
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
                instructions.extend([
                    abi::compare_immediate(abi::return_register(), "0"),
                    abi::branch_eq(&fail),
                    abi::store_u64(
                        abi::return_register(),
                        ARENA_STATE_REGISTER,
                        slot_offset(globals_base, link_count + k),
                    ),
                ]);
            }
        }
    }

    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NATIVE_LINK_LOAD_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    emit_data_address(
        symbol,
        RESULT_ERROR_MESSAGE_REGISTER,
        ERR_NATIVE_LINK_LOAD_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);

    Ok(finalize_vreg_helper(
        "linker.init",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// Emit one marshaling thunk for a `LINK` function (plan-linker.md §12.2/§12.3).
fn lower_link_thunk(
    function: &IrLinkFunction,
    index: usize,
    globals_base: usize,
    free_slot: Option<usize>,
) -> Result<CodeFunction, String> {
    let symbol = link_thunk_symbol(&function.alias, &function.name);
    let n_params = function.params.len();
    let m_slots = function.abi_slots.len();
    let n_out = function
        .abi_slots
        .iter()
        .filter(|slot| slot.direction.writes_back())
        .count();

    const STATUS_OFF: usize = 8;
    const CRET_OFF: usize = 16;
    // Scratch slot 24 is reserved for string-return marshaling (RET_OFF).
    let param_base = 32;
    let cslot_base = param_base + n_params * 8;
    let out_base = cslot_base + m_slots * 8;
    // One extra slot past the OUT buffers holds the floating-point return bits
    // (`d0`) when the native return is `CDouble`.
    let cretd_off = out_base + n_out * 8;
    let frame = align(cretd_off + 8 + 24, 16);

    // plan-50-H: the wrapper's result is whatever `RETURN <expr>` names. A bare
    // `RETURN <slot>` (an `IrLinkExpr::Var`) selects that slot's value; anything
    // else is a computed expression. Both magic-name tests — `slot.name ==
    // "return"` and `abi_return_name == "return"` — are gone.
    let result_var: Option<&str> = match &function.result {
        Some(IrLinkExpr::Var(name)) => Some(name.as_str()),
        _ => None,
    };

    // §12.3/§12.4 boundary validations that this signature needs.
    // The C return is the result exactly when `RETURN` names it.
    let returns_value = result_var == Some(function.abi_return_name.as_str());
    let needs_range = function.abi_slots.iter().any(|slot| {
        !slot.direction.writes_back()
            && slot.ctype == "CInt32"
            && function.params.iter().any(|(name, _)| name == &slot.name)
    });
    let needs_encoding =
        returns_value && function.abi_return_ctype == "CPtr" && function.return_type == "String";
    let needs_float = returns_value && function.abi_return_ctype == "CDouble";

    let alloc_fail = format!("{symbol}_alloc_fail");
    let call_fail = format!("{symbol}_call_fail");
    let range_fail = format!("{symbol}_range_fail");
    let encoding_fail = format!("{symbol}_encoding_fail");
    let nan_fail = format!("{symbol}_nan_fail");
    let inf_fail = format!("{symbol}_inf_fail");
    let done = format!("{symbol}_done");

    // Map wrapper-parameter name -> declared order (its incoming register).
    let param_index: HashMap<&str, usize> = function
        .params
        .iter()
        .enumerate()
        .map(|(idx, (name, _))| (name.as_str(), idx))
        .collect();
    // Map const-pin slot name -> immediate.
    let const_for: HashMap<&str, i64> = function
        .consts
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect();

    // Vreg-allocated (plan-00-G Phase 2): the C-ABI marshaling slots are an
    // explicit `sp`-relative local region; x9/x10/x16 scratch become vregs the
    // allocator places (incoming wrapper args stay in their ABI x0-x7).
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Save incoming wrapper arguments before any clobbering call.
    for index in 0..n_params {
        instructions.push(abi::store_u64(
            &abi::argument_register(index)?,
            abi::stack_pointer(),
            param_base + index * 8,
        ));
    }

    // Compute each C argument into its scratch slot. OUT buffers are addressed,
    // const pins are pinned, and ordinary params are marshaled per ABI type.
    let mut out_seq = 0usize;
    let mut result_out_off: Option<usize> = None;
    // The OUT-slot result's ctype drives the same marshalling the direct-return
    // path applies (bug-238); without it a `CInt32` OUT surfaced -1 as 4294967295.
    let mut result_out_ctype: Option<String> = None;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let cslot_off = cslot_base + slot_idx * 8;
        if slot.direction.writes_back() {
            let out_off = out_base + out_seq * 8;
            out_seq += 1;
            instructions.extend([
                abi::store_u64(abi::ZERO, abi::stack_pointer(), out_off),
                abi::add_immediate("%v9", abi::stack_pointer(), out_off),
                abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
            ]);
            if result_var == Some(slot.name.as_str()) {
                result_out_off = Some(out_off);
                result_out_ctype = Some(slot.ctype.clone());
            }
        } else if let Some(value) = const_for.get(slot.name.as_str()) {
            // §12.3: a `CInt32` slot is a signed 32-bit C argument. A param feeding
            // this slot is range-checked at runtime (`range_fail` → `ErrOverflow`);
            // a `CONST` pin is known at compile time, so an out-of-range value is
            // rejected here rather than silently truncated to its low 32 bits
            // (bug-66).
            if slot.ctype == "CInt32"
                && (*value < i64::from(i32::MIN) || *value > i64::from(i32::MAX))
            {
                return Err(format!(
                    "LINK function '{}.{}' CONST pin '{} = {}' does not fit the signed 32-bit \
                     range of its CInt32 ABI slot",
                    function.alias, function.name, slot.name, value
                ));
            }
            instructions.extend([
                abi::move_immediate("%v9", "Integer", &(*value as u64).to_string()),
                abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
            ]);
        } else if let Some(&pidx) = param_index.get(slot.name.as_str()) {
            let param_off = param_base + pidx * 8;
            if slot.ctype == "CString" {
                emit_copy_string_to_cstring(
                    &symbol,
                    param_off,
                    cslot_off,
                    &alloc_fail,
                    &mut instructions,
                    &mut relocations,
                );
            } else if slot.ctype == "CInt32" {
                // §12.3: the 64-bit MFBASIC Integer must fit signed 32-bit; an
                // out-of-range value fails rather than silently truncating.
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), param_off),
                    abi::shift_left_immediate("%v10", "%v9", 32),
                    abi::arithmetic_shift_right_immediate("%v10", "%v10", 32),
                    abi::compare_registers("%v9", "%v10"),
                    abi::branch_ne(&range_fail),
                    abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
                ]);
            } else {
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), param_off),
                    abi::store_u64("%v9", abi::stack_pointer(), cslot_off),
                ]);
            }
        } else {
            return Err(format!(
                "LINK function '{}.{}' ABI slot '{}' has no source (param, const, or OUT)",
                function.alias, function.name, slot.name
            ));
        }
    }

    // Load the C arguments into their AAPCS64 registers, then call through the
    // resolved pointer.
    let mut int_idx = 0usize;
    let mut flt_idx = 0usize;
    for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
        let cslot_off = cslot_base + slot_idx * 8;
        if slot.ctype == "CDouble" {
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), cslot_off),
                abi::float_move_d_from_x(abi::fp_argument_register(flt_idx)?, "%v9"),
            ]);
            flt_idx += 1;
        } else {
            instructions.push(abi::load_u64(
                &abi::argument_register(int_idx)?,
                abi::stack_pointer(),
                cslot_off,
            ));
            int_idx += 1;
        }
    }
    instructions.extend([
        abi::load_u64(
            "%v16",
            ARENA_STATE_REGISTER,
            slot_offset(globals_base, index),
        ),
        abi::branch_link_register("%v16"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CRET_OFF),
    ]);
    if needs_float {
        // A `double` return arrives in `d0`, not `x0`; stash its bits.
        instructions.extend([
            abi::float_move_x_from_d("%v9", abi::FP_SCRATCH[0]),
            abi::store_u64("%v9", abi::stack_pointer(), cretd_off),
        ]);
    }

    // Derive the status value (sign-extending a 32-bit native return).
    instructions.push(abi::load_u64("%v9", abi::stack_pointer(), CRET_OFF));
    if function.abi_return_ctype == "CInt32" {
        instructions.extend([
            abi::shift_left_immediate("%v9", "%v9", 32),
            abi::arithmetic_shift_right_immediate("%v9", "%v9", 32),
        ]);
    }
    instructions.push(abi::store_u64("%v9", abi::stack_pointer(), STATUS_OFF));

    // plan-50-I: every name a `Var` may hold, mapped to the frame slot holding
    // its value. The ABI return maps to STATUS_OFF, NOT CRET_OFF — STATUS_OFF
    // holds the sign-extended value, which is what `SUCCESS_ON status = -1` must
    // compare against.
    let mut expr_offsets: HashMap<&str, usize> = HashMap::new();
    expr_offsets.insert(function.abi_return_name.as_str(), STATUS_OFF);
    {
        let mut seq = 0usize;
        for (slot_idx, slot) in function.abi_slots.iter().enumerate() {
            let off = if slot.direction.writes_back() {
                let o = out_base + seq * 8;
                seq += 1;
                o
            } else {
                cslot_base + slot_idx * 8
            };
            // The ABI return name wins a collision: it is the older meaning and
            // `SUCCESS_ON status = 0` must keep resolving to the status.
            expr_offsets.entry(slot.name.as_str()).or_insert(off);
        }
    }

    // One label counter shared across the SUCCESS_ON gate and the RESULT
    // expression: a per-block counter restarted at 0 in each, so a thunk whose
    // SUCCESS_ON and RESULT both emit a comparison/NOT produced two identically
    // named labels (`{symbol}_cmp0_end`) — a duplicate the encoder now rejects
    // outright (bug-79).
    let mut counter = 0usize;

    // SUCCESS_ON gate: a failing status produces an Error result.
    if let Some(success) = &function.success_on {
        let mut vreg = LINK_EXPR_VREG_BASE;
        let value = emit_link_expr(
            success,
            &expr_offsets,
            &mut vreg,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.extend([
            abi::compare_immediate(&value, "0"),
            abi::branch_eq(&call_fail),
        ]);
    }

    // Produce the wrapper result value in RESULT_VALUE_REGISTER (x1).
    if let Some(out_off) = result_out_off {
        // bug-238: an OUT-slot result carries the same C value shapes as a direct
        // return, so apply the same ctype-driven marshalling instead of a bare
        // 8-byte load — otherwise a `CInt32` OUT writing -1 surfaced as
        // 4294967295 (zero-extended) and a `CDouble` OUT bypassed the finiteness
        // rejection an MFBASIC `Float` requires.
        match result_out_ctype.as_deref().unwrap_or("") {
            "CInt32" => {
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::sign_extend_word(RESULT_VALUE_REGISTER, "%v9"),
                ]);
            }
            "CBool" => {
                let set = format!("{symbol}_out_bool_true");
                let end = format!("{symbol}_out_bool_end");
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::compare_immediate("%v9", "0"),
                    abi::branch_ne(&set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                    abi::branch(&end),
                    abi::label(&set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                    abi::label(&end),
                ]);
            }
            "CByte" => {
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::move_immediate("%v10", "Integer", "255"),
                    abi::and_registers(RESULT_VALUE_REGISTER, "%v9", "%v10"),
                ]);
            }
            "CDouble" => {
                // Mirrors the direct-return finiteness gate: a non-finite double has
                // all exponent bits set; the mantissa distinguishes Inf from NaN.
                let finite = format!("{symbol}_out_float_finite");
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), out_off),
                    abi::move_immediate("%v10", "Integer", "9218868437227405312"),
                    abi::and_registers("%v11", "%v9", "%v10"),
                    abi::compare_registers("%v11", "%v10"),
                    abi::branch_ne(&finite),
                    abi::move_immediate("%v12", "Integer", "4503599627370495"),
                    abi::and_registers("%v13", "%v9", "%v12"),
                    abi::compare_immediate("%v13", "0"),
                    abi::branch_eq(&inf_fail),
                    abi::branch(&nan_fail),
                    abi::label(&finite),
                    abi::move_register(RESULT_VALUE_REGISTER, "%v9"),
                ]);
            }
            _ => {
                instructions.push(abi::load_u64(
                    RESULT_VALUE_REGISTER,
                    abi::stack_pointer(),
                    out_off,
                ));
            }
        }
    } else if result_var == Some(function.abi_return_name.as_str()) {
        emit_return_passthrough(
            function,
            ReturnMarshal {
                cret_off: CRET_OFF,
                cretd_off,
                status_off: STATUS_OFF,
                alloc_fail: &alloc_fail,
                encoding_fail: &encoding_fail,
                nan_fail: &nan_fail,
                inf_fail: &inf_fail,
            },
            &symbol,
            &mut instructions,
            &mut relocations,
        )?;
    } else if let Some(result) = &function.result {
        let mut vreg = LINK_EXPR_VREG_BASE;
        let value = emit_link_expr(
            result,
            &expr_offsets,
            &mut vreg,
            &symbol,
            &mut counter,
            &mut instructions,
        );
        instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &value));
    } else {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    }

    // FREE: release the caller-owned native return now that it is copied into the
    // owned wrapper result (mfbasic.md §17). The original pointer is still at
    // CRET_OFF; the deallocator (a C call) clobbers x0..x18, so the result value
    // is parked in the now-free STATUS slot across the call and reloaded after.
    // A NULL pointer is passed through unchanged — deallocators such as
    // sqlite3_free treat NULL as a no-op.
    if let Some(free_slot) = free_slot {
        instructions.extend([
            abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STATUS_OFF),
            abi::load_u64(&abi::argument_register(0)?, abi::stack_pointer(), CRET_OFF),
            abi::load_u64(
                "%v16",
                ARENA_STATE_REGISTER,
                slot_offset(globals_base, free_slot),
            ),
            abi::branch_link_register("%v16"),
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STATUS_OFF),
        ]);
    }

    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // call_fail: SUCCESS_ON rejected the native status.
    instructions.extend([
        abi::label(&call_fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NATIVE_LINK_CALL_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    emit_data_address(
        &symbol,
        RESULT_ERROR_MESSAGE_REGISTER,
        ERR_NATIVE_LINK_CALL_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&done));

    // alloc_fail: a marshaling allocation failed.
    instructions.extend([
        abi::label(&alloc_fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    emit_data_address(
        &symbol,
        RESULT_ERROR_MESSAGE_REGISTER,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );

    // Boundary-validation failure epilogues (plan-linker.md §12.3/§12.4), emitted
    // only when the signature can reach them.
    for (needed, label, code, message) in [
        (
            needs_range,
            &range_fail,
            ERR_OVERFLOW_CODE,
            ERR_OVERFLOW_SYMBOL,
        ),
        (
            needs_encoding,
            &encoding_fail,
            ERR_ENCODING_CODE,
            ERR_ENCODING_SYMBOL,
        ),
        (
            needs_float,
            &nan_fail,
            ERR_FLOAT_NAN_CODE,
            ERR_FLOAT_NAN_SYMBOL,
        ),
        (
            needs_float,
            &inf_fail,
            ERR_FLOAT_INF_CODE,
            ERR_FLOAT_INF_SYMBOL,
        ),
    ] {
        if !needed {
            continue;
        }
        instructions.push(abi::branch(&done));
        instructions.extend([
            abi::label(label),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        emit_data_address(
            &symbol,
            RESULT_ERROR_MESSAGE_REGISTER,
            message,
            &mut instructions,
            &mut relocations,
        );
    }

    instructions.extend([abi::label(&done), abi::return_()]);

    let (frame_obj, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], frame);
    Ok(CodeFunction {
        name: format!("linker.{}.{}", function.alias, function.name),
        symbol,
        params: function
            .params
            .iter()
            .enumerate()
            .map(|(idx, (name, type_))| {
                Ok(CodeParam {
                    name: name.clone(),
                    type_: type_.clone(),
                    // The wrapper's incoming MFB argument register, as a role
                    // token — the thunk body saves from the same bank
                    // (plan-34-D; ≤8 params, enforced by `argument_register`).
                    location: abi::argument_register(idx)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        returns: function.return_type.clone(),
        frame: frame_obj,
        stack_slots,
        instructions,
        relocations,
    })
}

/// Frame slots and failure labels the return marshaler needs.
struct ReturnMarshal<'a> {
    cret_off: usize,
    cretd_off: usize,
    status_off: usize,
    alloc_fail: &'a str,
    encoding_fail: &'a str,
    nan_fail: &'a str,
    inf_fail: &'a str,
}

/// Marshal the native return (`AS return <ctype>`) into the wrapper result in
/// `RESULT_VALUE_REGISTER` (plan-linker.md §12.3/§12.4).
fn emit_return_passthrough(
    function: &IrLinkFunction,
    m: ReturnMarshal,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let cret_off = m.cret_off;
    let status_off = m.status_off;
    match function.abi_return_ctype.as_str() {
        "CPtr" if function.return_type == "String" => {
            emit_copy_cstring_to_string(
                symbol,
                cret_off,
                m.alloc_fail,
                m.encoding_fail,
                instructions,
                relocations,
            );
        }
        "CDouble" => {
            // §12.3: a C `double` may be NaN/Inf, but an MFBASIC `Float` is always
            // finite (mfbasic.md §3), so reject non-finite results at the boundary.
            // A non-finite double has all exponent bits set (`0x7FF0…`); the
            // mantissa then distinguishes Inf (zero) from NaN (non-zero).
            let finite = format!("{symbol}_float_finite");
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), m.cretd_off),
                abi::move_immediate("%v10", "Integer", "9218868437227405312"),
                abi::and_registers("%v11", "%v9", "%v10"),
                abi::compare_registers("%v11", "%v10"),
                abi::branch_ne(&finite),
                abi::move_immediate("%v12", "Integer", "4503599627370495"),
                abi::and_registers("%v13", "%v9", "%v12"),
                abi::compare_immediate("%v13", "0"),
                abi::branch_eq(m.inf_fail),
                abi::branch(m.nan_fail),
                abi::label(&finite),
                abi::move_register(RESULT_VALUE_REGISTER, "%v9"),
            ]);
        }
        "CPtr" | "CInt64" => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                cret_off,
            ));
        }
        "CInt32" => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                status_off,
            ));
        }
        "CBool" => {
            let set = format!("{symbol}_bool_true");
            let end = format!("{symbol}_bool_end");
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), cret_off),
                abi::compare_immediate("%v9", "0"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::branch(&end),
                abi::label(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&end),
            ]);
        }
        "CByte" => {
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), cret_off),
                abi::move_immediate("%v10", "Integer", "255"),
                abi::and_registers(RESULT_VALUE_REGISTER, "%v9", "%v10"),
            ]);
        }
        // The narrow integers carry no return-side normalization: the C ABI
        // already delivers them in the low bits of the return register, and the
        // wrapper's MFBASIC type is `Integer`. Listed explicitly (rather than
        // left to a default arm) so an unknown ctype is a hard error — plan-50-A.
        "CInt8" | "CInt16" | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64" | "CFloat" => {
            instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                cret_off,
            ));
        }
        other => {
            // Unreachable: `abi_slot_ctype_is_known` gates this at syntaxcheck and
            // at `ir::verify` (both paths). This exists so a ctype added to the
            // allow-list without a marshaling arm fails loudly at build time
            // instead of silently moving a raw 64-bit value.
            return Err(format!(
                "LINK function '{}.{}' has unknown ABI return ctype '{other}'",
                function.alias, function.name
            ));
        }
    }
    Ok(())
}

/// Copy the NUL-free MFBASIC `String` at `[sp + str_off]` into a freshly arena
/// allocated NUL-terminated C buffer, storing the pointer at `[sp + out_off]`.
fn emit_copy_string_to_cstring(
    symbol: &str,
    str_off: usize,
    out_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let loop_label = format!("{symbol}_cs{out_off}_copy");
    let done_label = format!("{symbol}_cs{out_off}_done");
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), str_off),
        abi::load_u64("%v10", "%v9", 0),
        abi::add_immediate(abi::return_register(), "%v10", 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), out_off),
        abi::load_u64("%v9", abi::stack_pointer(), str_off),
        abi::load_u64("%v10", "%v9", 0),
        abi::add_immediate("%v11", "%v9", 8),
        abi::move_register("%v12", abi::RET[1]),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&loop_label),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&done_label),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&loop_label),
        abi::label(&done_label),
        abi::store_u8(abi::ZERO, "%v12", 0),
    ]);
}

/// Copy a NUL-terminated C string at `[sp + cret_off]` into an owned MFBASIC
/// `String`, leaving the result pointer in `RESULT_VALUE_REGISTER`
/// (plan-linker.md §12.4 copy-and-leave). A NULL pointer yields an empty
/// `String`.
fn emit_copy_cstring_to_string(
    symbol: &str,
    cret_off: usize,
    alloc_fail: &str,
    encoding_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let null_label = format!("{symbol}_ret_null");
    let len_loop = format!("{symbol}_ret_len");
    let len_done = format!("{symbol}_ret_len_done");
    let copy_loop = format!("{symbol}_ret_copy");
    let copy_done = format!("{symbol}_ret_copy_done");
    let ret_done = format!("{symbol}_ret_done");
    const RET_OFF: usize = 24; // RESULT_SAVE_OFF in the thunk frame
    const LEN_OFF: usize = 8; // STATUS slot is free here (status already gated)
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), cret_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&null_label),
        // strlen
        abi::move_register("%v12", "%v9"),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&len_loop),
        abi::load_u8("%v11", "%v12", 0),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&len_done),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v10", "%v10", 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64("%v10", abi::stack_pointer(), LEN_OFF),
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), LEN_OFF),
        abi::store_u64("%v10", abi::RET[1], 0),
        abi::store_u64(abi::RET[1], abi::stack_pointer(), RET_OFF),
        abi::load_u64("%v11", abi::stack_pointer(), cret_off),
        abi::add_immediate("%v12", abi::RET[1], 8),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
        // §12.4: returned bytes are validated as UTF-8 at the boundary.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RET_OFF),
        abi::add_immediate(abi::return_register(), abi::return_register(), 8),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), LEN_OFF),
    ]);
    emit_call_validate_utf8(symbol, encoding_fail, instructions, relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RET_OFF),
        abi::branch(&ret_done),
        // NULL -> empty string [u64 0][nul].
        abi::label(&null_label),
        abi::move_immediate(abi::return_register(), "Integer", "9"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], 0),
        abi::store_u8(abi::ZERO, abi::RET[1], 8),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::label(&ret_done),
    ]);
}

/// First virtual-register index `emit_link_expr` hands out. The thunk body uses
/// fixed scratch vregs `%v9`..`%v16`; starting the expression's own vregs well
/// past that window guarantees the two name spaces never overlap, so the shared
/// linear-scan allocator sees each expression temporary as an independent value
/// (bug-56).
const LINK_EXPR_VREG_BASE: usize = 64;

/// Emit code computing the boolean/integer value of a `SUCCESS_ON`/`RESULT`
/// expression, reading the native return variable from `[sp + status_off]`.
///
/// Every intermediate is a fresh virtual register (`%vN`, allocated from `vreg`)
/// that the shared linear-scan allocator places and spills — the same discipline
/// as the rest of the thunk (plan-00-G). This replaces the historical scheme that
/// hand-assigned escalating *physical* registers (`x{base}`, `+2`, `+4`) with no
/// bound: a moderately right-nested tree walked `base` up into `x19`, the pinned
/// arena-base register, corrupting the arena program-wide (bug-56). Vregs never
/// touch reserved/callee-saved registers the thunk does not save. Returns the
/// name of the vreg holding the expression's value (0/1 for comparisons).
/// Emit a `SUCCESS_ON`/`RESULT` expression, resolving each `Var(name)` through
/// `offsets` to the frame slot holding that value (plan-50-I).
///
/// `offsets` maps every ABI slot name, and the ABI return name, to its frame
/// offset. An absent name is unreachable: both checkers reject a `Var` naming no
/// slot before codegen runs.
fn emit_link_expr(
    expr: &IrLinkExpr,
    offsets: &HashMap<&str, usize>,
    vreg: &mut usize,
    symbol: &str,
    counter: &mut usize,
    instructions: &mut Vec<CodeInstruction>,
) -> String {
    let dst = format!("%v{vreg}");
    *vreg += 1;
    match expr {
        IrLinkExpr::Int(value) => {
            instructions.push(abi::move_immediate(
                &dst,
                "Integer",
                &(*value as u64).to_string(),
            ));
        }
        IrLinkExpr::Var(name) => {
            let off = offsets.get(name.as_str()).copied().unwrap_or_else(|| {
                unreachable!("LINK expr names slot `{name}`, which verification should have rejected")
            });
            instructions.push(abi::load_u64(&dst, abi::stack_pointer(), off));
        }
        IrLinkExpr::Not(inner) => {
            let inner_reg = emit_link_expr(inner, offsets, vreg, symbol, counter, instructions);
            let id = *counter;
            *counter += 1;
            let set = format!("{symbol}_not{id}_zero");
            let end = format!("{symbol}_not{id}_end");
            instructions.extend([
                abi::compare_immediate(&inner_reg, "0"),
                abi::branch_eq(&set),
                abi::move_immediate(&dst, "Integer", "0"),
                abi::branch(&end),
                abi::label(&set),
                abi::move_immediate(&dst, "Integer", "1"),
                abi::label(&end),
            ]);
        }
        IrLinkExpr::Compare { op, lhs, rhs } => {
            let lhs_reg = emit_link_expr(lhs, offsets, vreg, symbol, counter, instructions);
            let rhs_reg = emit_link_expr(rhs, offsets, vreg, symbol, counter, instructions);
            let id = *counter;
            *counter += 1;
            let end = format!("{symbol}_cmp{id}_end");
            let branch = match op.as_str() {
                "=" => abi::branch_eq(&end),
                "<>" => abi::branch_ne(&end),
                "<" => abi::branch_lt(&end),
                ">" => abi::branch_gt(&end),
                "<=" => abi::branch_le(&end),
                ">=" => abi::branch_ge(&end),
                _ => abi::branch_eq(&end),
            };
            instructions.push(abi::compare_registers(&lhs_reg, &rhs_reg));
            instructions.push(abi::move_immediate(&dst, "Integer", "1"));
            instructions.push(branch);
            instructions.push(abi::move_immediate(&dst, "Integer", "0"));
            instructions.push(abi::label(&end));
        }
        IrLinkExpr::And(lhs, rhs) => {
            // `AND`/`OR` are *logical* connectives: any nonzero operand is true. A
            // bare `Var`/`Int` leaf is an arbitrary integer, so combining with a raw
            // bitwise `and`/`or` would compute e.g. `2 & 1 = 0` and wrongly report
            // two truthy operands as false. Normalize each operand to a canonical
            // `0`/`1` first so bitwise coincides with logical (bug-66). Comparison /
            // logical sub-expressions already yield `0`/`1`, so they pass through
            // unchanged and `AND`/`OR`-of-comparisons stays byte-identical.
            let lhs_reg = emit_link_bool(lhs, offsets, vreg, symbol, counter, instructions);
            let rhs_reg = emit_link_bool(rhs, offsets, vreg, symbol, counter, instructions);
            instructions.push(abi::and_registers(&dst, &lhs_reg, &rhs_reg));
        }
        IrLinkExpr::Or(lhs, rhs) => {
            let lhs_reg = emit_link_bool(lhs, offsets, vreg, symbol, counter, instructions);
            let rhs_reg = emit_link_bool(rhs, offsets, vreg, symbol, counter, instructions);
            instructions.push(abi::or_registers(&dst, &lhs_reg, &rhs_reg));
        }
    }
    dst
}

/// True when `expr` already produces a canonical `0`/`1` truth value, so an
/// `AND`/`OR` operand of this shape needs no normalization. Only the bare `Var`
/// and `Int` leaves carry an arbitrary integer.
fn link_expr_is_boolean(expr: &IrLinkExpr) -> bool {
    matches!(
        expr,
        IrLinkExpr::Not(_)
            | IrLinkExpr::Compare { .. }
            | IrLinkExpr::And(_, _)
            | IrLinkExpr::Or(_, _)
    )
}

/// Emit `expr` and guarantee its value is a canonical `0`/`1` truth value for use
/// as an `AND`/`OR` operand. Sub-expressions that already yield `0`/`1`
/// (comparisons and boolean connectives) pass through unchanged; a bare
/// `Var`/`Int` leaf is normalized with a compare-nonzero so any nonzero value
/// becomes `1` (bug-66).
fn emit_link_bool(
    expr: &IrLinkExpr,
    offsets: &HashMap<&str, usize>,
    vreg: &mut usize,
    symbol: &str,
    counter: &mut usize,
    instructions: &mut Vec<CodeInstruction>,
) -> String {
    let value = emit_link_expr(expr, offsets, vreg, symbol, counter, instructions);
    if link_expr_is_boolean(expr) {
        return value;
    }
    let dst = format!("%v{vreg}");
    *vreg += 1;
    let id = *counter;
    *counter += 1;
    let nonzero = format!("{symbol}_bool{id}_nz");
    let end = format!("{symbol}_bool{id}_end");
    instructions.extend([
        abi::compare_immediate(&value, "0"),
        abi::branch_ne(&nonzero),
        abi::move_immediate(&dst, "Integer", "0"),
        abi::branch(&end),
        abi::label(&nonzero),
        abi::move_immediate(&dst, "Integer", "1"),
        abi::label(&end),
    ]);
    dst
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        IrAbiSlot, IrLinkFunction, abi_ctype_valid_as_argument, abi_ctype_valid_as_return,
    };

    /// Every ctype the allow-list accepts must reach a real marshaling arm.
    ///
    /// plan-50-A closed the slot-ctype namespace and turned `emit_return_passthrough`'s
    /// default arm into an `Err`. The allow-list is hand-written (Rust cannot
    /// enumerate match arms), so this walks every accepted name through
    /// `lower_link_thunk` and asserts none of them lands on that arm. Without it,
    /// adding a ctype to the list without a marshaling arm would only fail at the
    /// first binding that used it.
    #[test]
    fn every_known_ctype_lowers() {
        // Lowering reads the active backend for register/instruction shapes; the
        // ctype set under test is backend-independent, so any backend serves.
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);

        // The full accepted set. `ir::link::tests::ctype_list_is_exhaustive` holds
        // this in sync with `abi_slot_ctype_is_known`, so a name added to the
        // authority without an arm here fails there first.
        const CTYPES: &[&str] = &[
            "CPtr", "CString", "CInt8", "CInt16", "CInt32", "CInt64", "CUInt8", "CUInt16",
            "CUInt32", "CUInt64", "CBool", "CByte", "CFloat", "CDouble", "CVoid",
        ];

        // Every ctype valid as an ABI *return* must reach a return arm. This is the
        // arm that can `Err`, and it is how the guard caught `CString` having no
        // return meaning at all (a `char *` return is `CPtr` + a `String` wrapper).
        for ctype in CTYPES.iter().filter(|c| abi_ctype_valid_as_return(c)) {
            let returns_value = *ctype != "CVoid";
            let function = IrLinkFunction {
                alias: "lib".to_string(),
                name: format!("ret_{ctype}"),
                library: "demo".to_string(),
                symbol: "demo_f".to_string(),
                params: vec![],
                // A `CPtr` return with an `Integer` wrapper takes the raw path; the
                // `String` copy-out path is covered by the sqlite3 runtime tests.
                return_type: if returns_value { "Integer" } else { "Nothing" }.to_string(),
                return_resource: false,
                abi_slots: vec![],
                abi_return_name: if returns_value { "return" } else { "status" }.to_string(),
                abi_return_ctype: (*ctype).to_string(),
                consts: vec![],
                success_on: None,
                result: None,
                free: None,
            };
            let lowered = lower_link_thunk(&function, 0, 0, None);
            assert!(
                lowered.is_ok(),
                "accepted return ctype {ctype} does not lower: {:?}",
                lowered.err()
            );
        }

        // Every ctype valid as an ABI *argument* must stage without error. Pinned
        // with CONST so the slot needs no wrapper parameter.
        for ctype in CTYPES.iter().filter(|c| abi_ctype_valid_as_argument(c)) {
            let function = IrLinkFunction {
                alias: "lib".to_string(),
                name: format!("arg_{ctype}"),
                library: "demo".to_string(),
                symbol: "demo_f".to_string(),
                params: vec![],
                return_type: "Nothing".to_string(),
                return_resource: false,
                abi_slots: vec![IrAbiSlot {
                    name: "pinned".to_string(),
                    ctype: (*ctype).to_string(),
                    direction: crate::ir::AbiDirection::In,
                }],
                abi_return_name: "status".to_string(),
                abi_return_ctype: "CInt32".to_string(),
                consts: vec![("pinned".to_string(), 0)],
                success_on: None,
                result: None,
                free: None,
            };
            let lowered = lower_link_thunk(&function, 0, 0, None);
            assert!(
                lowered.is_ok(),
                "accepted argument ctype {ctype} does not lower: {:?}",
                lowered.err()
            );
        }
    }
}
