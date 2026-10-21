//! plan-179-B: the arena memory cap's startup check.
//!
//! `_mfb_rt_arena_cap_init` runs once, from the program entry, after the main arena
//! state is zeroed and before the first allocation. It reads the machine's
//! physical RAM (`CodegenPlatform::emit_host_memory_probe`; on Linux also the
//! cgroup memory limit, `cgroup.rs`), computes the requested cap from the
//! `options.maxArenaMemory*` keys, applies the 90% check, and stores the cap in
//! `_mfb_rt_arena_map_limit`. It returns 0 to continue or 1 to stop the program
//! (`maxArenaMemoryCheck: "halt"`); the entry turns 1 into exit status 1.
//!
//! The keys are compile-time constants, so the helper is specialized: an unset
//! `maxArenaMemoryMiB` emits no MiB arithmetic, and only the messages the chosen
//! `maxArenaMemoryCheck` can print are emitted. A build with
//! `maxArenaMemoryDisable: true` emits none of this module.

use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::data::*;
use crate::ir::{ArenaMemoryCheck, ArenaMemoryLimits};
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;
use std::collections::HashMap;

mod cgroup;

/// The process-wide arena map limit in bytes (u64). `0` means no enforcement.
pub(crate) const ARENA_MAP_LIMIT_SYMBOL: &str = "_mfb_rt_arena_map_limit";
/// The startup check helper.
pub(crate) const ARENA_CAP_INIT_SYMBOL: &str = "_mfb_rt_arena_cap_init";
/// The bytes at `sp + 0` a `CodegenPlatform::emit_host_memory_probe` body may use
/// for the structure its OS call fills.
pub(crate) const HOST_MEMORY_PROBE_BUFFER_BYTES: usize = 128;

// The helper's stack buffer, at `sp`-relative offsets: the probe's structure at
// 0, then the small buffers, so their offsets stay short (riscv64 encodes 2047 at
// most directly), then the two large ones.
const LIMIT_BUF: usize = HOST_MEMORY_PROBE_BUFFER_BYTES;
const LIMIT_BUF_BYTES: usize = 64;
const DIGIT_BUF: usize = LIMIT_BUF + LIMIT_BUF_BYTES;
const DIGIT_BUF_BYTES: usize = 32;
const PATH_BUF: usize = DIGIT_BUF + DIGIT_BUF_BYTES;
const PATH_BUF_BYTES: usize = 1024;
const CGROUP_BUF: usize = PATH_BUF + PATH_BUF_BYTES;
/// `/proc/self/cgroup` is read whole into this many bytes: a v1 or hybrid host
/// lists 12-13 hierarchies, and the `:memory:` line is often past byte 511.
const CGROUP_BUF_BYTES: usize = 4096;
const LOCALS_BYTES: usize = CGROUP_BUF + CGROUP_BUF_BYTES;

const S_ERROR: &str = "_mfb_str_arena_cap_error";
const S_WARNING: &str = "_mfb_str_arena_cap_warning";
const S_KEY_PERCENT: &str = "_mfb_str_arena_cap_key_percent";
const S_KEY_MIB: &str = "_mfb_str_arena_cap_key_mib";
const S_CAP: &str = "_mfb_str_arena_cap_cap";
const S_ABOVE: &str = "_mfb_str_arena_cap_above";
const S_HALT_TAIL: &str = "_mfb_str_arena_cap_halt_tail";
const S_WARN_TAIL: &str = "_mfb_str_arena_cap_warn_tail";
const S_PROBE_FAILED: &str = "_mfb_str_arena_cap_probe_failed";

/// Whether `module` carries the cap: an executable without `maxArenaMemoryDisable`.
pub(crate) fn arena_cap_enabled(module: &NirModule) -> bool {
    module.entry.is_some() && !module.arena_memory.disable
}

/// The probe-failure line for this build's settings.
fn probe_failed_message(limits: &ArenaMemoryLimits, probe: &str) -> String {
    let what = format!(
        "maxArenaMemoryPercent: could not read physical memory on this platform ({probe} failed)"
    );
    match (limits.check, limits.mib) {
        (ArenaMemoryCheck::Halt, _) => format!("error: {what}; the program did not start.\n"),
        (ArenaMemoryCheck::Warn, -1) => {
            format!("warning: {what}; the arena memory cap is not enforced.\n")
        }
        (ArenaMemoryCheck::Warn, mib) => {
            format!("warning: {what}; the arena memory cap is maxArenaMemoryMiB, {mib} MiB.\n")
        }
    }
}

/// The data the helper reads and writes: the limit word and the message strings
/// this build's settings can print. Empty when the cap is disabled.
pub(crate) fn data_objects(
    module: &NirModule,
    platform: &dyn CodegenPlatform,
) -> Vec<CodeDataObject> {
    if !arena_cap_enabled(module) {
        return Vec::new();
    }
    let limits = &module.arena_memory;
    let mut objects = vec![CodeDataObject {
        symbol: ARENA_MAP_LIMIT_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.arena_map_limit.v1 { u64 limitBytes }".to_string(),
        align: 8,
        size: 8,
        value: "0000000000000000".to_string(),
    }];
    let (prefix, tail) = match limits.check {
        ArenaMemoryCheck::Halt => (
            (S_ERROR, "error: "),
            (S_HALT_TAIL, " bytes); the program did not start.\n"),
        ),
        ArenaMemoryCheck::Warn => ((S_WARNING, "warning: "), (S_WARN_TAIL, " bytes).\n")),
    };
    let mut strings = vec![
        prefix,
        (S_KEY_PERCENT, "maxArenaMemoryPercent"),
        (S_CAP, ": the arena memory cap of "),
        (S_ABOVE, " bytes is above 90% of physical memory ("),
        tail,
    ];
    if limits.mib != -1 {
        strings.push((S_KEY_MIB, "maxArenaMemoryMiB"));
    }
    objects.extend(
        strings
            .into_iter()
            .map(|(symbol, text)| string_data_object(symbol, text.to_string())),
    );
    objects.push(string_data_object(
        S_PROBE_FAILED,
        probe_failed_message(limits, platform.host_memory_probe_name()),
    ));
    if platform.family() == PlatformFamily::Linux {
        objects.extend(cgroup::data_objects());
    }
    objects
}

/// The helper's emission state: the symbol its labels and relocations name, and
/// the instruction stream.
struct Emit<'a> {
    symbol: &'static str,
    platform: &'a dyn CodegenPlatform,
    platform_imports: &'a HashMap<String, String>,
    instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
}

impl Emit<'_> {
    fn label(&self, name: &str) -> String {
        format!("{}_{name}", self.symbol)
    }

    fn push(&mut self, instructions: impl IntoIterator<Item = CodeInstruction>) {
        self.instructions.extend(instructions);
    }

    fn call(&mut self, base: &str) -> Result<(), String> {
        self.platform.emit_external_call(
            base,
            self.symbol,
            self.platform_imports,
            &mut self.instructions,
            &mut self.relocations,
        )
    }

    fn variadic_call(&mut self, base: &str) -> Result<(), String> {
        self.platform.emit_variadic_external_call(
            base,
            self.symbol,
            self.platform_imports,
            &mut self.instructions,
            &mut self.relocations,
        )
    }

    fn address(&mut self, symbol: &str, dst: &VirtualRegister) {
        push_symbol_address(
            self.symbol,
            symbol,
            dst,
            &mut self.instructions,
            &mut self.relocations,
        );
    }

    /// Write the string object `symbol` to stderr.
    fn write_string(&mut self, symbol: &str, scratch: &VirtualRegister) -> Result<(), String> {
        self.address(symbol, scratch);
        self.push([
            abi::load_u64(abi::string_length_register(), scratch, 0),
            abi::add_immediate(abi::string_data_register(), scratch, 8),
            abi::move_immediate(abi::return_register(), "Integer", "2"),
        ]);
        self.platform.emit_write(
            self.symbol,
            self.platform_imports,
            &mut self.instructions,
            &mut self.relocations,
        )
    }

    /// Write the unsigned decimal of `value` to stderr. `value` is clobbered.
    fn write_decimal(
        &mut self,
        name: &str,
        value: &VirtualRegister,
        vregs: &mut Vregs,
    ) -> Result<(), String> {
        let cursor = vregs.next();
        let ten = vregs.next();
        let quotient = vregs.next();
        let digit = vregs.next();
        let end = vregs.next();
        let digit_loop = self.label(&format!("{name}_digit"));
        self.push([
            abi::add_immediate(&cursor, abi::stack_pointer(), DIGIT_BUF + DIGIT_BUF_BYTES),
            abi::move_immediate(&ten, "Integer", "10"),
            abi::label(&digit_loop),
            abi::unsigned_divide_registers(&quotient, value, &ten),
            abi::multiply_subtract_registers(&digit, &quotient, &ten, value),
            abi::add_immediate(&digit, &digit, 48),
            abi::subtract_immediate(&cursor, &cursor, 1),
            abi::store_u8(&digit, &cursor, 0),
            abi::move_register(value, &quotient),
            abi::compare_immediate(value, "0"),
            abi::branch_ne(&digit_loop),
            abi::add_immediate(&end, abi::stack_pointer(), DIGIT_BUF + DIGIT_BUF_BYTES),
            // Cursor first: on Win64 the length and data registers may share a
            // physical register with the cursor's (see the entry's integer writer).
            abi::move_register(abi::string_data_register(), &cursor),
            abi::subtract_registers(abi::string_length_register(), &end, &cursor),
            abi::move_immediate(abi::return_register(), "Integer", "2"),
        ]);
        self.platform.emit_write(
            self.symbol,
            self.platform_imports,
            &mut self.instructions,
            &mut self.relocations,
        )
    }
}

/// `value = value × numerator / 100`, rounded down, without the overflow of
/// multiplying first: `(value / 100) × n + (value % 100) × n / 100`.
fn emit_percent_of(
    emit: &mut Emit<'_>,
    dst: &VirtualRegister,
    value: &VirtualRegister,
    numerator: u64,
    vregs: &mut Vregs,
) {
    let hundred = vregs.next();
    let quotient = vregs.next();
    let remainder = vregs.next();
    let factor = vregs.next();
    emit.push([
        abi::move_immediate(&hundred, "Integer", "100"),
        abi::move_immediate(&factor, "Integer", &numerator.to_string()),
        abi::unsigned_divide_registers(&quotient, value, &hundred),
        abi::multiply_subtract_registers(&remainder, &quotient, &hundred, value),
        abi::multiply_registers(&quotient, &quotient, &factor),
        abi::multiply_registers(&remainder, &remainder, &factor),
        abi::unsigned_divide_registers(&remainder, &remainder, &hundred),
        abi::add_registers(dst, &quotient, &remainder),
    ]);
}

/// `_mfb_rt_arena_cap_init`: probe, check, store the limit. Returns 0 to run the
/// program, 1 to stop it with exit status 1.
pub(crate) fn lower_arena_cap_init(
    limits: &ArenaMemoryLimits,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let mut emit = Emit {
        symbol: ARENA_CAP_INIT_SYMBOL,
        platform,
        platform_imports,
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
    };
    let mut vregs = Vregs::new();
    let ram = vregs.next();
    let scratch = vregs.next();
    let requested = vregs.next();
    let threshold = vregs.next();
    let from_mib = vregs.next();
    let probe_failed = emit.label("probe_failed");
    let store = emit.label("store");

    platform.emit_host_memory_probe(
        ARENA_CAP_INIT_SYMBOL,
        platform_imports,
        (&ram).into(),
        (&scratch).into(),
        &mut emit.instructions,
        &mut emit.relocations,
    )?;
    emit.push([
        abi::compare_immediate(&ram, "0"),
        abi::branch_eq(&probe_failed),
    ]);
    if platform.family() == PlatformFamily::Linux {
        cgroup::emit_cgroup_limit(&mut emit, &ram, &mut vregs)?;
    }

    // requested = the percent limit, or the MiB limit when that is set and smaller.
    emit_percent_of(&mut emit, &requested, &ram, limits.percent, &mut vregs);
    emit.push([abi::move_immediate(&from_mib, "Integer", "0")]);
    if limits.mib != -1 {
        let keep_percent = emit.label("keep_percent");
        let mib_bytes = (limits.mib as u64) << 20;
        emit.push([
            abi::move_immediate(&scratch, "Integer", &mib_bytes.to_string()),
            // Keep the percent limit when it is not above the MiB limit.
            abi::compare_registers(&requested, &scratch),
            abi::branch_ls(&keep_percent),
            abi::move_register(&requested, &scratch),
            abi::move_immediate(&from_mib, "Integer", "1"),
            abi::label(&keep_percent),
        ]);
    }
    // A cgroup limit under 100 bytes gives a percent limit of 0, which would read
    // as "no enforcement"; the smallest real cap is one byte.
    let nonzero = emit.label("nonzero");
    emit.push([
        abi::compare_immediate(&requested, "0"),
        abi::branch_ne(&nonzero),
        abi::move_immediate(&requested, "Integer", "1"),
        abi::label(&nonzero),
    ]);
    emit_percent_of(&mut emit, &threshold, &ram, 90, &mut vregs);
    emit.push([
        abi::compare_registers(&requested, &threshold),
        abi::branch_ls(&store),
    ]);
    // Over the threshold: one line naming the key that produced the cap.
    emit.write_string(
        match limits.check {
            ArenaMemoryCheck::Halt => S_ERROR,
            ArenaMemoryCheck::Warn => S_WARNING,
        },
        &scratch,
    )?;
    if limits.mib != -1 {
        let key_percent = emit.label("key_percent");
        let key_done = emit.label("key_done");
        emit.push([
            abi::compare_immediate(&from_mib, "0"),
            abi::branch_eq(&key_percent),
        ]);
        emit.write_string(S_KEY_MIB, &scratch)?;
        emit.push([abi::branch(&key_done), abi::label(&key_percent)]);
        emit.write_string(S_KEY_PERCENT, &scratch)?;
        emit.push([abi::label(&key_done)]);
    } else {
        emit.write_string(S_KEY_PERCENT, &scratch)?;
    }
    emit.write_string(S_CAP, &scratch)?;
    let shown = vregs.next();
    emit.push([abi::move_register(&shown, &requested)]);
    emit.write_decimal("requested", &shown, &mut vregs)?;
    emit.write_string(S_ABOVE, &scratch)?;
    emit.push([abi::move_register(&shown, &threshold)]);
    emit.write_decimal("threshold", &shown, &mut vregs)?;
    match limits.check {
        ArenaMemoryCheck::Halt => {
            emit.write_string(S_HALT_TAIL, &scratch)?;
            emit.push([
                abi::move_immediate(abi::return_register(), "Integer", "1"),
                abi::return_(),
            ]);
        }
        ArenaMemoryCheck::Warn => {
            emit.write_string(S_WARN_TAIL, &scratch)?;
            emit.push([abi::branch(&store)]);
        }
    }

    // The probe failed: `halt` stops; `warn` caps at the MiB limit, or not at all.
    emit.push([abi::label(&probe_failed)]);
    emit.write_string(S_PROBE_FAILED, &scratch)?;
    match limits.check {
        ArenaMemoryCheck::Halt => emit.push([
            abi::move_immediate(abi::return_register(), "Integer", "1"),
            abi::return_(),
        ]),
        ArenaMemoryCheck::Warn => {
            let mib_bytes = if limits.mib == -1 {
                0
            } else {
                (limits.mib as u64) << 20
            };
            emit.push([
                abi::move_immediate(&requested, "Integer", &mib_bytes.to_string()),
                abi::branch(&store),
            ]);
        }
    }

    emit.push([abi::label(&store)]);
    emit.address(ARENA_MAP_LIMIT_SYMBOL, &scratch);
    emit.push([
        abi::store_u64(&requested, &scratch, 0),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
    ]);

    let Emit {
        mut instructions,
        relocations,
        ..
    } = emit;
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCALS_BYTES);
    Ok(CodeFunction {
        name: "runtime.arena_cap_init".to_string(),
        symbol: ARENA_CAP_INIT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
}
