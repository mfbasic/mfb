//! The built-in `audio` package (raw interleaved `s16le` PCM) on the clean-room
//! registry.
//!
//! `audio` enumerates audio devices, opens capture (`AudioInput`) and playback
//! (`AudioOutput`) streams, and moves whole frames of `s16le` PCM through them.
//! Direction is part of the type: `read` is defined only over `AudioInput`,
//! `write` only over `AudioOutput`, so a swapped stream is a compile error caught
//! by overload resolution — never a runtime check. The two opaque handles are
//! `add_resource`-registered resources whose strict-mode base-resource matching
//! (`resource_base_eq`) rejects the wrong direction with no custom resolver
//! (the `tls`/`process`/`datetime` multi-overload idiom).
//!
//! `audio` is a **native OS-seam** package: the device I/O members carry a
//! per-platform runtime-helper lowering, migrated to crypto/io's clean-room
//! `Body::abi_function` shape. The whole per-backend emission (macOS Core Audio
//! `AudioQueue`, Linux ALSA over a `dlopen`'d `libasound.so.2`, Windows WASAPI over
//! COM) lives in the `gen_*` backend modules; each device-I/O member owns its
//! `abi_function` body ([`Body::abi_function_aliased`]) in its own `func_*.rs`
//! (`lower_<name>`), branching on `platform.family()` to the shared backend
//! emitters (the family routing for the multi-member `open`/`query` cases is the
//! shared [`gen_shared::dispatch_open`]/[`gen_shared::dispatch_query`]). A member
//! plus its code-form aliases (`openInputDevice`/`openOutputDevice`/`readTimeout`/
//! `pollTimeout`/`closeInput`/`closeOutput`) route to that same member body through
//! `registry::abi_function_lower`, distinguished inside `lower_<name>` off
//! `AbiCtx::call`. The IR-level overload rewrites live in [`runtime_overload_name`]
//! (`audio.read` → `audio.readTimeout`, …), keyed on argument shape.
//!
//! The `render`/`play` members are pure MFBASIC: each `func_*.rs` descriptor
//! carries its `__audio_render`/`__audio_play`/`__audio_playTracks` body as
//! `Body::mfb`, rewritten through the generic `registry::rewrite_target`; the
//! PRIVATE `__audio_*` sequencer helpers (one `helper_*.rs` per FUNC,
//! `add_helper` — private-only, the bodies CALL the migrated `math` registry
//! package by name) render before them in the `get_mfb` assembly. The three
//! value records (`AudioDevice`, `AudioEnvelope`, `AudioNote`) and the internal
//! `__audio_MmlEvent` event record are registered via `add_record` and rendered
//! into the injected source by `get_mfb`.

use crate::codegen::registry::{
    Body, DefaultValue, Parameter, RecordProp, Registry, RegistryPackage, RegistryRecord,
    RegistryResource,
};
use crate::types::ParameterType;

pub(crate) mod gen_alsa_devices;
pub(crate) mod gen_alsa_io;
pub(crate) mod gen_alsa_shared;
pub(crate) mod gen_alsa_stream;
pub(crate) mod gen_common;
pub(crate) mod gen_macos_callbacks;
pub(crate) mod gen_macos_devices;
pub(crate) mod gen_macos_io;
pub(crate) mod gen_macos_shared;
pub(crate) mod gen_macos_stream;
pub(crate) mod gen_shared;
pub(crate) mod gen_windows;

mod func_available;
mod func_close;
mod func_devices;
mod func_open_input;
mod func_open_output;
mod func_play;
mod func_poll;
mod func_read;
mod func_render;
mod func_write;
mod func_xruns;

mod helper_append_s16_le;
mod helper_clamp_s16;
mod helper_mml_apply_legato;
mod helper_mml_clamp_fade;
mod helper_mml_encode;
mod helper_mml_expand;
mod helper_mml_frames;
mod helper_mml_has_open;
mod helper_mml_is_digit;
mod helper_mml_lcg;
mod helper_mml_mix;
mod helper_mml_note;
mod helper_mml_note_semitone;
mod helper_mml_parse;
mod helper_mml_parse_uint;
mod helper_mml_render_samples;
mod helper_mml_req_int;
mod helper_mml_rest;
mod helper_mml_synth;
mod helper_mml_tokens;
mod helper_mml_trailing_dots;
mod helper_mml_wave_code;
mod helper_play_samples;

/// The `AudioInput` capture handle's bare type name (the `RegistryResource` name
/// and the `type` half of its qualified id).
pub(crate) const AUDIO_INPUT_TYPE: &str = "AudioInput";
/// The `AudioOutput` playback handle's bare type name.
pub(crate) const AUDIO_OUTPUT_TYPE: &str = "AudioOutput";

/// The `AudioInput` resource's **package-qualified type identity**
/// (`audio.AudioInput`) — plan-97 / bug-441. The string every `RES` binding,
/// parameter, and return of a capture stream carries.
pub(crate) const AUDIO_INPUT_TYPE_ID: &str = "audio.AudioInput";
/// The `AudioOutput` resource's package-qualified type identity (`audio.AudioOutput`).
pub(crate) const AUDIO_OUTPUT_TYPE_ID: &str = "audio.AudioOutput";

/// The `AudioDevice` value record's type name (a plain read-only record obtained
/// only from `audio::devices()`).
pub(crate) const AUDIO_DEVICE_TYPE: &str = "AudioDevice";
/// Its package-qualified identity — what a consumer must write, and what the
/// resolver seeds, so a bare `AS AudioDevice` is refused (bug-484).
pub(crate) const AUDIO_DEVICE_TYPE_ID: &str = "audio.AudioDevice";
/// The `AudioEnvelope`/`AudioNote` value records the user constructs and passes to
/// `audio::render` (rendered into the injected source by `get_mfb` so the source
/// `render` body operates on them).
pub(crate) const AUDIO_ENVELOPE_TYPE: &str = "AudioEnvelope";
pub(crate) const AUDIO_NOTE_TYPE: &str = "AudioNote";

/// The per-direction internal close bodies. `audio::close` stays the single
/// user-facing name over both handle types; IR-time overload split routes each
/// operand to the matching internal target (a code-form `os_alias`), and
/// scope-drop reaches them directly as the resources' registered close ops. Not
/// user-callable.
pub(crate) const CLOSE_INPUT: &str = "audio.closeInput";
pub(crate) const CLOSE_OUTPUT: &str = "audio.closeOutput";

const MODULE_INTRO: &str = r#"Raw interleaved `s16le` PCM capture and playback"#;
const MODULE_DESC: &str = r#"The `audio` package moves raw interleaved signed 16-bit little-endian PCM
(`s16le`) through the operating system's audio hardware. It enumerates devices,
opens a capture or playback stream, moves whole frames of PCM through it, and
closes it. There is no audio file, container, codec, mixing, resampling, or
channel-conversion API at any layer — the only format is `s16le`, and one frame
is `channels * 2` bytes.


Direction is part of the type. `audio::openInput` returns an `AudioInput` and
`audio::openOutput` returns an `AudioOutput`; `audio::read` is defined only over
`AudioInput` and `audio::write` only over `AudioOutput`, so passing the wrong
stream is a compile error rather than a runtime one. This mirrors the hardware:
no operating system in scope has a duplex stream handle, so full duplex means
opening one stream of each direction and driving both from a single loop with
`audio::poll`, `audio::available`, and the timed form of `audio::read`.


Both stream types are move-only, non-sendable resource handles: neither can
cross a thread boundary. Each closes itself when its
binding goes out of scope, or explicitly with `audio::close`. `audio::AudioDevice` is a
plain read-only record obtained only from `audio::devices()`. The `render` and
`play` members are pure synthesis: `render` turns one `audio::AudioNote` into `s16le`
PCM, and `play` parses MML music text and writes it to an open `AudioOutput`.


macOS drives Core Audio's `AudioQueue`; Linux drives ALSA's blocking PCM API
through a `libasound.so.2` resolved at runtime with `dlopen` — so a binary that
imports `audio` still starts on a Linux host without alsa-lib, and every
`audio::` call there raises `ErrAudioUnavailable`. A program that does not
`IMPORT audio` gains no audio symbol and no dynamic-library dependency."#;

/// A required parameter with optional keyword aliases.
pub(crate) fn param(
    name: &'static str,
    desc: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
) -> Parameter {
    Parameter {
        name,
        desc,
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

/// An optional trailing `timeoutMs AS Integer` (the timed `read`/`poll` forms).
/// `DefaultValue::Optional` widens the arity range WITHOUT default-padding: the
/// timed form is selected at codegen (`builder_values` → `audio.readTimeout` /
/// `audio.pollTimeout`), and the emitter branches on the runtime-call name (the
/// `process::poll` idiom).
pub(crate) fn timeout_ms(desc: &'static str) -> Parameter {
    Parameter {
        name: "timeoutMs",
        desc,
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::Optional,
    }
}

/// Build a device-I/O member's `abi_function` body (crypto/io's clean-room shape):
/// its own per-member lowering `lower` (which lives in the member's `func_*.rs` and
/// branches on `platform.family()` to the shared backend emitters), plus the
/// code-form `os_aliases` this overload declares (the IR-level overload-split forms
/// `openInputDevice`/`readTimeout`/`closeInput`/… routed to this same body by
/// `abi_function_lower`, and distinguished inside `lower` off
/// [`AbiCtx::call`](crate::codegen::registry::AbiCtx)).
pub(crate) fn native_body(
    lower: crate::codegen::registry::AbiFunction,
    os_aliases: &'static [&'static str],
) -> Body {
    Body::abi_function_aliased(lower, os_aliases)
}

/// The internal runtime-helper name a surface `audio::` call rewrites to during IR
/// lowering, when the overload needs its own body — the named-device opens, the
/// timed `read`/`poll`, and the per-direction `close`. Returns `None` for the calls
/// that keep their surface name (`devices`, default-device `open*`, untimed `read`/
/// `poll`, `write`, `available`, `xruns`), and for every non-`audio` call.
///
/// These rewrites are done at IR level (not in the code layer via `builder_values`)
/// so the NIR carries the exact runtime-call name: the derived runtime-spec catalog,
/// the required-helper emission, and the per-target import planning all key on the
/// NIR call, so an IR-level rewrite keeps them — and the native `.ncode` — unchanged.
/// This is the `tls::close`→`tls.closeListener` idiom (which likewise rewrites at IR
/// level), extended to audio's five overload-split cases; the `os_aliases` on each
/// `func_*.rs` descriptor still derive the specs and route the emitted symbol to the
/// shared `lower_audio_os_seam` body (`registry::abi_function_lower`). The result is a runtime helper, not a
/// source companion, so IR lowering must NOT internalize it. `render`/`play` are
/// source members handled by the generic `registry::rewrite_target` and are excluded
/// here (they are not native runtime calls).
pub(crate) fn runtime_overload_name(qualified: &str, arg_types: &[String]) -> Option<&'static str> {
    let first = arg_types.first().map(String::as_str);
    match qualified {
        "audio.openInput" if first == Some(AUDIO_DEVICE_TYPE) => Some("audio.openInputDevice"),
        "audio.openOutput" if first == Some(AUDIO_DEVICE_TYPE) => Some("audio.openOutputDevice"),
        "audio.read" if arg_types.len() == 3 => Some("audio.readTimeout"),
        "audio.poll" if arg_types.len() == 2 => Some("audio.pollTimeout"),
        "audio.close" if first == Some(AUDIO_INPUT_TYPE_ID) => Some(CLOSE_INPUT),
        "audio.close" if first == Some(AUDIO_OUTPUT_TYPE_ID) => Some(CLOSE_OUTPUT),
        _ => None,
    }
}

/// Register the `audio` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("audio", MODULE_INTRO, MODULE_DESC);

    // The source companion (`render` tone synth + `play` MML sequencer) IMPORTs
    // these; `IMPORT audio` is the self-reference the companion's `audio::write`
    // /`audio::AudioOutput` need. Rendered as leading `IMPORT` lines by `get_mfb`.
    pkg.add_imports(vec!["audio", "collections", "math", "bits", "strings"]);

    // The three value records, rendered into the injected source by `get_mfb` in
    // registration order (`AudioEnvelope` before `AudioNote`, which references it).
    pkg.add_record(RegistryRecord {
        name: AUDIO_DEVICE_TYPE,
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "id",
                ty: ParameterType::String,
                description: "Opaque device id (a Core Audio UID on macOS, an ALSA PCM hint name on Linux). Pass it to `openInput`/`openOutput`; never construct it.",
            },
            RecordProp {
                name: "name",
                ty: ParameterType::String,
                description: "Human-readable device name.",
            },
            RecordProp {
                name: "canInput",
                ty: ParameterType::Boolean,
                description: "Whether the device can capture. On Linux every hint reports `TRUE`.",
            },
            RecordProp {
                name: "canOutput",
                ty: ParameterType::Boolean,
                description: "Whether the device can play back. On Linux every hint reports `TRUE`.",
            },
            RecordProp {
                name: "isDefaultInput",
                ty: ParameterType::Boolean,
                description: "Whether this is the system default input. Always `FALSE` on Linux.",
            },
            RecordProp {
                name: "isDefaultOutput",
                ty: ParameterType::Boolean,
                description: "Whether this is the system default output. Always `FALSE` on Linux.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: AUDIO_ENVELOPE_TYPE,
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "attackFrames",
                ty: ParameterType::Integer,
                description: "Frame count of the linear ramp from silence to full amplitude.",
            },
            RecordProp {
                name: "decayFrames",
                ty: ParameterType::Integer,
                description: "Frame count of the linear ramp from full amplitude down to `sustainLevel`.",
            },
            RecordProp {
                name: "holdFrames",
                ty: ParameterType::Integer,
                description: "Informational sustain length in frames; the actual sustain fills the note's remaining frames.",
            },
            RecordProp {
                name: "releaseFrames",
                ty: ParameterType::Integer,
                description: "Frame count of the linear ramp from `sustainLevel` back to silence.",
            },
            RecordProp {
                name: "sustainLevel",
                ty: ParameterType::Integer,
                description: "Held amplitude in s16 sample units (0..32767) during the sustain phase.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: AUDIO_NOTE_TYPE,
        export: true,
        description: "",
        props: vec![
            RecordProp {
                name: "frequencyHz",
                ty: ParameterType::Float,
                description: "Pitch of the sine tone in hertz.",
            },
            RecordProp {
                name: "noteFrames",
                ty: ParameterType::Integer,
                description: "Total length of the note in sample frames.",
            },
            RecordProp {
                name: "envelope",
                ty: ParameterType::named(AUDIO_ENVELOPE_TYPE),
                description: "The `audio::AudioEnvelope` shaping the note's amplitude over time.",
            },
            RecordProp {
                name: "gainOverall",
                ty: ParameterType::Float,
                description: "Overall gain applied to the note, in the range 0..1.",
            },
        ],
    });
    // The MML sequencer's internal event record: a single synthesized event. A
    // rest has freq <= 0. `soundFrames` < `totalFrames` leaves a trailing silence
    // (staccato); fadeIn/fadeOut are short click-guard ramps in frames (0 at the
    // interior joins of a legato run). Not exported: only the `__audio_mml*`
    // helper bodies touch it.
    pkg.add_record(RegistryRecord {
        name: "__audio_MmlEvent",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "freq",
                ty: ParameterType::Float,
                description: "",
            },
            RecordProp {
                name: "totalFrames",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "soundFrames",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "gain",
                ty: ParameterType::Float,
                description: "",
            },
            RecordProp {
                name: "wave",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "fadeIn",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "fadeOut",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    // The two opaque, move-only resource handles. Their strict base-resource
    // matching rejects a cross-direction argument (`read(AudioOutput)`); their
    // registered close ops are the per-direction internal bodies (also the
    // `close` member's code-form `os_aliases`).
    pkg.add_resource(RegistryResource {
        name: AUDIO_INPUT_TYPE,
        export: true,
        description: "An opaque, move-only PCM capture stream from `audio::openInput`, closed \
                      automatically when its binding goes out of scope.",
        close_function: CLOSE_INPUT,
        // A capture stream is driven from its owning thread (blocking read / OS
        // callback ring); not thread-sendable in v1 (plan-33-A §4).
        sendable: false,
        // Audio records DO carry live tail state (backend handles at 32/40/48),
        // but bug-464 deliberately left these unaudited and out of scope: an
        // audio handle's backend callbacks are bound to a device thread, which
        // is a separate question from whether the record copies. Declaring the
        // slots here would imply an audit that has not happened -- opting one in
        // means doing that audit and filling this list, not flipping the bit.
        live_slots: &[],
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });
    pkg.add_resource(RegistryResource {
        name: AUDIO_OUTPUT_TYPE,
        export: true,
        description: "An opaque, move-only PCM playback stream from `audio::openOutput`, closed \
                      automatically when its binding goes out of scope.",
        close_function: CLOSE_OUTPUT,
        // A playback stream blocks on write from its owning thread; not
        // thread-sendable in v1 (plan-33-A §4).
        sendable: false,
        // As the capture stream above: live tail state, deliberately unaudited
        // and out of bug-464's scope.
        live_slots: &[],
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    func_devices::register(&mut pkg);
    func_open_input::register(&mut pkg);
    func_open_output::register(&mut pkg);
    func_read::register(&mut pkg);
    func_write::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_available::register(&mut pkg);
    func_xruns::register(&mut pkg);
    func_close::register(&mut pkg);
    func_render::register(&mut pkg);
    func_play::register(&mut pkg);

    // The injected `__audio_*` bodies — the `render` tone synth and the `play`
    // MML sequencer (including the `__audio_render`/`__audio_play`/
    // `__audio_playTracks` rewrite targets). Each lives in its own `helper_*.rs`
    // and registers via `add_helper`; they render (in this order) in the helper
    // section of the assembled source. Order is preserved from the old single
    // `package.mfb` blob. The value records are registered above (add_record).
    //
    // Shared s16 primitives + the tone renderer.
    helper_clamp_s16::register(&mut pkg);
    helper_append_s16_le::register(&mut pkg);
    // audio::play — a small MML (Music Macro Language) sequencer. A track is a
    // space-separated string of tokens; every token is separated by a single
    // space. Tokens: notes A..G (with a trailing + / - accidental, an inline
    // length, and trailing dots), R (rest of the current length), P<n> (a pause
    // of length n), O0..O6 / < / > (octave), L1..L64 (default length),
    // T32..T255 (tempo), V0..V10 (volume), `I <name>` (instrument: square/
    // triangle/sine/saw/noise), ( .. ) legato, [ .. ] staccato, and { .. }<count>
    // repeat (may nest). Each track is isolated — no shared tempo/length/octave/
    // etc. A play() call pre-renders every track to PCM, mixes them, and writes
    // the result. Invalid MML raises ErrInvalidArgument (7-705-0002).
    // Waveform codes: 0 sine, 1 square, 2 triangle, 3 saw, 4 noise. All
    // rendering is mono s16le at 48 kHz (one frame = 2 bytes), matching render().
    helper_mml_is_digit::register(&mut pkg);
    helper_mml_parse_uint::register(&mut pkg);
    helper_mml_note_semitone::register(&mut pkg);
    helper_mml_wave_code::register(&mut pkg);
    helper_mml_trailing_dots::register(&mut pkg);
    helper_mml_frames::register(&mut pkg);
    helper_mml_req_int::register(&mut pkg);
    helper_mml_rest::register(&mut pkg);
    helper_mml_note::register(&mut pkg);
    helper_mml_has_open::register(&mut pkg);
    helper_mml_expand::register(&mut pkg);
    helper_mml_apply_legato::register(&mut pkg);
    helper_mml_tokens::register(&mut pkg);
    helper_mml_parse::register(&mut pkg);
    helper_mml_clamp_fade::register(&mut pkg);
    helper_mml_lcg::register(&mut pkg);
    helper_mml_synth::register(&mut pkg);
    helper_mml_render_samples::register(&mut pkg);
    helper_mml_mix::register(&mut pkg);
    helper_mml_encode::register(&mut pkg);
    helper_play_samples::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    /// plan-111-F: `rewrite_target` selects on TYPES, so the test's spellings
    /// are parsed at this one helper rather than threaded onward as names.
    fn types(items: &[&str]) -> Vec<crate::types::ParameterType> {
        items
            .iter()
            .map(|s| crate::types::ParameterType::declared(s))
            .collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn audio_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("audio").expect("audio package");
        assert_eq!(pkg.functions().len(), 11);
        // The three public value records + the internal `__audio_MmlEvent`.
        assert_eq!(pkg.records().len(), 4);
        assert_eq!(pkg.resources().len(), 2);
        // The three value records are visible to the generic type query.
        assert!(registry().is_builtin_type("AudioDevice"));
        assert!(registry().is_builtin_type("AudioEnvelope"));
        assert!(registry().is_builtin_type("AudioNote"));
        // The opaque handles are semantic-only resources (not value types).
        assert!(!registry().is_builtin_type("AudioInput"));
        assert_eq!(
            registry().qualified_builtin_type("audio.AudioInput"),
            Some("audio.AudioInput".to_string())
        );
    }

    #[test]
    fn direction_is_part_of_the_type() {
        // `read` is input-only, `write` output-only: strict base-resource matching
        // rejects the wrong direction with no custom resolver.
        assert_eq!(
            registry::resolve_call(
                "audio.read",
                &strings(&["audio.AudioInput", "Integer"]),
                true
            ),
            Some("List OF Byte".to_string())
        );
        assert_eq!(
            registry::resolve_call(
                "audio.read",
                &strings(&["audio.AudioOutput", "Integer"]),
                true
            ),
            None
        );
        assert_eq!(
            registry::resolve_call(
                "audio.write",
                &strings(&["audio.AudioOutput", "List OF Byte"]),
                true
            ),
            Some("Nothing".to_string())
        );
        assert_eq!(
            registry::resolve_call(
                "audio.write",
                &strings(&["audio.AudioInput", "List OF Byte"]),
                true
            ),
            None
        );
    }

    #[test]
    fn dual_direction_members_accept_either_handle() {
        for t in ["audio.AudioInput", "audio.AudioOutput"] {
            assert_eq!(
                registry::resolve_call("audio.poll", &strings(&[t]), true),
                Some("Boolean".to_string())
            );
            assert_eq!(
                registry::resolve_call("audio.available", &strings(&[t]), true),
                Some("Integer".to_string())
            );
            assert_eq!(
                registry::resolve_call("audio.xruns", &strings(&[t]), true),
                Some("Integer".to_string())
            );
            assert_eq!(
                registry::resolve_call("audio.close", &strings(&[t]), true),
                Some("Nothing".to_string())
            );
        }
    }

    #[test]
    fn open_and_timed_overloads_resolve() {
        // Device-vs-default open overloads.
        assert_eq!(
            registry::resolve_call(
                "audio.openInput",
                &strings(&["Integer", "Integer", "Integer"]),
                true
            ),
            Some("audio.AudioInput".to_string())
        );
        assert_eq!(
            registry::resolve_call(
                "audio.openOutput",
                &strings(&["AudioDevice", "Integer", "Integer", "Integer"]),
                true
            ),
            Some("audio.AudioOutput".to_string())
        );
        // Untimed and timed read/poll arities.
        assert_eq!(registry().arity("audio.read"), Some((2, 3)));
        assert_eq!(registry().arity("audio.poll"), Some((1, 2)));
        // Native members carry no rewrite target (they lower through Body::abi_function).
        assert_eq!(
            registry::rewrite_target("audio.read", &types(&["audio.AudioInput", "Integer"])),
            None
        );
    }

    #[test]
    fn source_members_rewrite_to_their_companion_bodies() {
        assert_eq!(
            registry::rewrite_target("audio.render", &types(&["AudioNote"])),
            Some("__audio_render")
        );
        assert_eq!(
            registry::rewrite_target("audio.play", &types(&["audio.AudioOutput", "String"])),
            Some("__audio_play")
        );
        assert_eq!(
            registry::rewrite_target(
                "audio.play",
                &types(&["audio.AudioOutput", "List OF String"])
            ),
            Some("__audio_playTracks")
        );
    }

    #[test]
    fn close_ops_are_the_per_direction_bodies() {
        assert_eq!(
            crate::codegen::builtins::resource_close_function(&crate::types::ParameterType::named(
                super::AUDIO_INPUT_TYPE_ID
            )),
            Some(super::CLOSE_INPUT)
        );
        assert_eq!(
            crate::codegen::builtins::resource_close_function(&crate::types::ParameterType::named(
                super::AUDIO_OUTPUT_TYPE_ID
            )),
            Some(super::CLOSE_OUTPUT)
        );
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("audio")
            .expect("audio")
            .get_mfb();
        assert!(source.contains("EXPORT TYPE AudioNote"));
        assert!(source.contains("FUNC __audio_render"));
        assert!(source.contains("SUB __audio_playTracks"));
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-audio>"),
            "builtins/audio.mfb",
            &source,
        )
        .expect("reassembled audio source parses");
    }
}
