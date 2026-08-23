//! ALSA `audio` stream lifecycle code generation (open + close).

use super::gen_alsa_shared::*;
use super::gen_common::*;
use super::gen_os_seam::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::*;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) fn lower_open(
    symbol: &str,
    input: bool,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v15 = vregs.next();
    if device {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), DEVID_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(3), abi::stack_pointer(), BF_OFF),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), BF_OFF),
        ]);
    }
    // Zero the state and hw-params slots so the open-error cleanup can tell what
    // has actually been acquired (nothing to close/munmap before mmap and
    // snd_pcm_open run; no params object before hw_params_malloc).
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PARAMS_OFF),
    ]);
    emit_validate_open(
        symbol,
        SR_OFF,
        CH_OFF,
        BF_OFF,
        &invalid,
        &mut instructions,
        &mut vregs,
    );
    // bytesPerFrame, AudioHandle, mmap state.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::move_immediate(&v10, "Integer", "2"),
        abi::multiply_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &H_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::store_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        // Canonical plan-80 header: tag@0, kind (handle)@8, closed@16, STATE@24.
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_AUDIO),
        abi::store_u64(&v9, &v15, RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, &v15, RESOURCE_OFFSET_STATE),
        abi::move_immediate(&v9, "Integer", if input { KIND_INPUT } else { KIND_OUTPUT }),
        abi::store_u64(&v9, &v15, H_KIND),
        abi::store_u64(abi::ZERO, &v15, H_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SR_OFF),
        abi::store_u64(&v9, &v15, H_SAMPLE_RATE),
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::store_u64(&v9, &v15, H_CHANNELS),
        abi::load_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::store_u64(&v9, &v15, H_BYTES_PER_FRAME),
        abi::load_u64(&v9, abi::stack_pointer(), BF_OFF),
        abi::store_u64(&v9, &v15, H_BUFFER_FRAMES),
        // mmap one state page (ring unused on Linux; §3.2).
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::c_arg(1), "Integer", &STATE_PAGE.to_string()),
        abi::move_immediate(abi::c_arg(2), "Integer", "3"), // PROT_READ|WRITE
        abi::move_immediate(abi::c_arg(3), "Integer", "34"), // MAP_PRIVATE|MAP_ANONYMOUS (Linux)
        abi::bitwise_not(abi::c_arg(4), abi::ZERO),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "mmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::add_immediate(&v9, abi::return_register(), 1),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&dev_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::return_register(), &v15, H_STATE),
        abi::load_u64(&v15, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v15, S_XRUNS),
        abi::store_u64(abi::ZERO, &v15, S_CLOSED),
        abi::store_u64(abi::ZERO, &v15, S_OSOBJECT),
        abi::move_immediate(&v9, "Integer", &STATE_PAGE.to_string()),
        abi::store_u64(&v9, &v15, S_MAP_SIZE),
    ]);
    emit_dlopen(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &unavailable,
    )?;
    // Device name: the id string, or "default".
    if device {
        emit_device_cstring(DEVID_OFF, &mut instructions, symbol, &mut vregs);
    } else {
        emit_data_address(
            symbol,
            &v9,
            "_mfb_audio_alsa_default",
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::store_u64(&v9, abi::stack_pointer(), NAME_OFF));
    }
    // snd_pcm_open(&state->osobject, name, stream, 0)
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_pcm_open",
        &unavailable,
        false,
        |ins, _relocs| {
            ins.extend([
                abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
                abi::add_immediate(abi::return_register(), &v9, S_OSOBJECT),
                abi::load_u64(abi::c_arg(1), abi::stack_pointer(), NAME_OFF),
                abi::move_immediate(
                    abi::c_arg(2),
                    "Integer",
                    if input {
                        STREAM_CAPTURE
                    } else {
                        STREAM_PLAYBACK
                    },
                ),
                abi::move_immediate(abi::c_arg(3), "Integer", "0"),
            ]);
        },
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
    ]);
    emit_configure_hw_params(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &unavailable,
        &dev_fail,
    )?;
    // snd_pcm_prepare(pcm)
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_pcm_prepare",
        &unavailable,
        false,
        |ins, _relocs| {
            ins.extend([
                abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
                abi::load_u64(abi::return_register(), &v9, S_OSOBJECT),
            ]);
        },
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::load_u64(&v15, abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v15, S_STARTED),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // Both open-error exits release everything acquired so far. `unavailable` is
    // reached from the dlopen and every dlsym miss, which on a host without
    // libasound is *every* open — so before bug-319 each `audio::openOutput` on
    // such a host leaked the 16 KiB state page (and, with a partial/wrong-ABI
    // libasound where snd_pcm_open resolves but a later symbol does not, the
    // open PCM handle and the hw-params object too). `dev_fail` had this cleanup
    // since bug-180 but never freed the hw-params object.
    instructions.push(abi::label(&unavailable));
    emit_open_cleanup(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "unavail",
    )?;
    emit_fail(
        symbol,
        "ErrAudioUnavailable",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&dev_fail));
    emit_open_cleanup(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "dev",
    )?;
    emit_fail(
        symbol,
        "ErrAudioDevice",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME))
}

/// close(stream): drain (playback) or drop (capture), snd_pcm_close, munmap.
pub(crate) fn lower_close(
    symbol: &str,
    input: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let already = format!("{symbol}_already");
    let unavailable = format!("{symbol}_unavailable");
    let done = format!("{symbol}_done");
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&already),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
    ]);
    emit_dlopen(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &unavailable,
    )?;
    // snd_pcm_drain (playback) / snd_pcm_drop (capture); failure is reported but
    // must not skip close.
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        if input {
            "snd_pcm_drop"
        } else {
            "snd_pcm_drain"
        },
        &unavailable,
        false,
        |ins, _relocs| {
            ins.extend([
                abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
            ]);
        },
    )?;
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_pcm_close",
        &unavailable,
        false,
        |ins, _relocs| {
            ins.extend([
                abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
            ]);
        },
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v10, H_CLOSED),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::c_arg(1), abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_external_call(
        "munmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&unavailable),
    ]);
    emit_fail(
        symbol,
        "ErrAudioUnavailable",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME))
}

#[cfg(test)]
mod open_error_cleanup_tests {
    //! bug-319 regression guards. These paths are Linux-only and need a real
    //! libasound to execute, so the assertions pin the emitted cleanup instead:
    //! the two open-error exits must dispose of everything the open acquired.
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;
    use crate::codegen::engine::tests::{has_label, TestPlatform};

    fn open_ins(device: bool) -> Vec<CodeInstruction> {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (ins, _r, _s) =
            lower_open("o", false, device, &imports, &TestPlatform).expect("lower open");
        ins
    }

    /// Whether the window between labels `start` and `end` calls `name`.
    ///
    /// `emit_alsa_call` materialises the symbol's data address with an `adrp`
    /// carrying `_mfb_audio_alsa_sym_<name>`, and `emit_external_call` emits a `bl
    /// _<name>`, so both are visible positionally. A whole-function scan cannot
    /// substitute: the success path already closes and frees, so only a windowed
    /// check proves the *error exits* clean up.
    fn calls_between(ins: &[CodeInstruction], start: &str, end: &str, name: &str) -> bool {
        let at = |label: &str| {
            ins.iter()
                .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(label))
                .unwrap_or_else(|| panic!("missing label {label}"))
        };
        let (from, to) = (at(start), at(end));
        assert!(from < to, "expected {start} to precede {end}");
        let dl = sym_data_symbol(name);
        let libc = format!("_{name}");
        ins[from..to].iter().any(|i| {
            i.get("symbol").as_deref() == Some(&dl) || i.get("target").as_deref() == Some(&libc)
        })
    }

    // The `unavailable` exit is reached from the dlopen and from every dlsym
    // miss — i.e. from *every* open on a host without libasound. It used to
    // `emit_fail` with no cleanup at all, leaking the 16 KiB state page each
    // time (and the PCM handle on a partial libasound).
    #[test]
    fn unavailable_exit_releases_the_state_page_and_pcm() {
        for device in [false, true] {
            let ins = open_ins(device);
            for name in ["snd_pcm_hw_params_free", "snd_pcm_close", "munmap"] {
                assert!(
                    calls_between(&ins, "o_unavailable", "o_dev_fail", name),
                    "the unavailable exit must call {name} (device={device})"
                );
            }
            // Each disposal is guarded on its own slot, so an exit reached
            // before that resource existed skips it rather than acting on NULL.
            for label in [
                "o_unavail_params_done",
                "o_unavail_munmap",
                "o_unavail_cleanup_done",
            ] {
                assert!(
                    has_label(&ins, label),
                    "missing guard label {label} (device={device})"
                );
            }
        }
    }

    // dev_fail had the close+munmap since bug-180 but never freed the hw-params
    // object, so a device that could not honour the requested rate/channels
    // leaked one heap block per failed open.
    #[test]
    fn dev_fail_exit_frees_the_hw_params_object() {
        let ins = open_ins(false);
        assert!(
            calls_between(&ins, "o_dev_fail", "o_alloc_fail", "snd_pcm_hw_params_free"),
            "dev_fail must free the hw-params object, not just close and munmap"
        );
        for name in ["snd_pcm_close", "munmap"] {
            assert!(
                calls_between(&ins, "o_dev_fail", "o_alloc_fail", name),
                "dev_fail must still {name} (bug-180)"
            );
        }
    }
}
