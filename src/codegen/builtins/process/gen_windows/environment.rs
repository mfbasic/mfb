//! plan-176-T: the Windows `process` working-directory and environment-block builders
//! (`emit_win_build_cwd`, `emit_win_build_env_block`) and their frame offsets. The shared spawn
//! helpers live in `builtins/process/gen_windows/mod.rs`.

use super::*;

// --- spawnEnv's cwd + environment-block scratch, above the builder's ---
//
// plan-119-C. The four-argument `process::spawn` adds a working directory and an
// environment `Map OF String TO String` with a replace-vs-merge flag. On Unix
// those are applied *in the fork child* (`chdir` + `unsetenv`/`setenv` loops);
// `CreateProcessA` instead takes both as pointers it reads before the child
// exists, so Windows has to MATERIALIZE the result — one NUL-terminated path and
// one `name=value\0…\0\0` ANSI block — and the merge semantics are reproduced by
// *building* that block rather than by mutating anything.

/// **Caller input**: the cwd `String` object (length@0, bytes@8). An empty
/// string means "inherit", and becomes a NULL `lpCurrentDirectory`.
pub(crate) const WIN_ENV_CWDSTR: usize = WIN_CMDLINE_SCRATCH_END;

/// Its byte length.
const WIN_ENV_CWDLEN: usize = WIN_CMDLINE_SCRATCH_END + 0x08;

/// **Caller input**: the environment `Map OF String TO String`.
pub(crate) const WIN_ENV_MAP: usize = WIN_CMDLINE_SCRATCH_END + 0x10;

/// **Caller input**: the `envReplace` flag — nonzero means "only the map".
pub(crate) const WIN_ENV_REPLACE: usize = WIN_CMDLINE_SCRATCH_END + 0x18;

/// Running byte length of the block while sizing it.
const WIN_ENV_LEN: usize = WIN_CMDLINE_SCRATCH_END + 0x20;

/// Write cursor into the block.
const WIN_ENV_DP: usize = WIN_CMDLINE_SCRATCH_END + 0x28;

/// Map-entry index for the size and append walks.
const WIN_ENV_IDX: usize = WIN_CMDLINE_SCRATCH_END + 0x30;

/// The map's capacity — a Map is walked `0..capacity`, skipping unused slots.
const WIN_ENV_CAP: usize = WIN_CMDLINE_SCRATCH_END + 0x38;

/// Base of the map's string data region.
const WIN_ENV_DBASE: usize = WIN_CMDLINE_SCRATCH_END + 0x40;

/// Byte length of the key or value currently being copied.
const WIN_ENV_TMP: usize = WIN_CMDLINE_SCRATCH_END + 0x48;

/// Base of the inherited environment block (merge mode); 0 when replacing, which
/// is also what says "nothing to free".
const WIN_ENV_INHB: usize = WIN_CMDLINE_SCRATCH_END + 0x50;

/// Cursor into the inherited block.
const WIN_ENV_INHP: usize = WIN_CMDLINE_SCRATCH_END + 0x58;

/// Name length of the inherited entry at the cursor (bytes before its `=`).
const WIN_ENV_NLEN: usize = WIN_CMDLINE_SCRATCH_END + 0x60;

/// Total length of that entry (bytes before its NUL).
const WIN_ENV_ELEN: usize = WIN_CMDLINE_SCRATCH_END + 0x68;

/// Map-entry index for the override scan.
const WIN_ENV_MIDX: usize = WIN_CMDLINE_SCRATCH_END + 0x70;

/// Result of that scan: nonzero when the map overrides this inherited name.
const WIN_ENV_MATCH: usize = WIN_CMDLINE_SCRATCH_END + 0x78;

/// Map-key pointer during a name comparison.
const WIN_ENV_KP: usize = WIN_CMDLINE_SCRATCH_END + 0x80;

/// Inherited-name pointer during a name comparison.
const WIN_ENV_IP: usize = WIN_CMDLINE_SCRATCH_END + 0x88;

/// Byte counter, reused by every inner walk.
const WIN_ENV_CNT: usize = WIN_CMDLINE_SCRATCH_END + 0x90;

/// First offset above everything `spawnEnv` needs.
pub(crate) const WIN_ENV_SCRATCH_END: usize = WIN_CMDLINE_SCRATCH_END + 0x98;

/// Build `lpCurrentDirectory` from the cwd `String` in `WIN_ENV_CWDSTR` into
/// `WIN_SPAWN_CWD`, or store 0 there when the string is empty.
///
/// The Windows counterpart of the posix body's leading-NUL sentinel: Unix builds
/// a C string whose first byte is NUL so the fork child skips its `chdir`, while
/// here "inherit" is expressed directly as the NULL `CreateProcessA` accepts.
pub(crate) fn emit_win_build_cwd(
    symbol: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let sp = abi::stack_pointer();
    let inherit = format!("{symbol}_cwd_inherit");
    let copy = format!("{symbol}_cwd_copy");
    let copy_done = format!("{symbol}_cwd_copy_done");
    let done = format!("{symbol}_cwd_done");
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_CWDSTR),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_CWDLEN),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&inherit),
        abi::add_immediate(abi::return_register(), abi::mfb_arg(2), 1),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, WIN_SPAWN_CWD),
        abi::move_register(abi::mfb_arg(1), abi::mfb_return(1)),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_CWDSTR),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 8),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::label(&copy),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CWDLEN),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&copy_done),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::branch(&copy),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0),
        abi::branch(&done),
        abi::label(&inherit),
        abi::store_u64(abi::ZERO, sp, WIN_SPAWN_CWD),
        abi::label(&done),
    ]);
}

/// Build `lpEnvironment` from the map in `WIN_ENV_MAP` and the flag in
/// `WIN_ENV_REPLACE` into `WIN_SPAWN_ENV`.
///
/// The block is `name=value\0…\0\0`, ANSI (no `CREATE_UNICODE_ENVIRONMENT`), and
/// `CreateProcess` does not require it sorted. Two passes over the same walks —
/// size, then copy — so exactly one arena block is allocated.
///
/// **Replace** (`envReplace` nonzero) is a single flat walk of the map. **Merge**
/// additionally walks the inherited block from `GetEnvironmentStringsA` and keeps
/// every entry the map does not override.
///
/// The override test is **case-insensitive**, ASCII-folded. It has to be: Windows
/// environment names are case-insensitive, so a byte-exact compare against a map
/// key `PATH` would let an inherited `Path` through and hand the child *both*
/// — after which which one wins is the child's business, not the caller's. Names
/// outside ASCII are outside the documented contract and fold to themselves.
///
/// A failure to allocate branches to `alloc_fail`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_win_build_env_block(
    symbol: &str,
    alloc_fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    const HDR: usize = COLLECTION_HEADER_SIZE;
    const ENT: usize = COLLECTION_ENTRY_SIZE;
    const FLAGS: usize = COLLECTION_ENTRY_OFFSET_FLAGS;
    const KOFF: usize = COLLECTION_ENTRY_OFFSET_KEY_OFFSET;
    const KLEN: usize = COLLECTION_ENTRY_OFFSET_KEY_LENGTH;
    const VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
    const VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;
    const EQUALS: &str = "61";
    let sp = abi::stack_pointer();
    let used = COLLECTION_ENTRY_FLAG_USED.to_string();
    let ent = ENT.to_string();

    // entry = map + HDR + [index_slot]*ENT, left in mfb_arg(2). Clobbers 0..3.
    let entry_at = |index_slot: usize, out: &mut Vec<CodeInstruction>| {
        out.extend([
            abi::load_u64(abi::mfb_arg(0), sp, index_slot),
            abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_MAP),
            abi::move_immediate(abi::mfb_arg(3), "Integer", &ent),
            abi::multiply_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(3)),
            abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), HDR),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(1), abi::mfb_arg(2)),
        ]);
    };
    // Branch to `skip` unless the entry in mfb_arg(2) is USED. A Map's entry array
    // is capacity-sized and sparse, so this is not the same walk a List gets.
    let skip_unless_used = |skip: &str, out: &mut Vec<CodeInstruction>| {
        out.extend([
            abi::load_u8(abi::mfb_arg(3), abi::mfb_arg(2), FLAGS),
            abi::move_immediate(abi::mfb_arg(1), "Integer", &used),
            abi::and_registers(abi::mfb_arg(3), abi::mfb_arg(3), abi::mfb_arg(1)),
            abi::compare_immediate(abi::mfb_arg(3), "0"),
            abi::branch_eq(skip),
        ]);
    };
    // ASCII uppercase fold, in place, on one register.
    let fold = |reg: usize, site: &str, out: &mut Vec<CodeInstruction>| {
        let skip = format!("{symbol}_env_fold_{site}");
        out.extend([
            abi::compare_immediate(abi::mfb_arg(reg), "97"), // 'a'
            abi::branch_lt(&skip),
            abi::compare_immediate(abi::mfb_arg(reg), "122"), // 'z'
            abi::branch_gt(&skip),
            abi::subtract_immediate(abi::mfb_arg(reg), abi::mfb_arg(reg), 32),
            abi::label(&skip),
        ]);
    };
    // Measure the inherited entry at [WIN_ENV_INHP]: total bytes before its NUL
    // into WIN_ENV_ELEN, bytes before its first `=` into WIN_ENV_NLEN. An entry
    // with no `=` (and Windows really does have them — the `=C:` drive-cwd
    // pseudo-variables start with one) measures NLEN == ELEN, which matches no
    // map key and is therefore passed through.
    let measure = |site: &str, out: &mut Vec<CodeInstruction>| {
        let len_loop = format!("{symbol}_env_{site}_len");
        let len_done = format!("{symbol}_env_{site}_len_done");
        let nlen_loop = format!("{symbol}_env_{site}_nlen");
        let nlen_done = format!("{symbol}_env_{site}_nlen_done");
        out.extend([
            abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
            abi::label(&len_loop),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_eq(&len_done),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::branch(&len_loop),
            abi::label(&len_done),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_ELEN),
            abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
            abi::label(&nlen_loop),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
            abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
            abi::branch_eq(&nlen_done),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
            abi::compare_immediate(abi::mfb_arg(0), EQUALS),
            abi::branch_eq(&nlen_done),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::branch(&nlen_loop),
            abi::label(&nlen_done),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_NLEN),
        ]);
    };
    // WIN_ENV_MATCH = 1 when some USED map key equals the inherited name at
    // [WIN_ENV_INHP] (WIN_ENV_NLEN bytes), compared case-insensitively.
    let match_scan = |site: &str, out: &mut Vec<CodeInstruction>| {
        let loop_l = format!("{symbol}_env_{site}_m");
        let next = format!("{symbol}_env_{site}_m_next");
        let byte = format!("{symbol}_env_{site}_m_byte");
        let hit = format!("{symbol}_env_{site}_m_hit");
        let done = format!("{symbol}_env_{site}_m_done");
        out.extend([
            abi::store_u64(abi::ZERO, sp, WIN_ENV_MATCH),
            abi::store_u64(abi::ZERO, sp, WIN_ENV_MIDX),
            abi::label(&loop_l),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_MIDX),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(2)),
            abi::branch_eq(&done),
        ]);
        entry_at(WIN_ENV_MIDX, out);
        skip_unless_used(&next, out);
        out.extend([
            // a different length can never match
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), KLEN),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_NLEN),
            abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(3)),
            abi::branch_ne(&next),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), KOFF),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_DBASE),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(3), abi::mfb_arg(1)),
            abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_KP),
            abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_INHP),
            abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_IP),
            abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
            abi::label(&byte),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_NLEN),
            abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
            abi::branch_eq(&hit),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_KP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_IP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(1), abi::mfb_arg(2), 0),
        ]);
        fold(0, &format!("{site}k"), out);
        fold(1, &format!("{site}n"), out);
        out.extend([
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::branch_ne(&next),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::branch(&byte),
            abi::label(&hit),
            abi::move_immediate(abi::mfb_arg(2), "Integer", "1"),
            abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_MATCH),
            abi::branch(&done),
            abi::label(&next),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_MIDX),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_MIDX),
            abi::branch(&loop_l),
            abi::label(&done),
        ]);
    };
    // Add each USED map entry's `key=value\0` length to WIN_ENV_LEN.
    let size_map = |out: &mut Vec<CodeInstruction>| {
        let loop_l = format!("{symbol}_env_msize");
        let next = format!("{symbol}_env_msize_next");
        let done = format!("{symbol}_env_msize_done");
        out.extend([
            abi::store_u64(abi::ZERO, sp, WIN_ENV_IDX),
            abi::label(&loop_l),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(2)),
            abi::branch_eq(&done),
        ]);
        entry_at(WIN_ENV_IDX, out);
        skip_unless_used(&next, out);
        out.extend([
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), KLEN),
            abi::load_u64(abi::mfb_arg(3), abi::mfb_arg(2), VLEN),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(3)),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 2), // '=' and NUL
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
            abi::add_registers(abi::mfb_arg(3), abi::mfb_arg(3), abi::mfb_arg(1)),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
            abi::label(&next),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
            abi::branch(&loop_l),
            abi::label(&done),
        ]);
    };
    // Copy `[WIN_ENV_TMP]` bytes from the map data region at `off_field` into the
    // block at [WIN_ENV_DP], advancing the cursor. The entry must be in mfb_arg(2).
    let copy_field =
        |site: &str, len_field: usize, off_field: usize, out: &mut Vec<CodeInstruction>| {
            let loop_l = format!("{symbol}_env_{site}_cp");
            let done = format!("{symbol}_env_{site}_cp_done");
            out.extend([
                abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), len_field),
                abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_TMP),
                abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(2), off_field),
                abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DBASE),
                abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(1), abi::mfb_arg(0)),
                abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
                abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
                abi::label(&loop_l),
                abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_TMP),
                abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
                abi::branch_eq(&done),
                abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
                abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
                abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
                abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
                abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
                abi::branch(&loop_l),
                abi::label(&done),
                abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
            ]);
        };

    let skip_inh_size = format!("{symbol}_env_skip_inh_size");
    let isize_loop = format!("{symbol}_env_isize");
    let isize_next = format!("{symbol}_env_isize_next");
    let isize_done = format!("{symbol}_env_isize_done");
    let skip_inh_copy = format!("{symbol}_env_skip_inh_copy");
    let icopy_loop = format!("{symbol}_env_icopy");
    let icopy_next = format!("{symbol}_env_icopy_next");
    let icopy_done = format!("{symbol}_env_icopy_done");
    let ic_byte = format!("{symbol}_env_ic_byte");
    let ic_done = format!("{symbol}_env_ic_done");
    let acopy_loop = format!("{symbol}_env_acopy");
    let acopy_next = format!("{symbol}_env_acopy_next");
    let acopy_done = format!("{symbol}_env_acopy_done");
    let no_free = format!("{symbol}_env_no_free");

    // Map geometry, shared by every walk below.
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_MAP),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ent),
        abi::multiply_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), HDR),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_DBASE),
        // The block always ends with two NULs; an empty map's block is exactly
        // those two, which is a valid "no variables" environment.
        abi::move_immediate(abi::mfb_arg(2), "Integer", "2"),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_LEN),
        abi::store_u64(abi::ZERO, sp, WIN_ENV_INHB),
        // Merge mode takes the inherited block; replace mode never asks for it.
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_REPLACE),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_ne(&skip_inh_size),
    ]);
    platform.emit_external_call(
        "GetEnvironmentStringsA",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::store_u64(abi::c_return(0), sp, WIN_ENV_INHB),
        abi::store_u64(abi::c_return(0), sp, WIN_ENV_INHP),
        abi::label(&isize_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&isize_done),
    ]);
    measure("size", instructions);
    match_scan("size", instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_MATCH),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_ne(&isize_next),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
        abi::add_registers(abi::mfb_arg(3), abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
        abi::label(&isize_next),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::branch(&isize_loop),
        abi::label(&isize_done),
        abi::label(&skip_inh_size),
    ]);
    size_map(instructions);
    instructions.extend([
        abi::load_u64(abi::return_register(), sp, WIN_ENV_LEN),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, WIN_SPAWN_ENV),
        abi::store_u64(abi::mfb_return(1), sp, WIN_ENV_DP),
        // Second pass over the inherited block, from the same base.
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHB),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&skip_inh_copy),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
        abi::label(&icopy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&icopy_done),
    ]);
    measure("copy", instructions);
    match_scan("copy", instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_MATCH),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_ne(&icopy_next),
        // copy ELEN + 1 bytes (the entry and its NUL)
        abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
        abi::label(&ic_byte),
        abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&ic_done),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::store_u8(abi::mfb_arg(0), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
        abi::branch(&ic_byte),
        abi::label(&ic_done),
        abi::label(&icopy_next),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::branch(&icopy_loop),
        abi::label(&icopy_done),
        abi::label(&skip_inh_copy),
        // Then every USED map entry, `key=value\0`.
        abi::store_u64(abi::ZERO, sp, WIN_ENV_IDX),
        abi::label(&acopy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::branch_eq(&acopy_done),
    ]);
    entry_at(WIN_ENV_IDX, instructions);
    skip_unless_used(&acopy_next, instructions);
    copy_field("key", KLEN, KOFF, instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::move_immediate(abi::mfb_arg(2), "Integer", EQUALS),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
    ]);
    entry_at(WIN_ENV_IDX, instructions);
    copy_field("val", VLEN, VOFF, instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::label(&acopy_next),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
        abi::branch(&acopy_loop),
        abi::label(&acopy_done),
        // The block terminator.
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 1),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHB),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&no_free),
        abi::move_register(abi::mfb_arg(0), abi::mfb_arg(2)),
    ]);
    platform.emit_external_call(
        "FreeEnvironmentStringsA",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&no_free));
    Ok(())
}
