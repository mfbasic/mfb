//! The registry's other arena kinds (the Canvas graphics thread's, a worker
//! not yet started, a kind the debugger does not know), the role tests that
//! fail on the way, and names a native function shares.

use super::*;

/// A registry at 0x10_0000 whose slots hold `(state, kind)` in turn.
fn registry(memory: &mut Memory, slots: &[(u64, u64)]) {
    let region = 0x10_0000;
    memory.symbols.insert(ARENA_BASE_SYMBOL, 0x600);
    memory.put(0x600, region);
    memory.put(region + REGION_COUNT_OFFSET as u64, slots.len() as u64);
    for (id, (state, kind)) in slots.iter().enumerate() {
        let slot = region + REGION_HEADER_SIZE as u64 + id as u64 * SLOT_SIZE as u64;
        memory.put(slot, *state);
        memory.put(slot + SLOT_KIND_OFFSET as u64, *kind);
        memory.put(
            state + (ARENA_ALLOC_SCALARS_OFFSET + ARENA_SC_COMMITTED) as u64,
            0,
        );
    }
}

#[test]
fn graphics_unstarted_worker_and_unknown_arenas_find_their_threads() {
    let mut memory = Memory::default();
    memory.symbols.insert(GRAPHICS_STATE_SYMBOL, 0x700);
    memory.put(0x700 + GRAPHICS_OFFSET_TID as u64, 0x66);
    // A graphics arena (its thread named by the graphics state's tid), a
    // worker arena with no control block yet (its thread found by its
    // arena register), and an arena of a kind this debugger does not know.
    registry(
        &mut memory,
        &[(0x20_0000, 2), (0x21_0000, 1), (0x22_0000, 9)],
    );
    memory.put(0x21_0000 + ARENA_WORKER_THREAD_OFFSET as u64, 0);
    let threads = [
        thread(0x11, 0, (0x1_0000, 0x2_0000)),
        thread(0x66, 0, (0x3_0000, 0x4_0000)),
        thread(0x77, 0x21_0000, (0x5_0000, 0x6_0000)),
        thread(0x88, 0x22_0000, (0x7_0000, 0x8_0000)),
    ];
    let owners = arenas(&mut memory, &threads, &[])
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|arena| (arena.id, arena.thread))
        .collect::<Vec<_>>();
    assert_eq!(owners, [(0, Some(1)), (1, Some(2)), (2, Some(3))]);
}

#[test]
fn a_freed_slot_and_a_cleared_handle_are_no_worker() {
    let mut memory = Memory::default();
    with_worker(&mut memory);
    // The live worker's control block has no thread handle yet.
    memory.put(BLOCK + CONTROL_OFFSET_OS_HANDLE as u64, 0);
    let threads = [
        thread(0x11, 0, (0x10_0000, 0x20_0000)),
        thread(0x77, 0, (0x30_0000, 0x40_0000)),
    ];
    assert_eq!(
        roles(&mut memory, &threads, false).unwrap(),
        [Role::MfbMain, Role::Other]
    );
}

#[test]
fn a_thread_with_no_known_stack_or_an_arena_that_is_not_main_s_runs_no_main() {
    let mut memory = Memory::default();
    // A worker arena (its control block set) on the thread's own stack.
    memory.put(0x30_8000 + ARENA_WORKER_THREAD_OFFSET as u64, 0x5000);
    // A state whose block head cannot be read.
    memory.put(0x50_8000 + ARENA_WORKER_THREAD_OFFSET as u64, 0);
    let mut stackless = thread(0x22, 0x30_8000, (0, 0));
    stackless.stack = None;
    let threads = [
        thread(0x11, 0, (0x10_0000, 0x20_0000)),
        stackless,
        thread(0x33, 0x30_8000, (0x30_0000, 0x40_0000)),
        thread(0x44, 0x50_8000, (0x50_0000, 0x60_0000)),
    ];
    assert_eq!(
        roles(&mut memory, &threads, true).unwrap(),
        [Role::App, Role::Other, Role::Other, Role::Other]
    );
}

#[test]
fn when_two_threads_look_like_main_the_others_keep_their_roles() {
    let mut memory = Memory::default();
    with_worker(&mut memory);
    let state = |memory: &mut Memory, at: u64| {
        memory.put(at + ARENA_WORKER_THREAD_OFFSET as u64, 0);
        memory.put(at + ARENA_SLICE_BASE_OFFSET as u64, 0x40_0000_0000);
    };
    state(&mut memory, 0x30_8000);
    state(&mut memory, 0x50_8000);
    let threads = [
        thread(0x11, 0, (0x10_0000, 0x20_0000)),
        thread(0x77, 0, (0x70_0000, 0x80_0000)),
        thread(0x22, 0x30_8000, (0x30_0000, 0x40_0000)),
        thread(0x33, 0x50_8000, (0x50_0000, 0x60_0000)),
    ];
    assert_eq!(
        roles(&mut memory, &threads, true).unwrap(),
        [Role::App, Role::User, Role::Other, Role::Other]
    );
}

#[test]
fn a_name_a_native_function_shares_is_tagged_in_a_stop_place() {
    let exit_c = format!("{}exit", crate::debugger::names::C_PREFIX);
    let function = |name: &str, linked: bool, address| FunctionSymbol {
        name: name.to_string(),
        linked,
        loaded: false,
        address: Some(address),
        size: Some(0x10),
    };
    let functions = [
        function("_mfb_fn_exit", false, 0x1000),
        function(&exit_c, true, 0x9000),
    ];
    assert_eq!(
        locate(&functions, 0x1004).as_deref(),
        Some("exit+0x4 <mfb>")
    );
}

#[test]
fn a_one_word_state_field_is_eight_bytes_once() {
    let field = word("blockHead", 0);
    assert_eq!(
        (field.name, field.offset, field.count, field.width),
        ("blockHead", 0, 1, 8)
    );
}
