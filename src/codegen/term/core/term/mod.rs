//! Native code generation for the built-in `term::` console backend
//! (plan-01-term.md §6.1, Phase 2). Each helper emits a self-contained AArch64
//! runtime function that updates the term-state global (writable slots in the
//! program-entry frame, reached off the pinned arena-state register `x19` at
//! `term_state_offset`) and writes ANSI escape sequences to stdout.
//!
//! The §4.2.1 gate lives here: every helper except `term::on`/`term::isOn`
//! begins by loading the `active` flag and short-circuiting to a no-op (or, for
//! getters, the inert default) while TUI mode is off.

// --- codegen tier imports (migration) ---
use crate::codegen::builtins::io::gen_read_key_family::INPUT_EVENTS_SYMBOL;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::io::mouse::decode as mouse_decode;
use crate::codegen::io::mouse::ring as mouse_ring;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::term::grid as term_grid;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use std::collections::HashMap;

mod attributes;
mod draw;
mod kitty;
mod mouse;
mod screen_mode;
mod text;
use attributes::*;
use draw::*;
pub(crate) use kitty::emit_kitty_line_window;
use kitty::*;
use mouse::*;
pub(crate) use mouse::{emit_mouse_inject, emit_mouse_tracking_window, emit_store_mouse_mode};
use screen_mode::*;
pub(crate) use text::emit_encode_utf8;
use text::*;

// Frame layout. `LOCALS_SIZE` is the size of this locals region, which
// `finalize_vreg_body_with_locals` rounds to 16 and reserves; the vreg frame
// owns saving the link register, not a slot named here. The
// Darwin variadic `ioctl` spill is handled by the macOS `emit_terminal_size`
// hook, which brackets its own `sub_sp`/`str x2, [sp]`/`add_sp` around the call
// rather than commandeering a fixed slot here.
const LOCALS_SIZE: usize = 64;
const ARG0_OFFSET: usize = 8;
const ARG1_OFFSET: usize = 16;
/// Scratch buffer for runtime decimal formatting and the `winsize` struct.
const SCRATCH_OFFSET: usize = 32;
const SCRATCH_END: usize = 56;
/// bug-967: `term::on` parks the grid it is replacing here (a re-entered `on`)
/// across the new grid's allocation. The last word of `LOCALS_SIZE`; `term::on`
/// is a helper of its own, so it shares the word with nothing (`term::sync`'s
/// present uses the same offset in its own frame).
const PREVIOUS_GRID_OFFSET: usize = SCRATCH_END;

// plan-94-B: the two mouse members need 16 more bytes than every other helper —
// a `struct timespec` (or a pair of `LARGE_INTEGER`s on Windows) for the ring's
// monotonic stamp. They carry their own larger frame rather than raising
// `LOCALS_SIZE` for all 26, which would have re-sized every `term::` helper's
// stack and diffed every `term::` golden for a slot only two of them address.
const MOUSE_CLOCK_SCRATCH_OFFSET: usize = LOCALS_SIZE;
const MOUSE_LOCALS_SIZE: usize =
    MOUSE_CLOCK_SCRATCH_OFFSET + crate::codegen::io::mouse::clock::MOUSE_CLOCK_SCRATCH_BYTES;
// bug-1015: `term::pollMouse` decodes pending input before it dequeues, which
// needs a byte, a clock scratch, a pollfd and a deadline slot past the ordinary
// locals. Its own frame, for the same reason the mouse frame is its own.
const POLL_MOUSE_PENDING_OFFSET: usize = LOCALS_SIZE;
const POLL_MOUSE_LOCALS_SIZE: usize = POLL_MOUSE_PENDING_OFFSET
    + crate::codegen::builtins::io::gen_read_family::PendingInputSlots::BYTES;

const DARWIN_TIOCGWINSZ: &str = "1074295912";
const LINUX_TIOCGWINSZ: &str = "21523";

// Fixed ANSI escape-sequence byte strings (ESC = 0x1b). `term::on` resets state
// to defaults and switches to the alternate screen; `term::off` restores it.
//
// These two are the whole set. plan-35-C moved every other sequence — clear,
// bold, underline, cursor show/hide, the SGR colour prefixes, and the cursor
// -addressing pieces — into `term_grid/present.rs`'s `append_const`, which composes them
// into the shadow-grid diff. The thirteen leftover data objects here were still
// emitted into every binary that used `term::` while no emitted code referenced
// them (bug-326-A21).
const ESC_ON: &[u8] =
    b"\x1b[?1049h\x1b[0m\x1b[38;2;255;255;255m\x1b[48;2;0;0;0m\x1b[2J\x1b[H\x1b[?25h";
const ESC_OFF: &[u8] = b"\x1b[?25h\x1b[?1049l\x1b[0m";

const ESC_ON_SYMBOL: &str = "_mfb_term_esc_on";
const ESC_OFF_SYMBOL: &str = "_mfb_term_esc_off";

// plan-94-B: the mouse-tracking mode set/reset pair. Three modes, because they
// answer three different questions and a terminal needs all three answers:
//
//   1000  report button presses and releases at all
//   1002  also report motion WHILE A BUTTON IS HELD — without it a drag is
//         invisible: only the press and the release arrive, and everything in
//         between is lost
//   1006  encode the report in the SGR extended form
//
// **1006 is mandatory, not an optimisation.** The default encoding biases each
// coordinate by 32 into a single byte, so it cannot express a column past 223 —
// and plan-94-C/D/E inject *pixel* coordinates through the same decoder, where
// that ceiling is absurd. The SGR form writes plain decimal integers with no
// ceiling, which is what lets one decoder serve cells and pixels alike.
//
// Written only by `term::enableMouse` (opt-in) and by `term::off` (which resets
// unconditionally, so a program that forgets still leaves the terminal clean).
const ESC_MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const ESC_MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1002l\x1b[?1000l";
const ESC_MOUSE_ON_SYMBOL: &str = "_mfb_term_esc_mouse_on";
const ESC_MOUSE_OFF_SYMBOL: &str = "_mfb_term_esc_mouse_off";
/// The name of the SGR-injection environment variable, as a NUL-terminated C
/// string for `getenv` (plan-94-B Corrections B2).
const MOUSE_INJECT_ENV_SYMBOL: &str = "_mfb_term_mouse_inject_env";

// bug-699 Phase 5: the kitty keyboard protocol
// (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>, "Progressive
// enhancement"). `term::on` asks whether the terminal speaks it — `CSI ? u`
// (report the current flags) followed by `CSI c` (Primary Device Attributes,
// which every terminal answers) — and a flags reply arriving before the DA reply
// means it does. Only then does it push the flags (disambiguate 1, event types 2,
// alternate keys 4, all keys as escape codes 8, associated text 16 = 31; bug-699
// Decision 9), and `term::off` pops them.
const ESC_KITTY_QUERY: &[u8] = b"\x1b[?u\x1b[c";
const ESC_KITTY_PUSH: &[u8] = b"\x1b[>31u";
const ESC_KITTY_POP: &[u8] = b"\x1b[<u";
const ESC_KITTY_QUERY_SYMBOL: &str = "_mfb_term_esc_kitty_query";
const ESC_KITTY_PUSH_SYMBOL: &str = "_mfb_term_esc_kitty_push";
const ESC_KITTY_POP_SYMBOL: &str = "_mfb_term_esc_kitty_pop";
/// How long `term::on` waits for the terminal's replies. A local terminal
/// answers in well under a millisecond; this is for a slow remote session, and
/// for a terminal that answers neither query (which then counts as not speaking
/// the protocol).
const KITTY_PROBE_MS: u64 = 500;
// The probe's frame slots, appended past the ordinary locals (only `term::on` in
// a program with key readers carries them).
const KITTY_BYTE_OFFSET: usize = LOCALS_SIZE;
const KITTY_CLOCK_OFFSET: usize = LOCALS_SIZE + 8;
const KITTY_POLLFD_OFFSET: usize =
    KITTY_CLOCK_OFFSET + crate::codegen::io::mouse::clock::MOUSE_CLOCK_SCRATCH_BYTES;
const KITTY_DEADLINE_OFFSET: usize = KITTY_POLLFD_OFFSET + 8;
const KITTY_STATE_OFFSET: usize = KITTY_DEADLINE_OFFSET + 8;
const KITTY_FOUND_OFFSET: usize = KITTY_STATE_OFFSET + 8;
const KITTY_LOCALS_SIZE: usize = KITTY_FOUND_OFFSET + 8;

/// The kitty escape sequences, emitted with `term::` in a program whose `io`
/// readers decode keys (the only programs `term::on` switches the protocol for).
pub(crate) fn kitty_data_objects() -> Vec<CodeDataObject> {
    [
        (ESC_KITTY_QUERY_SYMBOL, ESC_KITTY_QUERY),
        (ESC_KITTY_PUSH_SYMBOL, ESC_KITTY_PUSH),
        (ESC_KITTY_POP_SYMBOL, ESC_KITTY_POP),
    ]
    .iter()
    .map(|(symbol, bytes)| CodeDataObject {
        symbol: (*symbol).to_string(),
        kind: "raw".to_string(),
        layout: "ANSI escape sequence (raw bytes)".to_string(),
        align: 1,
        size: bytes.len(),
        value: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    })
    .collect()
}

/// Bytes allocated for the `color::Color` record `term::getForeground`/`getBackground`
/// return: four `Byte` fields, one 8-byte slot each (plan-122-F widened it from the
/// retired 3-field `TermColor`).
const COLOR_RECORD_SIZE: usize = 32;
const TERM_SIZE_RECORD_SIZE: usize = 16;
/// Bytes allocated for the `term::MouseEvent` record `term::pollMouse` returns:
/// seven props (`kind`, `button`, `row`, `column`, `shift`, `ctrl`, `alt`), one
/// 8-byte value slot each. An enum prop is an ordinal, not a composite, so none of
/// the seven is inlined and the block carries no trailing data region — the flat
/// `8 * fields.len()` layout `pointer_field_offset` assumes.
const MOUSE_EVENT_RECORD_SIZE: usize = 56;
/// Default foreground while inactive (white, packed `r | g<<8 | b<<16`).
const DEFAULT_FOREGROUND_PACKED: &str = "16777215";

fn esc_entries() -> &'static [(&'static str, &'static [u8])] {
    &[(ESC_ON_SYMBOL, ESC_ON), (ESC_OFF_SYMBOL, ESC_OFF)]
}

/// The mouse-only data objects, emitted **only** for a program that uses
/// `enableMouse`/`pollMouse`.
///
/// Deliberately not folded into [`esc_entries`]: doing that would put them in
/// every binary that uses `term::` at all, with nothing referencing them in the
/// overwhelming majority — which is precisely the dead-data-object bug
/// (bug-326-A21) that trimmed this table down to two entries in the first place.
/// Keyed on the same `uses_mouse` test that reserves the arena region, so the
/// data and the storage appear and disappear together.
pub(crate) fn mouse_data_objects() -> Vec<CodeDataObject> {
    let mut objects: Vec<CodeDataObject> = [
        (ESC_MOUSE_ON_SYMBOL, ESC_MOUSE_ON),
        (ESC_MOUSE_OFF_SYMBOL, ESC_MOUSE_OFF),
    ]
    .iter()
    .map(|(symbol, bytes)| CodeDataObject {
        symbol: (*symbol).to_string(),
        kind: "raw".to_string(),
        layout: "ANSI escape sequence (raw bytes)".to_string(),
        align: 1,
        size: bytes.len(),
        value: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    })
    .collect();
    // The injection variable's name, NUL-terminated for `getenv`.
    let mut name = MOUSE_INJECT_ENV.as_bytes().to_vec();
    name.push(0);
    objects.push(CodeDataObject {
        symbol: MOUSE_INJECT_ENV_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "NUL-terminated C string (environment variable name)".to_string(),
        align: 1,
        size: name.len(),
        value: name.iter().map(|byte| format!("{byte:02x}")).collect(),
    });
    objects
}

/// Read-only data objects for the fixed escape-sequence byte strings.
pub(crate) fn console_data_objects() -> Vec<CodeDataObject> {
    esc_entries()
        .iter()
        .map(|(symbol, bytes)| CodeDataObject {
            symbol: (*symbol).to_string(),
            kind: "raw".to_string(),
            layout: "ANSI escape sequence (raw bytes)".to_string(),
            align: 1,
            size: bytes.len(),
            value: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        })
        .collect()
}

fn data_reloc(from: &str, symbol: &str, kind: RelocIntent) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind,
        binding: "data".to_string(),
        library: None,
    }
}

/// Materialize the address of a data symbol into `dst` (adrp + add page-off).
fn load_data_address(
    from: &str,
    symbol: &str,
    dst: impl Into<Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let dst = dst.into();
    instructions.push(abi::load_page_address(&dst, symbol));
    relocations.push(data_reloc(from, symbol, RelocIntent::DataAddrHi));
    instructions.push(abi::add_page_offset(&dst, &dst, symbol));
    relocations.push(data_reloc(from, symbol, RelocIntent::DataAddrLo));
}

/// Emit a write of a fixed escape-sequence byte string to stdout (fd 1). The
/// write result is intentionally ignored: a failed escape write is not a program
/// error (term setters are best-effort, plan §4.2.1 / §9.4).
fn emit_write_const(ctx: &mut EmitCtx, data_symbol: &str, len: usize) -> Result<(), String> {
    // This helper's original parameters were (from, symbol): `from` is the
    // emitting function (each relocation's source) and `symbol` is the constant
    // being addressed. They are NOT the usual order, so name them explicitly —
    // conflating them emits relocations sourced from the data symbol, which the
    // linker rejects with "relocation source does not match function".
    let from = ctx.symbol;
    let symbol = data_symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    load_data_address(
        from,
        symbol,
        abi::string_data_register(),
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::move_immediate(
        abi::string_length_register(),
        "Integer",
        &len.to_string(),
    ));
    ctx.instructions
        .push(abi::move_immediate(abi::return_register(), "Integer", "1"));
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)
}

/// Load `active` and branch to `target` when TUI mode is off (the §4.2.1 gate).
///
/// `scratch` is a caller-minted vreg that holds the flag only until the branch.
/// Each caller passes one of its own vregs that is not yet live at the gate (it
/// is written before it is next read), so the gate adds no register of its own.
fn emit_gate_inactive(
    term_state_offset: usize,
    target: &str,
    scratch: Reg,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.push(abi::load_u64(
        scratch,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    instructions.push(abi::compare_immediate(scratch, "0"));
    instructions.push(abi::branch_eq(target));
}

/// Emit a `Result.Err(ERR_UNSUPPORTED_OPERATION)` into the result registers.
fn emit_unsupported(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    raise_error_into(symbol, "ErrUnsupported", instructions, relocations);
}

pub(crate) fn lower_term_helper(
    call: &str,
    symbol: &str,
    term_state_offset: usize,
    // plan-94-B: `Some` only for a program that uses `enableMouse`/`pollMouse`.
    // Only the two mouse arms read it.
    mouse_state_offset: Option<usize>,
    // bug-699: `Some` only for a program whose `io` readers decode keys. Only
    // `on`/`off` read it — they switch the kitty keyboard protocol for it.
    input_state_offset: Option<usize>,
    // plan-94-B: whether this is an `--app` build. Only the mouse arms read it,
    // to decide whether writing terminal mode sequences means anything.
    app_mode: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>, usize), String> {
    // Vreg-allocated (plan-00-G Phase 2): the decimal/record-build scratch buffers
    // are an explicit sp-relative local region; x9-x15 scratch becomes vregs. The
    // `abi_function` wrapper (`term::gen_shared per-member body`) seeds the
    // entry label and finalizes; this body returns the pre-finalize
    // `(instructions, relocations, stack_size)` it consumes.
    let done = format!("{symbol}_done");
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    // Every helper but the two mouse ones reserves exactly `LOCALS_SIZE`; those two
    // raise it (plan-94-B), so nothing else diffs.
    let mut locals = LOCALS_SIZE;
    // bug-721: the one minter for this whole helper body. Every emitter below —
    // and every shared sub-emitter they call (`term_grid`, the mouse ring) — takes
    // its vregs from here, so no two of them can name the same register by
    // accident.
    let mut vregs = Vregs::new();
    let vregs = &mut vregs;

    match call {
        "term.on" => {
            emit_on(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                term_state_offset,
                mouse_state_offset,
                input_state_offset.filter(|_| !app_mode),
                &done,
                vregs,
            )?;
            if input_state_offset.is_some() && !app_mode {
                locals = KITTY_LOCALS_SIZE;
            }
        }
        "term.off" => {
            emit_off(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                term_state_offset,
                mouse_state_offset,
                input_state_offset.filter(|_| !app_mode),
                &done,
                vregs,
            )?;
        }
        "term.isOn" => emit_is_on(term_state_offset, &mut instructions),
        "term.setForeground" => emit_set_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_FG_OFFSET,
            &mut instructions,
            vregs,
        ),
        "term.setBackground" => emit_set_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BG_OFFSET,
            &mut instructions,
            vregs,
        ),
        "term.setBold" => emit_set_attr(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BOLD_OFFSET,
            &mut instructions,
            vregs,
        ),
        "term.setUnderline" => emit_set_attr(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_UNDERLINE_OFFSET,
            &mut instructions,
            vregs,
        ),
        "term.showCursor" => {
            emit_set_cursor_visible(symbol, term_state_offset, "1", &mut instructions, vregs)
        }
        "term.hideCursor" => {
            emit_set_cursor_visible(symbol, term_state_offset, "0", &mut instructions, vregs)
        }
        "term.clear" => emit_clear_grid(symbol, term_state_offset, &mut instructions, vregs),
        "term.sync" => {
            // plan-35-C: present the frame — diff the back buffer against the
            // last-presented front buffer and emit only the changed cells as one
            // batched write. A no-op while TUI mode is off (grid pointer null).
            let request = match platform.family() {
                PlatformFamily::MacOS => DARWIN_TIOCGWINSZ,
                PlatformFamily::Linux => LINUX_TIOCGWINSZ,
                // Windows ignores the ioctl request value; emit_terminal_size
                // uses GetConsoleScreenBufferInfo. A placeholder keeps the match total.
                PlatformFamily::Windows => "0",
            };
            term_grid::emit_grid_present(
                symbol,
                term_state_offset,
                SCRATCH_END,
                request,
                platform,
                platform_imports,
                &mut instructions,
                &mut relocations,
                vregs,
            )?;
            instructions.push(abi::move_immediate(
                RESULT_TAG_REGISTER,
                "Integer",
                RESULT_OK_TAG,
            ));
        }
        "term.moveTo" => emit_move_to(symbol, term_state_offset, &mut instructions, vregs),
        "term.drawHLine" => {
            emit_draw_line(symbol, term_state_offset, true, &mut instructions, vregs)
        }
        "term.drawVLine" => {
            emit_draw_line(symbol, term_state_offset, false, &mut instructions, vregs)
        }
        "term.drawBox" => emit_draw_box(symbol, term_state_offset, &mut instructions, vregs),
        "term.fillRect" => emit_fill_rect(symbol, term_state_offset, &mut instructions, vregs),
        "term.drawText" => emit_draw_text(
            symbol,
            term_state_offset,
            &mut instructions,
            &mut relocations,
            vregs,
        ),
        "term.drawGlyph" => emit_draw_glyph(
            symbol,
            term_state_offset,
            &mut instructions,
            &mut relocations,
            vregs,
        ),
        "term.getForeground" => emit_get_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_FG_OFFSET,
            DEFAULT_FOREGROUND_PACKED,
            &done,
            &mut instructions,
            &mut relocations,
            vregs,
        ),
        "term.getBackground" => emit_get_color(
            symbol,
            term_state_offset,
            term_state_offset + TERM_STATE_BG_OFFSET,
            "0",
            &done,
            &mut instructions,
            &mut relocations,
            vregs,
        ),
        "term.getBold" => emit_get_attr(
            term_state_offset,
            term_state_offset + TERM_STATE_BOLD_OFFSET,
            &done,
            &mut instructions,
            vregs,
        ),
        "term.getUnderline" => emit_get_attr(
            term_state_offset,
            term_state_offset + TERM_STATE_UNDERLINE_OFFSET,
            &done,
            &mut instructions,
            vregs,
        ),
        "term.terminalSize" => emit_terminal_size(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            &done,
            vregs,
        )?,
        "term.didResize" => emit_did_resize(term_state_offset, &mut instructions, vregs),
        "term.enableMouse" => {
            emit_enable_mouse(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                mouse_state_offset,
                app_mode,
                &done,
                vregs,
            )?;
            // The mouse arms reach past the shared scratch for their clock buffer,
            // so they alone carry the larger frame; every other helper keeps
            // `LOCALS_SIZE` and its exact goldens.
            locals = MOUSE_LOCALS_SIZE;
        }
        "term.pollMouse" => {
            emit_poll_mouse(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                mouse_state_offset,
                input_state_offset,
                app_mode,
                &done,
                vregs,
            )?;
            locals = POLL_MOUSE_LOCALS_SIZE;
        }
        other => return Err(format!("unknown term runtime helper '{other}'")),
    }

    instructions.push(abi::label(&done));
    instructions.push(abi::return_());

    Ok((instructions, relocations, locals))
}

fn emit_is_on(term_state_offset: usize, instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::didResize()` (planning/term.md #11): read the cached resize flag and
/// clear it in the same helper, so the flag latches `true` from the moment a
/// terminal/window size change is detected until the program observes it, then
/// resets. Like `term::isOn` this getter is not gated on `active` — an inactive
/// terminal never sets the flag, so it simply reads 0/false. The flag is set by
/// the shared CLI reflow (`term_grid::emit_grid_resize`) and, in `--app` mode, by
/// each app backend's resize hook mirroring into this same term-state slot, so the
/// shared getter is correct in both modes.
fn emit_did_resize(
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v9: Reg = vregs.next().into();
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_DID_RESIZE_OFFSET,
    ));
    instructions.push(abi::move_immediate(&v9, "Integer", "0"));
    instructions.push(abi::store_u64(
        &v9,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_DID_RESIZE_OFFSET,
    ));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// Grid context + throwaway scratch registers shared by the run/cell stampers.
/// `rows`/`cols` are the grid dims, `back` the back-cell base, and the four
/// attribute registers the current fg/bg/bold/underline; the remaining fields are
/// scratch the stampers clobber, which the caller must keep disjoint from any
/// value that must stay live across a stamp (e.g. the normalised box extents).
struct StampCtx {
    rows: Reg,
    cols: Reg,
    back: Reg,
    fg: Reg,
    bg: Reg,
    bold: Reg,
    un: Reg,
    lo: Reg,
    hi: Reg,
    idx: Reg,
    cell: Reg,
    pos: Reg,
    tmp: Reg,
}

fn emit_terminal_size(
    ctx: &mut EmitCtx,
    term_state_offset: usize,
    done: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let active_flag: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();

    let unsupported = format!("{symbol}_unsupported");
    let active = format!("{symbol}_active");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let request = match platform.family() {
        PlatformFamily::MacOS => DARWIN_TIOCGWINSZ,
        PlatformFamily::Linux => LINUX_TIOCGWINSZ,
        // Windows ignores the ioctl request value (emit_terminal_size uses
        // GetConsoleScreenBufferInfo); a placeholder keeps the match total.
        PlatformFamily::Windows => "0",
    };
    // Gate: terminalSize is the one read with no inert value; while inactive it
    // returns ERR_UNSUPPORTED_OPERATION (§4.7).
    ctx.instructions.push(abi::load_u64(
        &active_flag,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    ctx.instructions
        .push(abi::compare_immediate(&active_flag, "0"));
    ctx.instructions.push(abi::branch_ne(&active));
    emit_unsupported(symbol, ctx.instructions, ctx.relocations);
    ctx.instructions.push(abi::branch(done));
    ctx.instructions.push(abi::label(&active));
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_immediate(abi::c_arg(1), "Integer", request),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), SCRATCH_OFFSET),
    ]);
    platform.emit_terminal_size(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&unsupported),
        abi::load_u16(&rows, abi::stack_pointer(), SCRATCH_OFFSET),
        abi::load_u16(&cols, abi::stack_pointer(), SCRATCH_OFFSET + 2),
        abi::compare_immediate(&rows, "0"),
        abi::branch_eq(&unsupported),
        abi::compare_immediate(&cols, "0"),
        abi::branch_eq(&unsupported),
        abi::store_u64(&rows, abi::stack_pointer(), ARG0_OFFSET),
        abi::store_u64(&cols, abi::stack_pointer(), ARG1_OFFSET),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &TERM_SIZE_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::load_u64(&rows, abi::stack_pointer(), ARG0_OFFSET),
        abi::load_u64(&cols, abi::stack_pointer(), ARG1_OFFSET),
        abi::store_u64(&cols, abi::mfb_return(1), 0),
        abi::store_u64(&rows, abi::mfb_return(1), 8),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&unsupported),
    ]);
    emit_unsupported(symbol, ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(done), abi::label(&alloc_error)]);
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
    Ok(())
}
