//! bug-996: how a built Vulkan device and pipeline reach the graphics thread.
//!
//! **The pipeline is never on a frame's critical path.** `vkCreateGraphicsPipelines`
//! for the canvas fragment shader takes tens of seconds on a real driver that has not
//! seen the executable before (38 s on Intel UHD 630, box 206), so the build
//! (`canvas::vulkanBuild`, [`super::vulkan::emit_vulkan_build`]) runs on a thread of
//! its own and frames are drawn in software until it has finished.
//!
//! # The hand-off is one word
//!
//! [`GRAPHICS_OFFSET_VULKAN_READY`] holds the build's state:
//!
//! | value | meaning | written by |
//! |---|---|---|
//! | [`VULKAN_UNTRIED`] | nobody has asked for a GPU frame yet | the linker (zero) |
//! | [`VULKAN_BUILDING`] | the build has been claimed and is running | the graphics thread, once |
//! | [`VULKAN_BUILT`] | every object the builder makes exists | the building thread, as its last store |
//! | [`VULKAN_FAILED`] | no Vulkan here; software forever | the building thread, or the graphics thread when no thread could be started |
//!
//! The order is the proof:
//!
//! * **Only the graphics thread moves the word off `UNTRIED`,** and it does so before
//!   the builder thread exists ([`emit_claim`], then the spawn). So there is one
//!   builder at most, and no compare-and-swap is needed to say so.
//! * **The builder writes the word last.** Every object word it fills (the library
//!   handle, instance, physical device, device, queue and family, the WSI flag, the
//!   descriptor layout, pool, set and the two mapped buffers, the pipeline layout,
//!   the render pass and the four pipelines) is a plain store made before
//!   [`emit_publish_ready`]. On AArch64 that publish is a store-release, visible only
//!   after every earlier store of the thread; x86-64 keeps stores in order by itself.
//!   After it the builder touches nothing and returns.
//! * **The graphics thread reads the word first** ([`emit_load_ready`], a load-acquire
//!   on AArch64) and reads an object word only after it has read `BUILT`. It never
//!   writes one of the builder's words. What it does write is its own, and it creates
//!   them only on a GPU frame, which is to say only after `BUILT`: the render target
//!   and its readback buffer, the command pool and buffer, and the surface, swapchain,
//!   semaphores and present pass of a direct present.
//!
//! So the graphics thread never sees a half-built pipeline, and the builder never
//! touches what the graphics thread owns. Vulkan objects are not tied to the thread
//! that created them; what the API asks for is that two threads do not use one object
//! at once, and the word is what keeps them apart.
//!
//! # No arena on the builder
//!
//! Arena state is per-thread (`.ai/canvas-threading.md` §2), and a helper thread that
//! allocated from an arena and handed the block over would be the hazard that section
//! describes. The builder has no arena at all: [`emit_vulkan_builder_entry`] pins
//! nothing into the arena-state register, the build uses its own stack and the
//! process-global graphics state, and what it hands over is that state's words.
//!
//! # Nobody waits for it, and nobody stops it
//!
//! The thread is handed back to the system the moment it is created (`pthread_detach`,
//! `CloseHandle`). Shutdown joins the graphics thread, which is not inside the build,
//! and then the process ends with the builder wherever it is: it owns no memory the
//! program frees (the graphics state is static data and the rest is the driver's), and
//! no Vulkan object is destroyed at exit, built or not.
//!
//! `MFB_CANVAS_GPU` (asking for the GPU by name) runs the same build on the graphics
//! thread instead, to its end, before the first frame is drawn: [`BUILD_ON_THIS_THREAD`].
//! That keeps a test's first frame a GPU frame. It is also the one case where the
//! graphics thread is inside the build, so there a first frame, and an exit during it,
//! wait for the pipeline as they always did.

use super::*;

/// Nothing has asked for the pipeline.
pub(crate) const VULKAN_UNTRIED: &str = "0";
/// The pipeline exists; the graphics thread may draw with it.
pub(crate) const VULKAN_BUILT: &str = "1";
/// It could not be built. Frames are software for the life of the process.
pub(crate) const VULKAN_FAILED: &str = "2";
/// The build is claimed and under way.
pub(crate) const VULKAN_BUILDING: &str = "3";

/// `canvas::vulkanBuild`'s argument: start the build on a thread of its own, unless it
/// has been started already.
///
/// Test-only as a name: the member compares against the two values below and treats
/// every other as this one, so nothing emitted spells it. The MFBASIC render loop
/// does, as a literal, and `the_render_loop_starts_the_vulkan_build_where_the_handoff_says`
/// holds that literal to this.
#[cfg(test)]
pub(crate) const BUILD_ON_ITS_OWN_THREAD: &str = "0";
/// Run the build on the calling thread, to its end, unless it has been started already.
pub(crate) const BUILD_ON_THIS_THREAD: &str = "1";
/// Run the build: the claim was made by the thread that started this one. Only
/// [`emit_vulkan_builder_entry`] passes it.
pub(crate) const BUILD_CLAIMED: &str = "2";

/// The builder thread's start routine.
pub(crate) const VULKAN_BUILDER_SYMBOL: &str = "_mfb_rt_canvas_vulkan_builder";
/// `canvas::vulkanBuild`'s runtime symbol, which the start routine calls.
pub(crate) const VULKAN_BUILD_SYMBOL: &str = "_mfb_rt_canvas_canvas_vulkanBuild";

/// Whether the target needs the ready word's accesses ordered explicitly: AArch64,
/// where a plain store may become visible ahead of an earlier one.
pub(crate) fn orders_ready_word(platform: &dyn CodegenPlatform) -> bool {
    platform.arch() == "aarch64"
}

/// `dst = state[READY]`, read before any load that follows it.
///
/// `dst` doubles as the address register, so the caller keeps no temporary live.
pub(crate) fn emit_load_ready(
    ordered: bool,
    state: abi::Reg,
    dst: abi::Reg,
    instructions: &mut Vec<CodeInstruction>,
) {
    if ordered {
        instructions.push(abi::add_immediate(dst, state, GRAPHICS_OFFSET_VULKAN_READY));
        instructions.push(abi::load_acquire_u64(dst, dst));
    } else {
        instructions.push(abi::load_u64(dst, state, GRAPHICS_OFFSET_VULKAN_READY));
    }
}

/// `state[READY] = value`, visible only after every store this thread made before it.
pub(crate) fn emit_publish_ready(
    ordered: bool,
    state: abi::Reg,
    addr: abi::Reg,
    value: abi::Reg,
    instructions: &mut Vec<CodeInstruction>,
) {
    if ordered {
        instructions.push(abi::add_immediate(
            addr,
            state,
            GRAPHICS_OFFSET_VULKAN_READY,
        ));
        instructions.push(abi::store_release_u64(value, addr));
    } else {
        instructions.push(abi::store_u64(value, state, GRAPHICS_OFFSET_VULKAN_READY));
    }
}

/// Claim the build: branch to `taken` unless the word is [`VULKAN_UNTRIED`], and
/// otherwise make it [`VULKAN_BUILDING`].
///
/// A load and a store, not a compare-and-swap: the graphics thread is the only caller
/// (the render loop, through `canvas::vulkanBuild`), and the builder thread does not
/// exist until after the store.
pub(crate) fn emit_claim(
    ordered: bool,
    state: abi::Reg,
    scratch: abi::Reg,
    addr: abi::Reg,
    taken: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    emit_load_ready(ordered, state, scratch, instructions);
    instructions.push(abi::compare_immediate(scratch, VULKAN_UNTRIED));
    instructions.push(abi::branch_ne(taken));
    instructions.push(abi::move_immediate(scratch, "Integer", VULKAN_BUILDING));
    emit_publish_ready(ordered, state, addr, scratch, instructions);
}

/// `canvas::vulkanReady() AS Boolean` — is the pipeline built, now?
///
/// One load and a compare. It starts nothing and waits for nothing, which is what lets
/// the render loop ask it every frame and a `--debug` build's stats line ask it too.
pub(crate) fn emit_vulkan_is_ready(
    symbol: &str,
    ordered: bool,
    state: abi::Reg,
    value: abi::Reg,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let built = format!("{symbol}_vk_built");
    let done = format!("{symbol}_vk_answered");
    state_base(symbol, state, instructions, relocations);
    emit_load_ready(ordered, state, value, instructions);
    instructions.push(abi::compare_immediate(value, VULKAN_BUILT));
    instructions.push(abi::branch_eq(&built));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&built));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    instructions.push(abi::label(&done));
}

/// The registers the builder's entry hands back to the thread library: the C ABI's whole
/// callee-saved integer bank.
///
/// Not the graphics entry's list. That entry's body is allocator-compiled MFBASIC,
/// which preserves the allocator's callee-saved registers frame by frame, so it saves
/// only what MFB code pins or stages outside the allocator. This entry calls a
/// hand-written member directly, and hand-written code stages values in the scratch
/// pool, which on x86-64 realizes to callee-saved registers no frame saves (the build
/// writes `r12`). Saving the whole bank is the rule that needs no audit of the body.
/// The build uses no floating-point register.
pub(crate) fn builder_entry_saved(arch: &str, windows: bool) -> Vec<abi::Reg> {
    use crate::arch::aarch64::regmodel::aapcs64_callee_saved_int_registers;
    use crate::arch::x86_64::regmodel::{
        sysv_nonvolatile_int_registers, win64_nonvolatile_int_registers,
    };
    match (arch, windows) {
        ("x86_64", true) => win64_nonvolatile_int_registers(),
        ("x86_64", false) => sysv_nonvolatile_int_registers(),
        _ => {
            // `x19`–`x28`: the AAPCS64 callee-saved bank without its last entry, the
            // frame pointer `x29`.
            let mut bank = aapcs64_callee_saved_int_registers();
            bank.pop();
            bank
        }
    }
}

/// `_mfb_rt_canvas_vulkan_builder` — the builder thread's whole life: run the build
/// this thread was started for, and return.
///
/// It takes the frame every canvas thread entry takes ([`emit_thread_entry`]) because
/// the thread library is its caller, and saves [`builder_entry_saved`]. It pins no
/// arena state: the build has none to use (see the module's "No arena on the
/// builder").
pub(crate) fn emit_vulkan_builder_entry(platform: &dyn CodegenPlatform) -> CodeFunction {
    let from = VULKAN_BUILDER_SYMBOL;
    let windows = platform.family() == PlatformFamily::Windows;
    emit_thread_entry(
        from,
        "canvas.vulkan_builder",
        platform,
        builder_entry_saved(platform.arch(), windows),
        |instructions, relocations| {
            instructions.push(abi::move_immediate(
                abi::mfb_arg(0),
                "Integer",
                BUILD_CLAIMED,
            ));
            instructions.push(abi::branch_link(VULKAN_BUILD_SYMBOL));
            relocations.push(internal_branch(from, VULKAN_BUILD_SYMBOL));
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;

    fn name(reg: abi::Reg) -> String {
        abi::move_register(reg, reg)
            .get("dst")
            .expect("a register renders")
    }

    /// Run a seam's instructions over a graphics state holding `ready`, and answer the
    /// ready word afterwards with every register the run left.
    ///
    /// The seams are loads, stores, compares and branches on the registers they were
    /// handed, so they can be executed rather than read. The state's address is 0; a
    /// store anywhere but the ready word, or an instruction this does not know, fails
    /// the test.
    fn run(instructions: &[CodeInstruction], ready: u64) -> (u64, HashMap<String, u64>) {
        let at_ready = GRAPHICS_OFFSET_VULKAN_READY as u64;
        let mut word = ready;
        let mut registers: HashMap<String, u64> = HashMap::new();
        let field = |at: usize, key: &str| -> String {
            instructions[at]
                .get(key)
                .unwrap_or_else(|| panic!("instruction {at} has no `{key}`"))
        };
        let label = |target: &str| -> usize {
            instructions
                .iter()
                .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(target))
                .unwrap_or_else(|| panic!("no label `{target}`"))
        };
        let read = |registers: &HashMap<String, u64>, key: String| -> u64 {
            registers.get(&key).copied().unwrap_or(0)
        };
        let mut flags = (0u64, 0u64);
        let mut at = 0;
        while at < instructions.len() {
            let mut next = at + 1;
            match instructions[at].op {
                CodeOp::Label => {}
                // The state's address: 0 here, so an offset is an address.
                CodeOp::Adrp => {
                    registers.insert(field(at, "dst"), 0);
                }
                CodeOp::AddPageOff => {
                    let value = read(&registers, field(at, "src"));
                    registers.insert(field(at, "dst"), value);
                }
                CodeOp::AddImm => {
                    let sum = read(&registers, field(at, "src"))
                        + field(at, "imm").parse::<u64>().expect("an immediate");
                    registers.insert(field(at, "dst"), sum);
                }
                CodeOp::LdrU64 => {
                    let address = read(&registers, field(at, "base"))
                        + field(at, "offset").parse::<u64>().expect("an offset");
                    assert_eq!(address, at_ready, "a load that is not the ready word");
                    registers.insert(field(at, "dst"), word);
                }
                CodeOp::LdarU64 => {
                    let address = read(&registers, field(at, "base"));
                    assert_eq!(address, at_ready, "a load that is not the ready word");
                    registers.insert(field(at, "dst"), word);
                }
                CodeOp::StrU64 => {
                    let address = read(&registers, field(at, "base"))
                        + field(at, "offset").parse::<u64>().expect("an offset");
                    assert_eq!(address, at_ready, "a store that is not the ready word");
                    word = read(&registers, field(at, "src"));
                }
                CodeOp::StlrU64 => {
                    let address = read(&registers, field(at, "base"));
                    assert_eq!(address, at_ready, "a store that is not the ready word");
                    word = read(&registers, field(at, "src"));
                }
                CodeOp::CmpImm => {
                    flags = (
                        read(&registers, field(at, "lhs")),
                        field(at, "rhs").parse().expect("an immediate"),
                    )
                }
                CodeOp::MovImm => {
                    registers.insert(
                        field(at, "dst"),
                        field(at, "value").parse().expect("an immediate"),
                    );
                }
                CodeOp::BranchEq => {
                    if flags.0 == flags.1 {
                        next = label(&field(at, "target"));
                    }
                }
                CodeOp::BranchNe => {
                    if flags.0 != flags.1 {
                        next = label(&field(at, "target"));
                    }
                }
                CodeOp::Branch => next = label(&field(at, "target")),
                other => panic!("the seam emitted {other:?}, which this does not run"),
            }
            at = next;
        }
        (word, registers)
    }

    fn state_value(text: &str) -> u64 {
        text.parse().expect("a state value")
    }

    /// The question answers TRUE for `BUILT` alone and writes nothing, on both orderings.
    #[test]
    fn the_question_is_true_only_once_built_and_changes_nothing() {
        for ordered in [false, true] {
            for (state, expected) in [
                (VULKAN_UNTRIED, 0),
                (VULKAN_BUILDING, 0),
                (VULKAN_FAILED, 0),
                (VULKAN_BUILT, 1),
            ] {
                let mut instructions = Vec::new();
                let mut relocations = Vec::new();
                emit_vulkan_is_ready(
                    "vk_ready_test",
                    ordered,
                    abi::LOCAL[0],
                    abi::LOCAL[1],
                    &mut instructions,
                    &mut relocations,
                );
                assert!(
                    !instructions
                        .iter()
                        .any(|i| matches!(i.op, CodeOp::BranchLink | CodeOp::BranchLinkRegister)),
                    "the question makes a call"
                );
                let (word, registers) = run(&instructions, state_value(state));
                assert_eq!(word, state_value(state), "ordered={ordered}");
                assert_eq!(
                    registers[&name(RESULT_VALUE_REGISTER)],
                    expected,
                    "ordered={ordered}, state {state}"
                );
            }
        }
    }

    /// The claim takes `UNTRIED` to `BUILDING` and leaves every other state alone, so
    /// a build that is running, done or failed is never started again.
    #[test]
    fn only_an_untried_build_can_be_claimed() {
        for ordered in [false, true] {
            for state in [VULKAN_UNTRIED, VULKAN_BUILDING, VULKAN_BUILT, VULKAN_FAILED] {
                let mut instructions = Vec::new();
                emit_claim(
                    ordered,
                    abi::LOCAL[0],
                    abi::LOCAL[1],
                    abi::LOCAL[2],
                    "taken",
                    &mut instructions,
                );
                // What the caller does with each outcome.
                instructions.push(abi::move_immediate(abi::LOCAL[3], "Integer", "1"));
                instructions.push(abi::branch("end"));
                instructions.push(abi::label("taken"));
                instructions.push(abi::move_immediate(abi::LOCAL[3], "Integer", "0"));
                instructions.push(abi::label("end"));
                let (word, registers) = run(&instructions, state_value(state));
                let claimed = state == VULKAN_UNTRIED;
                assert_eq!(registers[&name(abi::LOCAL[3])], u64::from(claimed));
                let after = if claimed { VULKAN_BUILDING } else { state };
                assert_eq!(word, state_value(after), "ordered={ordered}, state {state}");
            }
        }
    }

    /// Where order has to be asked for, the publish is a store-release and the read a
    /// load-acquire; elsewhere each is the plain instruction, and never a mix.
    #[test]
    fn the_ordered_target_releases_and_acquires() {
        for ordered in [false, true] {
            let mut publish = Vec::new();
            emit_publish_ready(
                ordered,
                abi::LOCAL[0],
                abi::LOCAL[1],
                abi::LOCAL[2],
                &mut publish,
            );
            let mut load = Vec::new();
            emit_load_ready(ordered, abi::LOCAL[0], abi::LOCAL[1], &mut load);
            let has = |list: &[CodeInstruction], op: CodeOp| list.iter().any(|i| i.op == op);
            assert_eq!(has(&publish, CodeOp::StlrU64), ordered);
            assert_eq!(has(&publish, CodeOp::StrU64), !ordered);
            assert_eq!(has(&load, CodeOp::LdarU64), ordered);
            assert_eq!(has(&load, CodeOp::LdrU64), !ordered);
        }
    }

    /// The builder's entry saves the ABI's whole callee-saved integer bank on every
    /// target, and its frame keeps the alignment a thread entry needs with that many
    /// saves (odd by 8 on x86-64, where the entry is reached by a `call`).
    #[test]
    fn the_builder_entry_saves_the_whole_callee_saved_bank() {
        let spelled =
            |saved: &[abi::Reg]| -> Vec<String> { saved.iter().map(|r| name(*r)).collect() };
        assert_eq!(
            spelled(&builder_entry_saved("x86_64", false)),
            ["rbx", "rbp", "r12", "r13", "r14", "r15"]
        );
        assert_eq!(
            spelled(&builder_entry_saved("x86_64", true)),
            ["rbx", "rbp", "rdi", "rsi", "r12", "r13", "r14", "r15"]
        );
        assert_eq!(
            spelled(&builder_entry_saved("aarch64", false)),
            ["x19", "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28"]
        );
        for (arch, windows) in [("x86_64", false), ("x86_64", true), ("aarch64", false)] {
            let saved = builder_entry_saved(arch, windows).len();
            let frame = thread_entry_frame(arch, windows, saved);
            let odd = if arch == "x86_64" { 8 } else { 0 };
            assert_eq!(frame % 16, odd, "{arch} windows={windows}: frame {frame}");
            assert!(frame >= 8 * (1 + saved) + graphics_trampoline_shadow(windows));
        }
    }

    /// The four states are distinct, and `UNTRIED` is what a zeroed state block holds.
    #[test]
    fn the_states_are_distinct_and_untried_is_zero() {
        let states = [VULKAN_UNTRIED, VULKAN_BUILT, VULKAN_FAILED, VULKAN_BUILDING];
        let mut seen = std::collections::HashSet::new();
        for state in states {
            assert!(seen.insert(state_value(state)), "{state} is used twice");
        }
        assert_eq!(state_value(VULKAN_UNTRIED), 0);
        let modes = [BUILD_ON_ITS_OWN_THREAD, BUILD_ON_THIS_THREAD, BUILD_CLAIMED];
        let mut seen = std::collections::HashSet::new();
        for mode in modes {
            assert!(seen.insert(state_value(mode)), "{mode} is used twice");
        }
    }
}
