//! The four keystroke readers — `io::readRaw`, `io::readScalar`,
//! `io::readGrapheme`, `io::readKey` (bug-699 Decision 1) — lowered through one
//! body that differs only in what it takes from the read engine
//! (`codegen::io::input::reader`) and what it returns.
//!
//! Every one of them:
//! - takes `[timeoutMs]` by the language's timeout convention: omitted waits
//!   for as long as it takes, `0` is one immediate attempt, a positive value
//!   waits that long, a negative one is `ErrInvalidArgument`, and a wait that
//!   runs out raises `ErrTimeout` with nothing consumed;
//! - reads one key at a time: on a terminal the line discipline is switched to
//!   single-key, no-echo for the call (and restored), and an `--app` window to
//!   raw input, for the WAIT as well as the read — so a key typed while the
//!   reader waits arrives at once, not at the next Return;
//! - reads through the input buffer, so readers can be mixed without losing or
//!   reordering input.
//!
//! What each takes:
//! - `readRaw`: the next byte, exactly as the platform delivered it;
//! - `readScalar`: the next scalar of text — keys with no Unicode value, and
//!   releases, are consumed and skipped (Decision 2); a key that typed several
//!   scalars gives the rest back to the buffer for the next read;
//! - `readGrapheme`: the next extended grapheme cluster of text, with the same
//!   skipping;
//! - `readKey`: the next key event, as an `io::Key`.
//!
//! Ill-formed UTF-8 is dropped, never reported (Decision 2).

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::io::input::decode::{
    FLAG_CLUSTER, TEXT_GRAPHEME, TEXT_NONE, TOKEN_EVENT, TOKEN_GRAPHEME, TOKEN_GRAPHEME_LEN,
    TOKEN_KEY, TOKEN_MODS, TOKEN_TEXT_KIND, TOKEN_TEXT_SCALAR,
};
use crate::codegen::io::input::keys::{KeyEvent, MOD_ALT, MOD_CTRL, MOD_META, MOD_SHIFT};
use crate::codegen::io::input::reader::{
    emit_engine, emit_reader_setup, engine_done_label, engine_next_label, Engine, EngineLabels,
    ReaderSlots, ENGINE_BYTE, ENGINE_TOKEN, OUT_BYTE, OUT_INVALID,
};
use crate::codegen::io::input::{INPUT_STATE_LEN_OFFSET, INPUT_STATE_TOKEN_OFFSET};
use crate::codegen::io::stdin::field_addr;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::memory::marshal::{
    emit_build_inlined_record_sized, MarshalRegs, RecordBuildScratch,
};
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Which keystroke reader a body is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyReader {
    Raw,
    Scalar,
    Grapheme,
    Key,
}

impl KeyReader {
    fn text(self) -> &'static str {
        match self {
            KeyReader::Raw => "io.readRaw",
            KeyReader::Scalar => "io.readScalar",
            KeyReader::Grapheme => "io.readGrapheme",
            KeyReader::Key => "io.readKey",
        }
    }

    fn return_type(self) -> ParameterType {
        match self {
            KeyReader::Raw => ParameterType::Byte,
            KeyReader::Scalar => crate::codegen::engine::types::type_utils::scalar_type(),
            KeyReader::Grapheme => ParameterType::String,
            KeyReader::Key => ParameterType::named(super::KEY_TYPE),
        }
    }
}

/// The process-global word `term::on` sets while the terminal reports key
/// events (bug-699 Phase 5).
pub(crate) const INPUT_EVENTS_SYMBOL: &str = "_mfb_rt_input_events";

// Frame layout (sp-relative).
const TIMEOUT: usize = 8;
const SLOTS: usize = 16;
const TERMINAL: usize = SLOTS + ReaderSlots::BYTES; // 104
const TERM_ACTIVE: usize = TERMINAL;
const TERM_SAVED_TAG: usize = TERMINAL + 8;
const TERM_SAVED_VALUE: usize = TERMINAL + 16;
const TERM_SAVED_MESSAGE: usize = TERMINAL + 24;
const TERM_ORIGINAL: usize = TERMINAL + 32;
/// Room for either platform's `struct termios` (macOS 72 bytes, Linux 60).
const TERMIOS_BYTES: usize = 72;
const TERM_MODIFIED: usize = TERM_ORIGINAL + TERMIOS_BYTES;
const RESULT: usize = TERM_MODIFIED + TERMIOS_BYTES; // the result, across the restore
/// `io::Key` field slots (declaration order) and the record builder's scratch.
const KEY_FIELDS: usize = RESULT + 8;
const KEY_FIELD_COUNT: usize = 7;
const KEY_SCRATCH: usize = KEY_FIELDS + 8 * KEY_FIELD_COUNT;
const FRAME: usize = KEY_SCRATCH + 32;

/// Shared `abi_function` body for the four keystroke readers.
pub(crate) fn lower_key_reader(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
    which: KeyReader,
) -> Result<ValueResult, String> {
    let symbol_owned = builder.current_symbol.clone();
    let symbol: &str = &symbol_owned;
    let platform_imports = ctx.platform_imports;
    let platform = ctx.platform;
    let app_mode = ctx.build_mode.is_app();
    let state_offset = ctx.input_state_offset.ok_or_else(|| {
        format!("'{symbol}' reads through the input buffer, which this program did not reserve")
    })?;
    let l = |name: &str| format!("{symbol}_{name}");
    let invalid_argument = l("invalid_argument");
    let timeout = l("timeout");
    let eof = l("eof");
    let input_error = l("input_error");
    let invalid_context = l("invalid_context");
    let alloc_error = l("alloc_error");
    let done = l("done");
    let terminal_slots = TerminalModeSlots {
        active: TERM_ACTIVE,
        saved_tag: TERM_SAVED_TAG,
        saved_value: TERM_SAVED_VALUE,
        saved_message: TERM_SAVED_MESSAGE,
        original: TERM_ORIGINAL,
        modified: TERM_MODIFIED,
    };

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let timeout_arg = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects its timeout argument"))?
        .location
        .clone();
    instructions.push(abi::store_u64(&timeout_arg, abi::stack_pointer(), TIMEOUT));
    // The terminal-restore flag reads zero until the terminal is configured, so
    // the shared restore at `done` is a no-op on the negative-timeout exit taken
    // before it (it read whatever the frame held — `readKey(-1)` raised
    // `ErrInputFailed` instead of `ErrInvalidArgument`).
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), TERM_ACTIVE));
    // The timeout convention: the omitted-argument sentinel waits without bound,
    // any other negative value is `ErrInvalidArgument`.
    let t = vregs.next();
    let sentinel = vregs.next();
    let timeout_ok = l("timeout_ok");
    instructions.extend([
        abi::load_u64(t, abi::stack_pointer(), TIMEOUT),
        abi::move_immediate(sentinel, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(t, sentinel),
        abi::branch_eq(&timeout_ok),
        abi::compare_immediate(t, "0"),
        abi::branch_lt(&invalid_argument),
        abi::label(&timeout_ok),
    ]);
    // Drain buffered stdout before waiting (plan-14-A §4.3 hook 2); no stdout
    // buffer in app mode.
    if !app_mode {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    }
    // One key at a time, for the wait and the read.
    if app_mode {
        platform
            .emit_app_raw_input_mode(symbol, &mut instructions, &mut relocations)
            .ok_or_else(|| {
                format!(
                    "native target '{}' does not support app-mode raw input",
                    platform.target()
                )
            })??;
    }
    {
        let mut e = EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        };
        emit_configure_stdin_terminal(
            &mut e,
            &mut vregs,
            &terminal_slots,
            abi::stack_pointer(),
            true,
            true,
            &input_error,
        )?;
    }
    let slots = ReaderSlots::at(SLOTS);
    let engine = Engine {
        state_offset,
        app_mode,
        mouse_state_offset: ctx.mouse_state_offset,
        slots,
        events_symbol: (!app_mode).then_some(INPUT_EVENTS_SYMBOL),
    };
    {
        let mut e = EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        };
        let t = vregs.next();
        e.instructions
            .push(abi::load_u64(t, abi::stack_pointer(), TIMEOUT));
        emit_reader_setup(&engine, t, &mut e, &mut vregs)?;
        let (mode, flags) = match which {
            KeyReader::Raw => (ENGINE_BYTE, 0),
            KeyReader::Scalar => (ENGINE_TOKEN, 0),
            KeyReader::Grapheme | KeyReader::Key => (ENGINE_TOKEN, FLAG_CLUSTER),
        };
        let v = vregs.next();
        e.instructions.extend([
            abi::move_immediate(v, "Integer", &mode.to_string()),
            abi::store_u64(v, abi::stack_pointer(), slots.mode),
            abi::move_immediate(v, "Integer", &flags.to_string()),
            abi::store_u64(v, abi::stack_pointer(), slots.flags),
        ]);
        emit_engine(
            &engine,
            &EngineLabels {
                timeout: &timeout,
                eof: &eof,
                input_error: &input_error,
                invalid_context: &invalid_context,
            },
            &mut e,
            &mut vregs,
        )?;
    }
    let next = engine_next_label(symbol);
    let _ = engine_done_label(symbol);
    let token = |offset: usize| INPUT_STATE_TOKEN_OFFSET + offset;
    let region = |dst: VirtualRegister, instructions: &mut Vec<CodeInstruction>| {
        field_addr(dst, ARENA_STATE_REGISTER, state_offset, instructions);
    };
    let outcome = vregs.next();
    instructions.push(abi::load_u64(outcome, abi::stack_pointer(), slots.outcome));
    match which {
        KeyReader::Raw => {
            // Byte mode only ever yields a byte.
            let r = vregs.next();
            let b = vregs.next();
            region(r, &mut instructions);
            instructions.extend([
                abi::load_u64(b, r, token(TOKEN_TEXT_SCALAR)),
                abi::store_u64(b, abi::stack_pointer(), RESULT),
                abi::branch(&l("ok")),
            ]);
            let _ = OUT_BYTE;
        }
        KeyReader::Scalar | KeyReader::Grapheme => {
            // Dropped ill-formed bytes, a release, or a key with no text: next.
            let r = vregs.next();
            let v = vregs.next();
            instructions.extend([
                abi::compare_immediate(outcome, &OUT_INVALID.to_string()),
                abi::branch_eq(&next),
            ]);
            region(r, &mut instructions);
            instructions.extend([
                abi::load_u64(v, r, token(TOKEN_EVENT)),
                abi::compare_immediate(v, &(KeyEvent::Release as u64).to_string()),
                abi::branch_eq(&next),
                abi::load_u64(v, r, token(TOKEN_TEXT_KIND)),
                abi::compare_immediate(v, &TEXT_NONE.to_string()),
                abi::branch_eq(&next),
            ]);
            if which == KeyReader::Scalar {
                emit_take_scalar(
                    symbol,
                    state_offset,
                    &input_error,
                    &mut instructions,
                    &mut vregs,
                );
            } else {
                emit_take_grapheme(
                    symbol,
                    state_offset,
                    &alloc_error,
                    &mut instructions,
                    &mut relocations,
                    &mut vregs,
                );
            }
            instructions.push(abi::branch(&l("ok")));
        }
        KeyReader::Key => {
            instructions.extend([
                abi::compare_immediate(outcome, &OUT_INVALID.to_string()),
                abi::branch_eq(&next),
            ]);
            emit_build_key(
                symbol,
                state_offset,
                &alloc_error,
                &mut instructions,
                &mut relocations,
                &mut vregs,
            )?;
            instructions.push(abi::branch(&l("ok")));
        }
    }

    instructions.extend([
        abi::label(&l("ok")),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    for (label, error) in [
        (&invalid_argument, "ErrInvalidArgument"),
        (&timeout, "ErrTimeout"),
        (&eof, "ErrEndOfFile"),
        (&input_error, "ErrInputFailed"),
        (&invalid_context, "ErrInvalidContext"),
        (&alloc_error, "ErrOutOfMemory"),
    ] {
        instructions.push(abi::label(label));
        raise_error_into(symbol, error, &mut instructions, &mut relocations);
        instructions.push(abi::branch(&done));
    }
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
        &terminal_slots,
    )?;
    instructions.push(abi::return_());
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = FRAME;
    Ok(ValueResult {
        origin: None,
        type_: which.return_type(),
        location: Operand::from("void"),
        text: which.text().to_string(),
    })
}

/// `RESULT` = the token's first scalar. A token whose text is several scalars
/// (a kitty text event, a key that typed a composed sequence) gives the rest
/// back to the front of the input buffer, where the next read finds them.
fn emit_take_scalar(
    symbol: &str,
    state_offset: usize,
    input_error: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let l = |name: &str| format!("{symbol}_scalar_{name}");
    let token = |offset: usize| INPUT_STATE_TOKEN_OFFSET + offset;
    let r = vregs.next();
    let kind = vregs.next();
    let v = vregs.next();
    field_addr(r, ARENA_STATE_REGISTER, state_offset, instructions);
    instructions.extend([
        abi::load_u64(kind, r, token(TOKEN_TEXT_KIND)),
        abi::compare_immediate(kind, &TEXT_GRAPHEME.to_string()),
        abi::branch_eq(&l("from_grapheme")),
        // The text is one scalar.
        abi::load_u64(v, r, token(TOKEN_TEXT_SCALAR)),
        abi::store_u64(v, abi::stack_pointer(), RESULT),
        abi::branch(&l("done")),
        abi::label(&l("from_grapheme")),
    ]);
    // Decode the grapheme's first scalar (the decoder only ever yields valid
    // UTF-8) and its length in bytes.
    let p = vregs.next();
    let b0 = vregs.next();
    let cp = vregs.next();
    let n = vregs.next();
    let b = vregs.next();
    let t = vregs.next();
    instructions.extend([
        abi::add_immediate(p, r, token(TOKEN_GRAPHEME)),
        abi::load_u8(b0, p, 0),
        abi::compare_immediate(b0, "128"),
        abi::branch_lo(&l("one")),
        abi::compare_immediate(b0, "224"),
        abi::branch_lo(&l("two")),
        abi::compare_immediate(b0, "240"),
        abi::branch_lo(&l("three")),
        // four: cp = (b0 & 7) << 18 | ...
        abi::move_immediate(t, "Integer", "7"),
        abi::and_registers(cp, b0, t),
        abi::move_immediate(n, "Integer", "4"),
        abi::branch(&l("cont")),
        abi::label(&l("three")),
        abi::move_immediate(t, "Integer", "15"),
        abi::and_registers(cp, b0, t),
        abi::move_immediate(n, "Integer", "3"),
        abi::branch(&l("cont")),
        abi::label(&l("two")),
        abi::move_immediate(t, "Integer", "31"),
        abi::and_registers(cp, b0, t),
        abi::move_immediate(n, "Integer", "2"),
        abi::branch(&l("cont")),
        abi::label(&l("one")),
        abi::move_register(cp, b0),
        abi::move_immediate(n, "Integer", "1"),
        abi::branch(&l("have")),
        // cp = cp << 6 | (b & 0x3F) for each continuation byte.
        abi::label(&l("cont")),
    ]);
    let i = vregs.next();
    let at = vregs.next();
    instructions.extend([
        abi::move_immediate(i, "Integer", "1"),
        abi::label(&l("cont_loop")),
        abi::compare_registers(i, n),
        abi::branch_eq(&l("have")),
        abi::add_registers(at, p, i),
        abi::load_u8(b, at, 0),
        abi::move_immediate(t, "Integer", "63"),
        abi::and_registers(b, b, t),
        abi::shift_left_immediate(cp, cp, 6),
        abi::or_registers(cp, cp, b),
        abi::add_immediate(i, i, 1),
        abi::branch(&l("cont_loop")),
        abi::label(&l("have")),
        abi::store_u64(cp, abi::stack_pointer(), RESULT),
    ]);
    // Give the rest back: the grapheme's remaining bytes, in front of whatever
    // is still buffered. They are a handful of bytes and the token has just been
    // consumed out of the buffer, so they fit.
    let glen = vregs.next();
    let rest = vregs.next();
    field_addr(r, ARENA_STATE_REGISTER, state_offset, instructions);
    instructions.extend([
        abi::load_u64(glen, r, token(TOKEN_GRAPHEME_LEN)),
        abi::subtract_registers(rest, glen, n),
        abi::compare_immediate(rest, "0"),
        abi::branch_eq(&l("done")),
        abi::add_immediate(p, r, token(TOKEN_GRAPHEME)),
        abi::add_registers(p, p, n),
    ]);
    // The rest always fits: it is re-encoded text of the token just consumed,
    // and no key's text is longer than the bytes that carried it (a kitty event
    // spells each code point in at least as many decimal digits as its UTF-8
    // has bytes). `input_error` guards the invariant rather than overrun.
    crate::codegen::io::input::buffer::emit_unread(
        symbol,
        p,
        rest,
        state_offset,
        input_error,
        instructions,
        vregs,
    );
    instructions.push(abi::label(&l("done")));
}

/// `RESULT` = a new `String` holding the token's text: its grapheme, or its one
/// (ASCII control) scalar.
fn emit_take_grapheme(
    symbol: &str,
    state_offset: usize,
    alloc_error: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let l = |name: &str| format!("{symbol}_grapheme_{name}");
    let token = |offset: usize| INPUT_STATE_TOKEN_OFFSET + offset;
    // A scalar-text token's text is one ASCII control byte: write it into the
    // grapheme field so both shapes are copied the same way.
    let r = vregs.next();
    let kind = vregs.next();
    let v = vregs.next();
    let one = vregs.next();
    field_addr(r, ARENA_STATE_REGISTER, state_offset, instructions);
    instructions.extend([
        abi::load_u64(kind, r, token(TOKEN_TEXT_KIND)),
        abi::compare_immediate(kind, &TEXT_GRAPHEME.to_string()),
        abi::branch_eq(&l("have")),
        abi::load_u64(v, r, token(TOKEN_TEXT_SCALAR)),
        abi::store_u8(v, r, token(TOKEN_GRAPHEME)),
        abi::move_immediate(one, "Integer", "1"),
        abi::store_u64(one, r, token(TOKEN_GRAPHEME_LEN)),
        abi::label(&l("have")),
    ]);
    let glen = vregs.next();
    instructions.extend([
        abi::load_u64(glen, r, token(TOKEN_GRAPHEME_LEN)),
        abi::add_immediate(abi::return_register(), glen, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    let dst = vregs.next();
    let src = vregs.next();
    let n = vregs.next();
    let b = vregs.next();
    let r = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(alloc_error),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RESULT),
    ]);
    field_addr(r, ARENA_STATE_REGISTER, state_offset, instructions);
    instructions.extend([
        abi::load_u64(dst, abi::stack_pointer(), RESULT),
        abi::load_u64(n, r, token(TOKEN_GRAPHEME_LEN)),
        abi::store_u64(n, dst, 0),
        abi::add_immediate(dst, dst, 8),
        abi::add_immediate(src, r, token(TOKEN_GRAPHEME)),
        abi::label(&l("copy")),
        abi::compare_immediate(n, "0"),
        abi::branch_eq(&l("copied")),
        abi::load_u8(b, src, 0),
        abi::store_u8(b, dst, 0),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(dst, dst, 1),
        abi::subtract_immediate(n, n, 1),
        abi::branch(&l("copy")),
        abi::label(&l("copied")),
        abi::store_u8(abi::ZERO, dst, 0),
    ]);
}

/// `RESULT` = a new `io::Key` built from the token.
fn emit_build_key(
    symbol: &str,
    state_offset: usize,
    alloc_error: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let token = |offset: usize| INPUT_STATE_TOKEN_OFFSET + offset;
    let field = |index: usize| KEY_FIELDS + 8 * index;
    let r = vregs.next();
    let v = vregs.next();
    let glen = vregs.next();
    let at = vregs.next();
    field_addr(r, ARENA_STATE_REGISTER, state_offset, instructions);
    instructions.extend([
        // key, event
        abi::load_u64(v, r, token(TOKEN_KEY)),
        abi::store_u64(v, abi::stack_pointer(), field(0)),
        abi::load_u64(v, r, token(TOKEN_EVENT)),
        abi::store_u64(v, abi::stack_pointer(), field(2)),
        // grapheme: the token's `{length, bytes}` is a `String` block once a NUL
        // follows the bytes (the region keeps a spare word after the record).
        abi::load_u64(glen, r, token(TOKEN_GRAPHEME_LEN)),
        abi::add_registers(at, r, glen),
        abi::store_u8(abi::ZERO, at, token(TOKEN_GRAPHEME)),
        abi::add_immediate(v, r, token(TOKEN_GRAPHEME_LEN)),
        abi::store_u64(v, abi::stack_pointer(), field(1)),
    ]);
    // shift, ctrl, alt, meta
    let mods = vregs.next();
    instructions.push(abi::load_u64(mods, r, token(TOKEN_MODS)));
    for (index, bit) in [(3, MOD_SHIFT), (4, MOD_CTRL), (5, MOD_ALT), (6, MOD_META)] {
        let b = vregs.next();
        let m = vregs.next();
        instructions.extend([
            abi::move_immediate(m, "Integer", &bit.to_string()),
            abi::and_registers(b, mods, m),
            abi::shift_right_immediate(b, b, bit.trailing_zeros() as u8),
            abi::store_u64(b, abi::stack_pointer(), field(index)),
        ]);
    }
    let result = vregs.next();
    emit_build_inlined_record_sized(
        symbol,
        "key",
        &ParameterType::named(super::KEY_TYPE_ID),
        TypeModel::builtin_records(),
        &(0..KEY_FIELD_COUNT).map(field).collect::<Vec<_>>(),
        &[None; KEY_FIELD_COUNT],
        &RecordBuildScratch {
            size: KEY_SCRATCH,
            result: KEY_SCRATCH + 8,
            cursor: KEY_SCRATCH + 16,
            block_size: KEY_SCRATCH + 24,
        },
        &MarshalRegs::fresh(vregs),
        result,
        alloc_error,
        instructions,
        relocations,
    )?;
    instructions.push(abi::store_u64(result, abi::stack_pointer(), RESULT));
    let _ = INPUT_STATE_LEN_OFFSET;
    Ok(())
}
