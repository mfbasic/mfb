//! plan-179-B: the Linux cgroup memory limit, for the arena memory cap.
//!
//! A process in a memory-limited cgroup (a container, a systemd slice with
//! `MemoryMax`) has less memory than `sysinfo` reports, so the startup check
//! takes the smaller of the two. The cgroup the process is in is named by
//! `/proc/self/cgroup`, one line per hierarchy:
//!
//! * `0::<path>` — cgroup v2; the limit file is `/sys/fs/cgroup<path>/memory.max`.
//! * `N:<controllers>:<path>` with `memory` among the comma-separated controllers —
//!   cgroup v1; the limit file is `/sys/fs/cgroup/memory<path>/memory.limit_in_bytes`.
//!
//! Both kinds are looked for on every line (pure v2 has no `memory` line, pure v1
//! no `0::` line, and on a hybrid host the memory controller is on v1). For each
//! one found, the walk tries the limit file at `<path>`, then at each parent, up to
//! the root, and keeps the smallest limit. The parents matter twice: a limit set on
//! a parent slice applies to every cgroup under it, and a container that shares the
//! host's cgroup namespace sees its own cgroup's path in `/proc/self/cgroup` while
//! having that cgroup mounted at `/sys/fs/cgroup` — so only the final, root step
//! finds its file.
//!
//! A file that is missing, unreadable, `max`, or holds 10^18 or more (the v1 root
//! reports 9223372036854771712) is no limit. None of this is a probe failure: a
//! process outside any memory-limited cgroup simply keeps the host RAM.

use super::*;

const CGROUP_SELF: &str = "_mfb_str_arena_cap_proc_self_cgroup";
const V2_PREFIX: &str = "_mfb_str_arena_cap_cgroup_v2_prefix";
const V2_SUFFIX: &str = "_mfb_str_arena_cap_cgroup_v2_suffix";
const V1_PREFIX: &str = "_mfb_str_arena_cap_cgroup_v1_prefix";
const V1_SUFFIX: &str = "_mfb_str_arena_cap_cgroup_v1_suffix";

/// `O_RDONLY | O_CLOEXEC`. `O_CLOEXEC` is `02000000` on x86-64, aarch64 and riscv64.
const OPEN_FLAGS: &str = "524288";
/// A limit at or above this is "no limit".
const NO_LIMIT_AT: &str = "1000000000000000000";

pub(super) fn data_objects() -> Vec<CodeDataObject> {
    [
        (CGROUP_SELF, "/proc/self/cgroup"),
        (V2_PREFIX, "/sys/fs/cgroup"),
        (V2_SUFFIX, "/memory.max"),
        (V1_PREFIX, "/sys/fs/cgroup/memory"),
        (V1_SUFFIX, "/memory.limit_in_bytes"),
    ]
    .into_iter()
    .map(|(symbol, text)| string_data_object(symbol, text.to_string()))
    .collect()
}

/// Copy `len` bytes from `src` to `dst`; `src` and `len` are clobbered and `dst`
/// is left one past the last byte written.
fn emit_copy(
    emit: &mut Emit<'_>,
    name: &str,
    dst: &VirtualRegister,
    src: &VirtualRegister,
    len: &VirtualRegister,
    byte: &VirtualRegister,
) {
    let copy = emit.label(&format!("{name}_copy"));
    let done = emit.label(&format!("{name}_copy_done"));
    emit.push([
        abi::label(&copy),
        abi::compare_immediate(len, "0"),
        abi::branch_eq(&done),
        abi::load_u8(byte, src, 0),
        abi::store_u8(byte, dst, 0),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(dst, dst, 1),
        abi::subtract_immediate(len, len, 1),
        abi::branch(&copy),
        abi::label(&done),
    ]);
}

/// Lower `ram` to the smallest cgroup memory limit that applies to this process.
pub(super) fn emit_cgroup_limit(
    emit: &mut Emit<'_>,
    ram: &VirtualRegister,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let fd = vregs.next();
    let len = vregs.next();
    let n = vregs.next();
    let t = vregs.next();
    let byte = vregs.next();
    let p = vregs.next();
    let q = vregs.next();
    let end = vregs.next();
    let c1 = vregs.next();
    let c2 = vregs.next();
    let path_start = vregs.next();
    let path_end = vregs.next();
    let tok = vregs.next();
    let tok_end = vregs.next();
    let prefix = vregs.next();
    let suffix = vregs.next();
    let cur = vregs.next();
    let dst = vregs.next();
    let src = vregs.next();
    let value = vregs.next();
    let digits = vregs.next();
    let big = vregs.next();

    let done = emit.label("cgroup_done");
    let read_loop = emit.label("cgroup_read");
    let read_done = emit.label("cgroup_read_done");
    let line_loop = emit.label("cgroup_line");
    let line_body = emit.label("cgroup_line_body");
    let next_line_q = emit.label("cgroup_next_line_q");
    let next_line = emit.label("cgroup_next_line");
    let c1_loop = emit.label("cgroup_colon1");
    let c1_found = emit.label("cgroup_colon1_found");
    let c2_loop = emit.label("cgroup_colon2");
    let c2_found = emit.label("cgroup_colon2_found");
    let eol_loop = emit.label("cgroup_eol");
    let eol_found = emit.label("cgroup_eol_found");
    let tok_loop = emit.label("cgroup_token");
    let tok_end_loop = emit.label("cgroup_token_end");
    let tok_ended = emit.label("cgroup_token_ended");
    let tok_next = emit.label("cgroup_token_next");
    let is_v2 = emit.label("cgroup_v2");
    let is_v1 = emit.label("cgroup_v1");
    let walk = emit.label("cgroup_walk");
    let walk_level = emit.label("cgroup_walk_level");
    let walk_up = emit.label("cgroup_walk_up");
    let up_loop = emit.label("cgroup_walk_up_loop");
    let parse_loop = emit.label("cgroup_parse");
    let parse_digit = emit.label("cgroup_parse_digit");
    let parse_room = emit.label("cgroup_parse_room");
    let parse_end = emit.label("cgroup_parse_end");
    let parse_limited = emit.label("cgroup_parse_limited");
    let parse_lower = emit.label("cgroup_parse_lower");

    // Read all of /proc/self/cgroup (up to CGROUP_BUF_BYTES - 1 bytes).
    emit.address(CGROUP_SELF, &t);
    emit.push([
        abi::add_immediate(abi::c_arg(0), &t, 8),
        abi::move_immediate(abi::c_arg(1), "Integer", OPEN_FLAGS),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    emit.variadic_call("open")?;
    emit.push([
        abi::move_register(&fd, abi::c_return(0)),
        abi::compare_immediate(&fd, "0"),
        abi::branch_lt(&done),
        abi::move_immediate(&len, "Integer", "0"),
        abi::label(&read_loop),
        abi::move_register(abi::c_arg(0), &fd),
        abi::add_immediate(&t, abi::stack_pointer(), CGROUP_BUF),
        abi::add_registers(abi::c_arg(1), &t, &len),
        abi::move_immediate(&t, "Integer", &(CGROUP_BUF_BYTES - 1).to_string()),
        abi::subtract_registers(abi::c_arg(2), &t, &len),
    ]);
    emit.call("read")?;
    emit.push([
        abi::move_register(&n, abi::c_return(0)),
        abi::compare_immediate(&n, "0"),
        abi::branch_le(&read_done),
        abi::add_registers(&len, &len, &n),
        abi::move_immediate(&t, "Integer", &(CGROUP_BUF_BYTES - 1).to_string()),
        abi::compare_registers(&len, &t),
        abi::branch_lo(&read_loop),
        abi::label(&read_done),
        abi::move_register(abi::c_arg(0), &fd),
    ]);
    emit.call("close")?;
    // A newline after the last byte read, so every line, the last one included, ends
    // in one.
    emit.push([
        abi::add_immediate(&p, abi::stack_pointer(), CGROUP_BUF),
        abi::add_registers(&end, &p, &len),
        abi::move_immediate(&byte, "Integer", "10"),
        abi::store_u8(&byte, &end, 0),
        abi::add_immediate(&end, &end, 1),
        // One line: `id:controllers:path\n`.
        abi::label(&line_loop),
        abi::compare_registers(&p, &end),
        abi::branch_lo(&line_body),
        abi::branch(&done),
        abi::label(&line_body),
        abi::move_register(&q, &p),
        abi::label(&c1_loop),
        abi::load_u8(&byte, &q, 0),
        abi::compare_immediate(&byte, "10"),
        abi::branch_eq(&next_line_q),
        abi::compare_immediate(&byte, "58"),
        abi::branch_eq(&c1_found),
        abi::add_immediate(&q, &q, 1),
        abi::branch(&c1_loop),
        abi::label(&c1_found),
        abi::move_register(&c1, &q),
        abi::add_immediate(&q, &q, 1),
        abi::label(&c2_loop),
        abi::load_u8(&byte, &q, 0),
        abi::compare_immediate(&byte, "10"),
        abi::branch_eq(&next_line_q),
        abi::compare_immediate(&byte, "58"),
        abi::branch_eq(&c2_found),
        abi::add_immediate(&q, &q, 1),
        abi::branch(&c2_loop),
        abi::label(&c2_found),
        abi::move_register(&c2, &q),
        abi::add_immediate(&q, &q, 1),
        abi::move_register(&path_start, &q),
        abi::label(&eol_loop),
        abi::load_u8(&byte, &q, 0),
        abi::compare_immediate(&byte, "10"),
        abi::branch_eq(&eol_found),
        abi::add_immediate(&q, &q, 1),
        abi::branch(&eol_loop),
        abi::label(&eol_found),
        abi::move_register(&path_end, &q),
        // An empty controller list is the v2 line.
        abi::subtract_registers(&t, &c2, &c1),
        abi::compare_immediate(&t, "1"),
        abi::branch_eq(&is_v2),
        // Otherwise look for a `memory` token in the controller list.
        abi::add_immediate(&tok, &c1, 1),
        abi::label(&tok_loop),
        abi::move_register(&tok_end, &tok),
        abi::label(&tok_end_loop),
        abi::compare_registers(&tok_end, &c2),
        abi::branch_eq(&tok_ended),
        abi::load_u8(&byte, &tok_end, 0),
        abi::compare_immediate(&byte, "44"),
        abi::branch_eq(&tok_ended),
        abi::add_immediate(&tok_end, &tok_end, 1),
        abi::branch(&tok_end_loop),
        abi::label(&tok_ended),
        abi::subtract_registers(&t, &tok_end, &tok),
        abi::compare_immediate(&t, "6"),
        abi::branch_ne(&tok_next),
    ]);
    for (index, expected) in b"memory".iter().enumerate() {
        emit.push([
            abi::load_u8(&byte, &tok, index),
            abi::compare_immediate(&byte, &expected.to_string()),
            abi::branch_ne(&tok_next),
        ]);
    }
    emit.push([
        abi::branch(&is_v1),
        abi::label(&tok_next),
        abi::compare_registers(&tok_end, &c2),
        abi::branch_eq(&next_line),
        abi::add_immediate(&tok, &tok_end, 1),
        abi::branch(&tok_loop),
        abi::label(&is_v2),
    ]);
    emit.address(V2_PREFIX, &prefix);
    emit.address(V2_SUFFIX, &suffix);
    emit.push([abi::branch(&walk), abi::label(&is_v1)]);
    emit.address(V1_PREFIX, &prefix);
    emit.address(V1_SUFFIX, &suffix);
    emit.push([
        // The walk: `cur` is the exclusive end of the path tried at this level. The
        // path `/` is the root, tried as the empty path.
        abi::label(&walk),
        abi::move_register(&cur, &path_end),
        abi::subtract_registers(&t, &cur, &path_start),
        abi::compare_immediate(&t, "1"),
        abi::branch_ne(&walk_level),
        abi::move_register(&cur, &path_start),
        abi::label(&walk_level),
        // Skip a level whose file path does not fit the path buffer.
        abi::load_u64(&t, &prefix, 0),
        abi::load_u64(&n, &suffix, 0),
        abi::add_registers(&t, &t, &n),
        abi::add_registers(&t, &t, &cur),
        abi::subtract_registers(&t, &t, &path_start),
        abi::move_immediate(&n, "Integer", &(PATH_BUF_BYTES - 1).to_string()),
        abi::compare_registers(&t, &n),
        abi::branch_hi(&walk_up),
        abi::add_immediate(&dst, abi::stack_pointer(), PATH_BUF),
        abi::load_u64(&len, &prefix, 0),
        abi::add_immediate(&src, &prefix, 8),
    ]);
    emit_copy(emit, "cgroup_prefix", &dst, &src, &len, &byte);
    emit.push([
        abi::subtract_registers(&len, &cur, &path_start),
        abi::move_register(&src, &path_start),
    ]);
    emit_copy(emit, "cgroup_path", &dst, &src, &len, &byte);
    emit.push([
        abi::load_u64(&len, &suffix, 0),
        abi::add_immediate(&src, &suffix, 8),
    ]);
    emit_copy(emit, "cgroup_suffix", &dst, &src, &len, &byte);
    emit.push([
        abi::move_immediate(&byte, "Integer", "0"),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), PATH_BUF),
        abi::move_immediate(abi::c_arg(1), "Integer", OPEN_FLAGS),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    emit.variadic_call("open")?;
    emit.push([
        abi::move_register(&fd, abi::c_return(0)),
        abi::compare_immediate(&fd, "0"),
        abi::branch_lt(&walk_up),
        abi::move_register(abi::c_arg(0), &fd),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), LIMIT_BUF),
        abi::move_immediate(abi::c_arg(2), "Integer", &(LIMIT_BUF_BYTES - 1).to_string()),
    ]);
    emit.call("read")?;
    emit.push([
        abi::move_register(&n, abi::c_return(0)),
        abi::move_register(abi::c_arg(0), &fd),
    ]);
    emit.call("close")?;
    emit.push([
        abi::compare_immediate(&n, "0"),
        abi::branch_le(&walk_up),
        // Parse the leading decimal digits.
        abi::add_immediate(&q, abi::stack_pointer(), LIMIT_BUF),
        abi::add_registers(&n, &q, &n),
        abi::move_immediate(&value, "Integer", "0"),
        abi::move_immediate(&digits, "Integer", "0"),
        abi::move_immediate(&big, "Integer", NO_LIMIT_AT),
        abi::label(&parse_loop),
        abi::compare_registers(&q, &n),
        abi::branch_lo(&parse_digit),
        abi::branch(&parse_end),
        abi::label(&parse_digit),
        abi::load_u8(&byte, &q, 0),
        abi::subtract_immediate(&byte, &byte, 48),
        abi::compare_immediate(&byte, "9"),
        abi::branch_hi(&parse_end),
        // At or past 10^18 already: no limit (and `value × 10` cannot overflow below it).
        abi::compare_registers(&value, &big),
        abi::branch_lo(&parse_room),
        abi::branch(&walk_up),
        abi::label(&parse_room),
        abi::move_immediate(&t, "Integer", "10"),
        abi::multiply_registers(&value, &value, &t),
        abi::add_registers(&value, &value, &byte),
        abi::add_immediate(&digits, &digits, 1),
        abi::add_immediate(&q, &q, 1),
        abi::branch(&parse_loop),
        abi::label(&parse_end),
        // No digits (`max`): no limit.
        abi::compare_immediate(&digits, "0"),
        abi::branch_eq(&walk_up),
        abi::compare_registers(&value, &big),
        abi::branch_lo(&parse_limited),
        abi::branch(&walk_up),
        abi::label(&parse_limited),
        abi::compare_registers(&value, ram),
        abi::branch_lo(&parse_lower),
        abi::branch(&walk_up),
        abi::label(&parse_lower),
        abi::move_register(ram, &value),
        // Up one level: drop the last `/component`. Done after the root.
        abi::label(&walk_up),
        abi::compare_registers(&cur, &path_start),
        abi::branch_eq(&next_line),
        abi::subtract_immediate(&cur, &cur, 1),
        abi::label(&up_loop),
        abi::compare_registers(&cur, &path_start),
        abi::branch_eq(&walk_level),
        abi::load_u8(&byte, &cur, 0),
        abi::compare_immediate(&byte, "47"),
        abi::branch_eq(&walk_level),
        abi::subtract_immediate(&cur, &cur, 1),
        abi::branch(&up_loop),
        // The next line starts after this one's newline.
        abi::label(&next_line),
        abi::add_immediate(&p, &path_end, 1),
        abi::branch(&line_loop),
        abi::label(&next_line_q),
        abi::add_immediate(&p, &q, 1),
        abi::branch(&line_loop),
        abi::label(&done),
    ]);
    Ok(())
}
