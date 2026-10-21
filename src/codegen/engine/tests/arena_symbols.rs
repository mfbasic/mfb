//! `emit_arena_alloc_call` / `emit_symbol_call` (`engine/builder/builder_emit_helpers.rs`):
//! the arena symbols are never platform imports.

/// `emit_arena_alloc_call` (bug-322) replaced 45 hand-written blocks that
/// pushed `binding: "internal", library: None` unconditionally. Routing them
/// through `emit_symbol_call` is output-identical only while no backend
/// declares an arena symbol as a platform import — if one ever did, those 45
/// sites would silently start emitting an *external* relocation against a
/// library, which is a linker-visible change no unit test would otherwise
/// catch.
///
/// The plan modules are the only source of `platform_imports` keys, so this
/// scans them as text: a grep-equivalent that cannot drift from the real
/// tables the way a hand-copied list would.
#[test]
fn arena_symbols_are_never_platform_imports() {
    use crate::codegen::error::constants::{ARENA_ALLOC_SYMBOL, ARENA_FREE_SYMBOL};
    let plans = [
        (
            "linux_aarch64",
            include_str!("../../../target/linux_aarch64/plan.rs"),
        ),
        (
            "linux_x86_64",
            include_str!("../../../target/linux_x86_64/plan.rs"),
        ),
        (
            "linux_riscv64",
            include_str!("../../../target/linux_riscv64/plan.rs"),
        ),
        (
            "macos_aarch64",
            include_str!("../../../target/macos_aarch64/plan.rs"),
        ),
    ];
    for (target, source) in plans {
        for symbol in [ARENA_ALLOC_SYMBOL, ARENA_FREE_SYMBOL] {
            assert!(
                !source.contains(symbol),
                "{target}'s plan mentions {symbol}: if it became a platform import, \
                 emit_arena_alloc_call would emit an external relocation where the \
                 hand-written blocks it replaced emitted an internal one"
            );
        }
    }
}
