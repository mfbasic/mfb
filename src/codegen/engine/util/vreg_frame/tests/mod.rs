use super::*;

/// The scratch a body needs is the high-water mark of its `[sp + N]`
/// accesses, and a load/store consumes 8 bytes at its offset — so a body
/// that stores at `sp+0x70` (what Win64's app-mode `term::on` does for
/// hdcScreen) needs 0x78 bytes, not 0x70 and emphatically not 0.
#[test]
fn required_sp_local_bytes_is_the_access_high_water_mark() {
    let instructions = vec![
        abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x10),
        abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x70),
    ];
    assert_eq!(required_sp_local_bytes(&instructions), 0x78);
}

/// A narrower or wider access needs exactly its own width (bug-774): a byte
/// read of a 16-byte scratch buffer's last byte at `sp+0x4f` fits an 0x50-byte
/// frame, which an 8-byte assumption rejected the moment a smaller save area
/// moved the buffer to the frame's end; a 16-byte `str_q` at `sp+0x10` needs
/// 0x20, which an 8-byte assumption under-reserved.
#[test]
fn required_sp_local_bytes_uses_each_access_width() {
    let needs = |instruction| required_sp_local_bytes(&[instruction]);
    assert_eq!(
        needs(abi::load_u8(abi::mfb_arg(0), abi::stack_pointer(), 0x4f)),
        0x50
    );
    assert_eq!(
        needs(abi::store_u16(abi::mfb_arg(0), abi::stack_pointer(), 0x4e)),
        0x50
    );
    assert_eq!(
        needs(abi::load_u32(abi::mfb_arg(0), abi::stack_pointer(), 0x4c)),
        0x50
    );
    assert_eq!(
        needs(abi::vector_store(
            abi::VEC_SCRATCH[0],
            abi::stack_pointer(),
            0x10
        )),
        0x20
    );
}

/// An address computation may legally name the frame's end as a limit, so it
/// needs `offset` bytes rather than `offset + 8`.
#[test]
fn required_sp_local_bytes_treats_add_immediate_as_a_limit() {
    let instructions = vec![abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        0x40,
    )];
    assert_eq!(required_sp_local_bytes(&instructions), 0x40);
}

/// A body with no sp-relative access reserves nothing — this is what keeps
/// the macOS/Linux app-mode term bodies byte-identical.
#[test]
fn required_sp_local_bytes_is_zero_without_stack_access() {
    let instructions = vec![abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0x80)];
    assert_eq!(required_sp_local_bytes(&instructions), 0);
}

/// A platform hook that brackets its own `sub_sp`/`add_sp` region addresses
/// that region, not this frame, so those accesses are not the caller's to
/// reserve — the same depth rule the drift assertion applies.
#[test]
fn required_sp_local_bytes_skips_a_hooks_own_sub_sp_region() {
    let instructions = vec![
        abi::subtract_stack(0x30),
        abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x20),
        abi::add_stack(0x30),
        abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x08),
    ];
    assert_eq!(required_sp_local_bytes(&instructions), 0x10);
}
