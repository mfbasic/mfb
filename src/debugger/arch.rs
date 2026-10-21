//! What the platform-neutral debugger code needs to know about a processor.
//!
//! The prompt, the disassembler, the unwinder, the breakpoint code and the ELF
//! reader never name a register, an instruction size or an opcode. They read
//! them from the [`Arch`] the [`Inferior`](super::Inferior) answers, so the same
//! code serves every processor a backend exists for, and a test can drive it with
//! any processor's descriptor on any host.

/// One processor, as the shared debugger code sees it.
pub(crate) struct Arch {
    /// The processor's name as `std::env::consts::ARCH` spells it.
    pub(crate) name: &'static str,
    /// The register names `list reg` shows for the program counter, the stack
    /// pointer, the register a call leaves its return address in, and the
    /// frame pointer.
    pub(crate) pc: &'static str,
    pub(crate) sp: &'static str,
    pub(crate) link: &'static str,
    pub(crate) frame: &'static str,
    /// The registers that carry a call's arguments, in order, under both
    /// MFBASIC's convention and C's; a call's results start in the same
    /// registers. `finish` shows the first two, MFBASIC's error code and value.
    pub(crate) arguments: [&'static str; 8],
    /// Whether a word names a register an address can be in.
    pub(crate) is_register: fn(&str) -> bool,
    /// Refuse a register `jump` does not set, with the reason.
    pub(crate) settable: fn(&str) -> Result<(), String>,
    /// Refuse a value a register `jump` sets cannot hold.
    pub(crate) holds: fn(&str, u64) -> Result<(), String>,
    /// Where an instruction may start: every address that is a multiple of
    /// this, and only those.
    pub(crate) alignment: u64,
    /// The most bytes one instruction takes.
    pub(crate) longest: usize,
    /// The instruction at the start of these bytes, at address `pc`, or `None`
    /// when the decoder does not know it or the bytes are too few.
    pub(crate) decode: fn(&[u8], u64) -> Option<Decoded>,
    /// The size of the instruction at the start of these bytes, read from its
    /// first bytes alone; [`Arch::alignment`] when there are too few to tell.
    pub(crate) length: fn(&[u8]) -> usize,
    /// What the instruction at the start of these bytes, at the address given,
    /// does to its function's frame.
    pub(crate) effect: fn(&[u8], u64) -> Effect,
    /// The bits of a user-space code address; a return address read from the
    /// stack is masked to them.
    pub(crate) address_mask: u64,
    /// The span a watchpoint must fit in without crossing a multiple of it,
    /// or `None` where the system provides no watchpoints.
    pub(crate) watch_span: Option<u64>,
    /// The processor's lines of the prompt's help.
    pub(crate) help: ArchHelp,
    /// How the processor's ELF files are told apart and read.
    pub(crate) elf: ElfArch,
}

/// One decoded instruction, as the disassembler and `finish` read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decoded {
    /// The bytes it takes.
    pub(crate) length: usize,
    /// `mnemonic operands`, the mnemonic padded to six columns.
    pub(crate) text: String,
    /// Whether it returns from the function.
    pub(crate) returns: bool,
    /// The address a branch, call or address-forming instruction names.
    pub(crate) target: Option<u64>,
    /// The value a move of an immediate sets, for a comment.
    pub(crate) value: Option<i128>,
    /// An instruction that sets a register to a page (or other partial)
    /// address: the register and the address.
    pub(crate) page: Option<(u32, u64)>,
    /// An instruction that adds an offset to a register a [`Decoded::page`] may
    /// have set: the register and the offset, so a listing can name the
    /// address the pair forms.
    pub(crate) page_offset: Option<(u32, i64)>,
}

/// A register as the unwinder tracks it: by the part it plays in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reg {
    /// The stack pointer.
    Sp,
    /// The register holding the return address on entry.
    Link,
    /// The frame pointer.
    Frame,
    /// Any other register, by number.
    Other(u8),
}

/// What one instruction does to its function's frame: the unwinder's reading
/// of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    /// `sp` moves by this much.
    AdjustSp(i64),
    /// `sp` set some way the walk cannot follow.
    SpUnknown,
    /// Registers stored at offsets from `base`.
    Store {
        base: Reg,
        pairs: Vec<(Reg, i64)>,
    },
    /// A pre-index store: `base` moves by `delta`, then the stores happen at
    /// offsets from its new value.
    StoreWriteback {
        base: Reg,
        pairs: Vec<(Reg, i64)>,
        delta: i64,
    },
    /// A post-index store: the stores happen at offsets from `base`, which
    /// then moves by `delta`.
    StoreThenMove {
        base: Reg,
        pairs: Vec<(Reg, i64)>,
        delta: i64,
    },
    /// Registers loaded; a post-index load from `sp` then moves it.
    Load {
        registers: Vec<Reg>,
        sp_delta: Option<i64>,
    },
    /// `register = sp + offset`.
    SpAddress {
        register: Reg,
        offset: i64,
    },
    /// `sp = frame + offset`.
    SpFromFp {
        offset: i64,
    },
    /// A return, an indirect jump, or a direct jump to `target`: control
    /// leaves this straight line.
    Leave {
        target: Option<u64>,
    },
    /// A hint: an instruction that changes nothing the walk follows and may
    /// sit in a prologue.
    Hint,
    Other,
}

impl Effect {
    /// Whether a prologue can hold this instruction: it sets up the frame,
    /// saves registers into it, or is a hint.
    pub(crate) fn in_prologue(&self) -> bool {
        match self {
            Effect::AdjustSp(delta) => *delta <= 0,
            Effect::Store { base, .. }
            | Effect::StoreWriteback { base, .. }
            | Effect::StoreThenMove { base, .. } => *base == Reg::Sp,
            Effect::SpAddress { .. } | Effect::Hint => true,
            _ => false,
        }
    }
}

/// The processor's parts of the prompt's help: register names, and the lines
/// that list them.
pub(crate) struct ArchHelp {
    /// What `jump` sets, as its help lists it after `j <where>
    /// [<register>=<value>]...`, wrapped to the help's columns.
    pub(crate) jump: &'static str,
    /// The `wp` lines after its first: the sizes it takes, or that it is not
    /// available.
    pub(crate) watch: &'static str,
    /// The `list reg` usage's lines for the `gpr`, `simd` and `sys` groups.
    pub(crate) groups: &'static str,
}

/// How a processor's ELF files are recognised and read.
pub(crate) struct ElfArch {
    /// `e_machine`.
    pub(crate) machine: u16,
    /// The relocation type that binds a function's GOT slot for a call
    /// through the PLT.
    pub(crate) jump_slot: u32,
    /// The relocation type that binds a GOT slot to a symbol's address.
    pub(crate) data_slot: u32,
    /// The size of one of the compiler's import stubs.
    pub(crate) stub_size: u64,
    /// The GOT slot an import stub of [`ElfArch::stub_size`] bytes at
    /// `address` loads its target from, or `None` when the bytes are not one.
    pub(crate) stub_slot: fn(&[u8], u64) -> Option<u64>,
}

/// The descriptor of the processor this `mfb` was built for, for text shown
/// with no program to ask (`mfb dbg --help`). A host with no debugger backend
/// shows AArch64's, the processor its help has always described.
pub(crate) fn host() -> &'static Arch {
    &super::arm64::AARCH64
}
