//! Front-end definitions for the built-in `audio` package: raw interleaved
//! `s16le` PCM capture and playback (plan-33-A).
//!
//! Two move-only, non-sendable resources carry direction statically:
//! `AudioInput` (capture) and `AudioOutput` (playback). `audio::read` is defined
//! only over `AudioInput`, `audio::write` only over `AudioOutput`, so a swapped
//! stream is a compile error caught by overload resolution — never a runtime
//! check. `AudioDevice` is a plain read-only record obtained only from
//! `audio::devices()`.
//!
//! `tls` is the precedent: two resource types, one user-facing `close`, and two
//! internal close bodies dispatched statically by `resource_close_function`
//! (`src/builtins/tls.rs:45`). Here the overloads that differ by *body* while no
//! user error is reachable — the device-open forms, the timed `read`/`poll`
//! forms, and per-direction `close` — are rewritten in IR lowering to their own
//! internal call names (`implementation_name`), so each maps to a distinct
//! runtime-helper symbol. `spec_for_call` is first-match on the call string, so
//! no two internal names collide.

use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    BuiltinType, DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType, TypeKind,
};

pub(crate) const AUDIO_INPUT_TYPE: &str = "AudioInput";
pub(crate) const AUDIO_OUTPUT_TYPE: &str = "AudioOutput";
pub(crate) const AUDIO_DEVICE_TYPE: &str = "AudioDevice";
/// Value records the user constructs and passes to `audio::render`. Registered
/// natively (fields below) so they are constructible, and defined in the source
/// companion (`audio_render.mfb`) so the source `render` operates on them — the
/// two field lists must match (the `vector::` value-record pattern).
pub(crate) const AUDIO_ENVELOPE_TYPE: &str = "AudioEnvelope";
pub(crate) const AUDIO_NOTE_TYPE: &str = "AudioNote";

const DEVICES: &str = "audio.devices";
const OPEN_INPUT: &str = "audio.openInput";
const OPEN_OUTPUT: &str = "audio.openOutput";
const READ: &str = "audio.read";
const WRITE: &str = "audio.write";
const POLL: &str = "audio.poll";
const AVAILABLE: &str = "audio.available";
const XRUNS: &str = "audio.xruns";
const CLOSE: &str = "audio.close";
/// Source-companion synthesis helpers, defined in `audio_render.mfb` (like
/// `csv::parse` → `__csv_parse`). `render` turns one `AudioNote` into PCM;
/// `play` parses MML text and writes it to an open `AudioOutput`. `play` is
/// overloaded by its second
/// argument — a single `String` track or a `List OF String` of tracks — onto two
/// distinct source bodies, so it dispatches through `source_implementation_name`
/// on the argument types (the `vector::` pattern).
const RENDER: &str = "audio.render";
const INTERNAL_RENDER: &str = "__audio_render";
const PLAY: &str = "audio.play";
const INTERNAL_PLAY: &str = "__audio_play";
const INTERNAL_PLAY_TRACKS: &str = "__audio_playTracks";

/// Internal call names produced by `implementation_name` during IR lowering.
/// They never appear as a source callee, so `resolve_call` does not accept them;
/// they each own a distinct runtime-helper symbol.
const OPEN_INPUT_DEVICE: &str = "audio.openInputDevice";
const OPEN_OUTPUT_DEVICE: &str = "audio.openOutputDevice";
const READ_TIMEOUT: &str = "audio.readTimeout";
const POLL_TIMEOUT: &str = "audio.pollTimeout";
/// The per-direction close bodies. `audio::close` stays the single user-facing
/// name over both handle types; IR lowering routes each operand to the matching
/// internal target, and scope-drop reaches them directly via
/// `resource_close_function`. Not user-callable.
pub(crate) const CLOSE_INPUT: &str = "audio.closeInput";
pub(crate) const CLOSE_OUTPUT: &str = "audio.closeOutput";

// plan-72-C: `AUDIO` is the descriptor authority for the 11 user-facing calls.
// The lowered-only internal names (device opens, timed read/poll, per-direction
// close) are NOT descriptor functions — they are IR-lowering artifacts that carry
// a return type but no user-facing membership/arity; `call_return_type_name` and
// `arity` fall back to a small map for them. Each function's return is fixed;
// argument VALIDATION (dual-direction overloads, input-only `read`) is
// argument-dependent and lives on `AudioResolver`. `implementation_name` (typed,
// `&'static`) stays static. 5 builtin types (2 opaque handles, 3 records).
const fn req(name: &'static str, ty: &'static str) -> Parameter {
    Parameter::required(name, ty)
}
const fn reqa(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}
// An optional trailing parameter (`timeoutMs`). The `Fill` is inert (audio has no
// default padding); it exists so `DefaultResolver::arity` derives the `..=` bound.
const fn opt(name: &'static str, ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Fill {
            type_name: ty,
            expr: "",
        },
    }
}
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}
const fn af(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Custom,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

// The two device-open overloads (params identical for input/output; only the
// return type differs), whose positions disagree (`call_param_name_overloads`).
const OPEN_OV0: &[Parameter] = &[
    req("sampleRate", "Integer"),
    req("channels", "Integer"),
    req("bufferFrames", "Integer"),
];
const OPEN_OV1: &[Parameter] = &[
    req("device", AUDIO_DEVICE_TYPE),
    req("sampleRate", "Integer"),
    req("channels", "Integer"),
    req("bufferFrames", "Integer"),
];

const AUDIO_FUNCTIONS: &[BuiltinFunction] = &[
    af(DEVICES, "devices", &[ov(&[], "List OF AudioDevice")]),
    af(
        OPEN_INPUT,
        "openInput",
        &[
            ov(OPEN_OV0, AUDIO_INPUT_TYPE),
            ov(OPEN_OV1, AUDIO_INPUT_TYPE),
        ],
    ),
    af(
        OPEN_OUTPUT,
        "openOutput",
        &[
            ov(OPEN_OV0, AUDIO_OUTPUT_TYPE),
            ov(OPEN_OV1, AUDIO_OUTPUT_TYPE),
        ],
    ),
    af(
        READ,
        "read",
        &[ov(
            &[
                req("input", AUDIO_INPUT_TYPE),
                req("frames", "Integer"),
                opt("timeoutMs", "Integer"),
            ],
            "List OF Byte",
        )],
    ),
    af(
        WRITE,
        "write",
        &[ov(
            &[
                req("output", AUDIO_OUTPUT_TYPE),
                req("bytes", "List OF Byte"),
            ],
            "Nothing",
        )],
    ),
    af(
        POLL,
        "poll",
        &[ov(
            &[req("stream", AUDIO_INPUT_TYPE), opt("timeoutMs", "Integer")],
            "Boolean",
        )],
    ),
    af(
        AVAILABLE,
        "available",
        &[ov(&[req("stream", AUDIO_INPUT_TYPE)], "Integer")],
    ),
    af(
        XRUNS,
        "xruns",
        &[ov(&[req("stream", AUDIO_INPUT_TYPE)], "Integer")],
    ),
    af(
        CLOSE,
        "close",
        &[ov(&[req("stream", AUDIO_INPUT_TYPE)], "Nothing")],
    ),
    af(
        RENDER,
        "render",
        &[ov(&[req("note", AUDIO_NOTE_TYPE)], "List OF Byte")],
    ),
    af(
        PLAY,
        "play",
        &[ov(
            &[
                req("output", AUDIO_OUTPUT_TYPE),
                reqa("mml", &["tracks"], "String"),
            ],
            "Nothing",
        )],
    ),
];

const AUDIO_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: AUDIO_INPUT_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: AUDIO_OUTPUT_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: AUDIO_DEVICE_TYPE,
        kind: TypeKind::Record,
        fields: &[
            ("id", "String"),
            ("name", "String"),
            ("canInput", "Boolean"),
            ("canOutput", "Boolean"),
            ("isDefaultInput", "Boolean"),
            ("isDefaultOutput", "Boolean"),
        ],
    },
    BuiltinType {
        name: AUDIO_ENVELOPE_TYPE,
        kind: TypeKind::Record,
        fields: &[
            ("attackFrames", "Integer"),
            ("decayFrames", "Integer"),
            ("holdFrames", "Integer"),
            ("releaseFrames", "Integer"),
            ("sustainLevel", "Integer"),
        ],
    },
    BuiltinType {
        name: AUDIO_NOTE_TYPE,
        kind: TypeKind::Record,
        fields: &[
            ("frequencyHz", "Float"),
            ("noteFrames", "Integer"),
            ("envelope", AUDIO_ENVELOPE_TYPE),
            ("gainOverall", "Float"),
        ],
    },
];

/// Argument-validating return-type resolution (dual-direction overloads,
/// input-only `read`, arity-variant timed forms), delegating to the retained
/// `dispatch_resolve`.
struct AudioResolver;
impl BuiltinResolver for AudioResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static AUDIO_RESOLVER: AudioResolver = AudioResolver;

pub(crate) static AUDIO: BuiltinModule = BuiltinModule {
    name: "audio",
    functions: AUDIO_FUNCTIONS,
    types: AUDIO_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&AUDIO_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

/// User-facing `audio::` surface. The lowered-only internal names (the named-device
/// opens, the timed `read`/`poll`, and the per-direction `close`) are deliberately
/// excluded so `audio::readTimeout()` in source draws an unknown-function
/// diagnostic rather than a call-argument mismatch (bug-213, the bug-173-E pattern
/// already applied to `tls`/`thread`). Codegen/IR-lowering sites that see the
/// synthesized names use [`is_audio_runtime_call`].
pub(crate) fn is_audio_call(name: &str) -> bool {
    DefaultResolver::contains(&AUDIO, name)
}

/// Post-lowering classifier: [`is_audio_call`] plus the internal names IR lowering
/// synthesizes (`audio::implementation_name`). Used by the runtime/plan sites that
/// route codegen and imports for those lowered-only targets.
pub(crate) fn is_audio_runtime_call(name: &str) -> bool {
    is_audio_call(name)
        || matches!(
            name,
            OPEN_INPUT_DEVICE
                | OPEN_OUTPUT_DEVICE
                | READ_TIMEOUT
                | POLL_TIMEOUT
                | CLOSE_INPUT
                | CLOSE_OUTPUT
        )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    // AudioEnvelope/AudioNote are constructible value records defined ALSO in the
    // source companion as `EXPORT TYPE` (the `vector::` value-record pattern).
    AUDIO.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        AUDIO_INPUT_TYPE => Some(CLOSE_INPUT),
        AUDIO_OUTPUT_TYPE => Some(CLOSE_OUTPUT),
        _ => None,
    }
}

// `call_param_names`/`call_param_name_overloads`/`expected_arguments`/
// `argument_types`/`implementation_name` return `&'static` borrowed shapes the
// owned `DefaultResolver` cannot produce; `expected_arguments`/`argument_types`
// also use bespoke phrasing. They stay static: `call_param_names` and
// `call_param_name_overloads` PINNED equal to `AUDIO` by the parity test
// (`DefaultResolver::param_names`/`param_name_overloads` derive them); the rest
// verified by the existing tests. BB removes them.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        DEVICES => Some(&[]),
        // openInput/openOutput place `device` at a different position across
        // their two overloads, so they carry a per-overload table instead
        // (`call_param_name_overloads`).
        READ => Some(&[&["input"], &["frames"], &["timeoutMs"]]),
        WRITE => Some(&[&["output"], &["bytes"]]),
        POLL => Some(&[&["stream"], &["timeoutMs"]]),
        AVAILABLE | XRUNS => Some(&[&["stream"]]),
        CLOSE => Some(&[&["stream"]]),
        RENDER => Some(&[&["note"]]),
        PLAY => Some(&[&["output"], &["mml", "tracks"]]),
        _ => None,
    }
}

/// The source-companion target for `audio::render`/`audio::play` (the `__audio_*`
/// bodies in `audio_mml.mfb`). `play` picks its single- vs multi-track body
/// from the second argument's type. Native calls return `None` and stay runtime
/// helpers. The result is internalized by IR lowering (it is a source function).
pub(crate) fn source_implementation_name(name: &str, arg_types: &[String]) -> Option<&'static str> {
    match name {
        RENDER => Some(INTERNAL_RENDER),
        PLAY if exact(arg_types, &[AUDIO_OUTPUT_TYPE, "List OF String"]) => {
            Some(INTERNAL_PLAY_TRACKS)
        }
        PLAY => Some(INTERNAL_PLAY),
        _ => None,
    }
}

// bug-339 C11: the audio companion is two unrelated subsystems — the tone
// renderer (`audio_render.mfb`, with the shared s16 clamp/emit helpers) and the
// MML sequencer (`audio_mml.mfb`) — concatenated into one source. The MML half
// relies on the IMPORTs and the shared helpers declared at the top of the render
// half, so the order is load-bearing.
super::package_source_glue!(
    "audio",
    "<builtin-audio>",
    "builtins/audio.mfb",
    concat!(
        include_str!("audio_render.mfb"),
        include_str!("audio_mml.mfb")
    )
);

/// Per-overload parameter names for the device-open calls, whose two overloads
/// disagree on where `sampleRate`/`channels`/`bufferFrames` sit.
pub(crate) fn call_param_name_overloads(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        OPEN_INPUT | OPEN_OUTPUT => Some(&[
            &["sampleRate", "channels", "bufferFrames"],
            &["device", "sampleRate", "channels", "bufferFrames"],
        ]),
        _ => None,
    }
}

/// The lowered-only internal names `implementation_name` synthesizes. They are not
/// user-callable, so `builtins::is_builtin_call` excludes them explicitly — its
/// `call_return_type_name` fallback would otherwise re-admit them as a builtin and
/// silently miscompile `audio::readTimeout()` (bug-213).
pub(crate) fn is_audio_internal_call(name: &str) -> bool {
    matches!(
        name,
        OPEN_INPUT_DEVICE
            | OPEN_OUTPUT_DEVICE
            | READ_TIMEOUT
            | POLL_TIMEOUT
            | CLOSE_INPUT
            | CLOSE_OUTPUT
    )
}

/// Return type of an `audio::` call. This **must** keep the lowered-only internal
/// names: IR lowering rewrites e.g. `audio::close` to `audio.closeOutput` and then
/// queries this for the rewritten target's return type. The user-facing gate is
/// `is_audio_call` / `is_audio_internal_call`, not this lookup.
pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    // User-facing calls resolve through the descriptor (each has a fixed return);
    // the lowered-only internal names are not descriptor functions, so they fall
    // back to this explicit map. See [`is_audio_internal_call`].
    DefaultResolver::return_type_name(&AUDIO, name).or_else(|| match name {
        OPEN_INPUT_DEVICE => Some(AUDIO_INPUT_TYPE),
        OPEN_OUTPUT_DEVICE => Some(AUDIO_OUTPUT_TYPE),
        READ_TIMEOUT => Some("List OF Byte"),
        POLL_TIMEOUT => Some("Boolean"),
        CLOSE_INPUT | CLOSE_OUTPUT => Some("Nothing"),
        _ => None,
    })
}

/// The argument-validating return-type resolution, invoked through the descriptor
/// resolver by `resolve_call`. `read` is input-only, `write` output-only;
/// `poll`/`available`/`xruns`/`close` accept either handle; `open*` accept an
/// optional leading `AudioDevice`.
fn dispatch_resolve<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        DEVICES if arg_types.is_empty() => Cow::Borrowed("List OF AudioDevice"),
        OPEN_INPUT
            if exact(arg_types, &["Integer", "Integer", "Integer"])
                || exact(
                    arg_types,
                    &[AUDIO_DEVICE_TYPE, "Integer", "Integer", "Integer"],
                ) =>
        {
            Cow::Borrowed(AUDIO_INPUT_TYPE)
        }
        OPEN_OUTPUT
            if exact(arg_types, &["Integer", "Integer", "Integer"])
                || exact(
                    arg_types,
                    &[AUDIO_DEVICE_TYPE, "Integer", "Integer", "Integer"],
                ) =>
        {
            Cow::Borrowed(AUDIO_OUTPUT_TYPE)
        }
        // `read` is defined ONLY over `AudioInput` — no `AudioOutput` form, so a
        // swapped stream fails to resolve (plan-33-A §3.1).
        READ if exact(arg_types, &[AUDIO_INPUT_TYPE, "Integer"])
            || exact(arg_types, &[AUDIO_INPUT_TYPE, "Integer", "Integer"]) =>
        {
            Cow::Borrowed("List OF Byte")
        }
        // `write` is defined ONLY over `AudioOutput`.
        WRITE if exact(arg_types, &[AUDIO_OUTPUT_TYPE, "List OF Byte"]) => Cow::Borrowed("Nothing"),
        POLL if exact(arg_types, &[AUDIO_INPUT_TYPE])
            || exact(arg_types, &[AUDIO_OUTPUT_TYPE])
            || exact(arg_types, &[AUDIO_INPUT_TYPE, "Integer"])
            || exact(arg_types, &[AUDIO_OUTPUT_TYPE, "Integer"]) =>
        {
            Cow::Borrowed("Boolean")
        }
        AVAILABLE | XRUNS
            if exact(arg_types, &[AUDIO_INPUT_TYPE]) || exact(arg_types, &[AUDIO_OUTPUT_TYPE]) =>
        {
            Cow::Borrowed("Integer")
        }
        CLOSE
            if exact(arg_types, &[AUDIO_INPUT_TYPE]) || exact(arg_types, &[AUDIO_OUTPUT_TYPE]) =>
        {
            Cow::Borrowed("Nothing")
        }
        RENDER if exact(arg_types, &[AUDIO_NOTE_TYPE]) => Cow::Borrowed("List OF Byte"),
        // `play(output, mml)` and `play(output, tracks)` — a single MML track or
        // a list of tracks. Both write to the (non-owned) open output stream and
        // return nothing; the caller keeps and closes the stream.
        PLAY if exact(arg_types, &[AUDIO_OUTPUT_TYPE, "String"])
            || exact(arg_types, &[AUDIO_OUTPUT_TYPE, "List OF String"]) =>
        {
            Cow::Borrowed("Nothing")
        }
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        DEVICES => Some("no arguments"),
        OPEN_INPUT | OPEN_OUTPUT => {
            Some("Integer, Integer, Integer or AudioDevice, Integer, Integer, Integer")
        }
        // `timeoutMs` is optional (arity 2..=3) — spell it (bug-213).
        READ => Some("AudioInput, Integer[, Integer]"),
        WRITE => Some("AudioOutput, List OF Byte"),
        // `timeoutMs` is optional (arity 1..=2) — spell it (bug-213).
        POLL => Some("AudioInput or AudioOutput[, Integer]"),
        AVAILABLE | XRUNS => Some("AudioInput or AudioOutput"),
        CLOSE => Some("AudioInput or AudioOutput"),
        RENDER => Some("AudioNote"),
        PLAY => Some("AudioOutput, String or AudioOutput, List OF String"),
        _ => None,
    }
}

/// Concrete per-argument types for literal coercion (typing a `[1, 2]` list
/// literal as `List OF Byte`). Only `write` has a non-overloaded, list-bearing
/// signature; the overloaded/typed-receiver calls rely on explicit types.
pub(crate) fn argument_types(name: &str) -> Option<&'static str> {
    match name {
        WRITE => Some("AudioOutput, List OF Byte"),
        _ => None,
    }
}

/// The internal runtime-helper call name a surface call rewrites to during IR
/// lowering, when the overload needs its own body. Returns `None` for the calls
/// that keep their surface name (`devices`, three-arg `open*`, two-arg `read`,
/// one-arg `poll`, `write`, `available`, `xruns`). The result is a runtime
/// helper, not a source companion, so callers must not internalize it.
pub(crate) fn implementation_name(name: &str, arg_types: &[String]) -> Option<&'static str> {
    match name {
        OPEN_INPUT if arg_types.first().map(String::as_str) == Some(AUDIO_DEVICE_TYPE) => {
            Some(OPEN_INPUT_DEVICE)
        }
        OPEN_OUTPUT if arg_types.first().map(String::as_str) == Some(AUDIO_DEVICE_TYPE) => {
            Some(OPEN_OUTPUT_DEVICE)
        }
        READ if arg_types.len() == 3 => Some(READ_TIMEOUT),
        POLL if arg_types.len() == 2 => Some(POLL_TIMEOUT),
        CLOSE if arg_types.first().map(String::as_str) == Some(AUDIO_INPUT_TYPE) => {
            Some(CLOSE_INPUT)
        }
        CLOSE if arg_types.first().map(String::as_str) == Some(AUDIO_OUTPUT_TYPE) => {
            Some(CLOSE_OUTPUT)
        }
        _ => None,
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `audio.close` (and its per-direction internal bodies) consumes the handle it
/// closes; every other call only uses the handle.
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!(
        (name, index),
        (CLOSE, 0) | (CLOSE_INPUT, 0) | (CLOSE_OUTPUT, 0)
    )
}

use super::exact;

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn impl_name(name: &str, args: &[&str]) -> Option<&'static str> {
        implementation_name(name, &strings(args))
    }

    #[test]
    fn is_call_accepts_only_the_user_facing_surface() {
        // bug-213: `is_audio_call` is the *user-facing* classifier. The lowered-only
        // internal names must be excluded so `audio::readTimeout()` in source draws
        // an unknown-function diagnostic; `is_audio_runtime_call` accepts both.
        for n in [
            DEVICES,
            OPEN_INPUT,
            OPEN_OUTPUT,
            READ,
            WRITE,
            POLL,
            AVAILABLE,
            XRUNS,
            CLOSE,
            RENDER,
            PLAY,
        ] {
            assert!(is_audio_call(n), "{n}");
            assert!(is_audio_runtime_call(n), "{n}");
        }
        for n in [
            OPEN_INPUT_DEVICE,
            OPEN_OUTPUT_DEVICE,
            READ_TIMEOUT,
            POLL_TIMEOUT,
            CLOSE_INPUT,
            CLOSE_OUTPUT,
        ] {
            assert!(
                !is_audio_call(n),
                "internal name must not be user-facing: {n}"
            );
            assert!(is_audio_internal_call(n), "{n}");
            assert!(is_audio_runtime_call(n), "{n}");
            // call_return_type_name MUST still know them (IR lowering queries the
            // rewritten target), but is_builtin_call must not re-admit them via its
            // call_return_type_name fallback.
            assert!(call_return_type_name(n).is_some(), "{n}");
            assert!(
                !crate::builtins::is_builtin_call(n),
                "internal name must not be user-callable: {n}"
            );
            assert!(!crate::builtins::is_builtin_member(n), "{n}");
        }
        assert!(!is_audio_call("audio.nope"));
        assert!(!is_audio_runtime_call("audio.nope"));
    }

    #[test]
    fn builtin_types_and_close_functions() {
        assert!(is_builtin_type(AUDIO_INPUT_TYPE));
        assert!(is_builtin_type(AUDIO_OUTPUT_TYPE));
        assert!(is_builtin_type(AUDIO_DEVICE_TYPE));
        assert!(!is_builtin_type("String"));
        assert_eq!(resource_close_function(AUDIO_INPUT_TYPE), Some(CLOSE_INPUT));
        assert_eq!(
            resource_close_function(AUDIO_OUTPUT_TYPE),
            Some(CLOSE_OUTPUT)
        );
        // A device is a plain record, not a resource.
        assert_eq!(resource_close_function(AUDIO_DEVICE_TYPE), None);
    }

    #[test]
    fn implementation_name_rewrites() {
        // Default-device opens keep their surface name.
        assert_eq!(
            impl_name(OPEN_INPUT, &["Integer", "Integer", "Integer"]),
            None
        );
        assert_eq!(
            impl_name(OPEN_OUTPUT, &["Integer", "Integer", "Integer"]),
            None
        );
        // Named-device opens rewrite to the device body.
        assert_eq!(
            impl_name(
                OPEN_INPUT,
                &[AUDIO_DEVICE_TYPE, "Integer", "Integer", "Integer"]
            ),
            Some(OPEN_INPUT_DEVICE)
        );
        assert_eq!(
            impl_name(
                OPEN_OUTPUT,
                &[AUDIO_DEVICE_TYPE, "Integer", "Integer", "Integer"]
            ),
            Some(OPEN_OUTPUT_DEVICE)
        );
        // Timed read/poll rewrite; the untimed forms keep their name.
        assert_eq!(impl_name(READ, &[AUDIO_INPUT_TYPE, "Integer"]), None);
        assert_eq!(
            impl_name(READ, &[AUDIO_INPUT_TYPE, "Integer", "Integer"]),
            Some(READ_TIMEOUT)
        );
        assert_eq!(impl_name(POLL, &[AUDIO_INPUT_TYPE]), None);
        assert_eq!(
            impl_name(POLL, &[AUDIO_OUTPUT_TYPE, "Integer"]),
            Some(POLL_TIMEOUT)
        );
        // close routes per direction.
        assert_eq!(impl_name(CLOSE, &[AUDIO_INPUT_TYPE]), Some(CLOSE_INPUT));
        assert_eq!(impl_name(CLOSE, &[AUDIO_OUTPUT_TYPE]), Some(CLOSE_OUTPUT));
        // write/available/xruns/devices never rewrite.
        assert_eq!(impl_name(WRITE, &[AUDIO_OUTPUT_TYPE, "List OF Byte"]), None);
        assert_eq!(impl_name(AVAILABLE, &[AUDIO_INPUT_TYPE]), None);
        assert_eq!(impl_name(DEVICES, &[]), None);
    }

    #[test]
    fn expected_and_argument_types() {
        assert!(expected_arguments(READ).unwrap().contains("AudioInput"));
        assert!(expected_arguments(WRITE).unwrap().contains("AudioOutput"));
        assert_eq!(expected_arguments(DEVICES), Some("no arguments"));
        assert_eq!(argument_types(WRITE), Some("AudioOutput, List OF Byte"));
        assert_eq!(argument_types(READ), None);
        assert!(expected_arguments("audio.nope").is_none());
    }

    #[test]
    fn param_name_tables_well_formed() {
        // Surface calls with a stable positional layout use the merged table.
        assert_eq!(call_param_names(DEVICES), Some(&[][..]));
        assert_eq!(
            call_param_names(READ),
            Some(&[&["input"][..], &["frames"], &["timeoutMs"]][..])
        );
        assert!(call_param_names(WRITE).is_some());
        assert!(call_param_names(POLL).is_some());
        assert!(call_param_names(CLOSE).is_some());
        // The device-open calls carry a per-overload table instead.
        assert!(call_param_names(OPEN_INPUT).is_none());
        assert!(call_param_name_overloads(OPEN_INPUT).is_some());
        assert!(call_param_name_overloads(OPEN_OUTPUT).is_some());
        assert!(call_param_name_overloads(READ).is_none());
    }

    #[test]
    fn consumes_only_close() {
        assert!(consumes_argument(CLOSE, 0));
        assert!(consumes_argument(CLOSE_INPUT, 0));
        assert!(consumes_argument(CLOSE_OUTPUT, 0));
        assert!(!consumes_argument(CLOSE, 1));
        assert!(!consumes_argument(READ, 0));
        assert!(!consumes_argument(WRITE, 0));
    }

    #[test]
    fn return_type_names() {
        assert_eq!(call_return_type_name(DEVICES), Some("List OF AudioDevice"));
        assert_eq!(call_return_type_name(OPEN_INPUT), Some(AUDIO_INPUT_TYPE));
        assert_eq!(call_return_type_name(OPEN_OUTPUT), Some(AUDIO_OUTPUT_TYPE));
        assert_eq!(call_return_type_name(READ), Some("List OF Byte"));
        assert_eq!(call_return_type_name(WRITE), Some("Nothing"));
        assert_eq!(call_return_type_name(POLL), Some("Boolean"));
        assert_eq!(call_return_type_name(AVAILABLE), Some("Integer"));
        assert_eq!(call_return_type_name(XRUNS), Some("Integer"));
        assert_eq!(call_return_type_name(CLOSE), Some("Nothing"));
        assert_eq!(call_return_type_name(RENDER), Some("List OF Byte"));
        assert_eq!(call_return_type_name(PLAY), Some("Nothing"));
        assert!(call_return_type_name("audio.nope").is_none());
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        dispatch_resolve(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `req`/`reqa`/`opt`/`ov`/`af` are const fns invoked only in const context,
        // so their bodies never run at runtime. Call them here to exercise (and pin
        // the shape of) each constructor.
        let r = req("frames", "Integer");
        assert_eq!(r.name, "frames");
        assert_eq!(r.ty, ParameterType::Named("Integer"));
        assert_eq!(r.default, DefaultValue::None);
        assert!(r.aliases.is_empty());

        const ALIASES: &[&str] = &["tracks"];
        let a = reqa("mml", ALIASES, "String");
        assert_eq!(a.name, "mml");
        assert_eq!(a.aliases, ALIASES);
        assert_eq!(a.ty, ParameterType::Named("String"));
        assert_eq!(a.default, DefaultValue::None);

        let o = opt("timeoutMs", "Integer");
        assert_eq!(o.name, "timeoutMs");
        assert_eq!(o.ty, ParameterType::Named("Integer"));
        assert_eq!(
            o.default,
            DefaultValue::Fill {
                type_name: "Integer",
                expr: ""
            }
        );
        assert!(o.aliases.is_empty());

        let overload = ov(OPEN_OV0, AUDIO_INPUT_TYPE);
        assert_eq!(overload.params.len(), 3);
        assert_eq!(overload.params[0].name, "sampleRate");
        assert_eq!(overload.return_type, ReturnType::Fixed(AUDIO_INPUT_TYPE));
        let niladic = ov(&[], "List OF AudioDevice");
        assert!(niladic.params.is_empty());
        assert_eq!(
            niladic.return_type,
            ReturnType::Fixed("List OF AudioDevice")
        );

        const OV: &[BuiltinOverload] = &[ov(&[req("note", AUDIO_NOTE_TYPE)], "List OF Byte")];
        let func = af(RENDER, "render", OV);
        assert_eq!(func.name, RENDER);
        assert_eq!(func.doc_slug, "render");
        assert_eq!(func.implementation, Implementation::Custom);
        assert_eq!(func.lowering, Lowering::Helper);
        assert_eq!(func.overloads.len(), 1);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }

    #[test]
    fn resolver_trait_dispatches() {
        // AudioResolver::resolve_return_type is wired through the descriptor; call
        // it directly to cover the delegation into `dispatch_resolve`.
        assert_eq!(
            AUDIO_RESOLVER.resolve_return_type(
                &AUDIO,
                READ,
                &strings(&[AUDIO_INPUT_TYPE, "Integer"])
            ),
            Some("List OF Byte".to_string())
        );
        assert_eq!(
            AUDIO_RESOLVER.resolve_return_type(
                &AUDIO,
                READ,
                &strings(&[AUDIO_OUTPUT_TYPE, "Integer"])
            ),
            None
        );
    }

    #[test]
    fn source_implementation_name_dispatch() {
        // render always maps to its body; play picks single- vs multi-track by the
        // second argument's type; native calls return None.
        assert_eq!(
            source_implementation_name(RENDER, &[]),
            Some(INTERNAL_RENDER)
        );
        assert_eq!(
            source_implementation_name(PLAY, &strings(&[AUDIO_OUTPUT_TYPE, "String"])),
            Some(INTERNAL_PLAY)
        );
        assert_eq!(
            source_implementation_name(PLAY, &strings(&[AUDIO_OUTPUT_TYPE, "List OF String"])),
            Some(INTERNAL_PLAY_TRACKS)
        );
        assert_eq!(source_implementation_name(DEVICES, &[]), None);
    }

    #[test]
    fn dispatch_resolve_every_overload() {
        assert_eq!(rt(DEVICES, &[]), Some("List OF AudioDevice".to_string()));
        assert_eq!(rt(DEVICES, &["Integer"]), None);
        // open* accept an optional leading AudioDevice.
        assert_eq!(
            rt(OPEN_INPUT, &["Integer", "Integer", "Integer"]),
            Some(AUDIO_INPUT_TYPE.to_string())
        );
        assert_eq!(
            rt(
                OPEN_INPUT,
                &[AUDIO_DEVICE_TYPE, "Integer", "Integer", "Integer"]
            ),
            Some(AUDIO_INPUT_TYPE.to_string())
        );
        assert_eq!(
            rt(OPEN_OUTPUT, &["Integer", "Integer", "Integer"]),
            Some(AUDIO_OUTPUT_TYPE.to_string())
        );
        assert_eq!(
            rt(
                OPEN_OUTPUT,
                &[AUDIO_DEVICE_TYPE, "Integer", "Integer", "Integer"]
            ),
            Some(AUDIO_OUTPUT_TYPE.to_string())
        );
        assert_eq!(rt(OPEN_INPUT, &["Integer", "Integer"]), None);
        // read is input-only (both arities); write is output-only.
        assert_eq!(
            rt(READ, &[AUDIO_INPUT_TYPE, "Integer"]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(
            rt(READ, &[AUDIO_INPUT_TYPE, "Integer", "Integer"]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(rt(READ, &[AUDIO_OUTPUT_TYPE, "Integer"]), None);
        assert_eq!(
            rt(WRITE, &[AUDIO_OUTPUT_TYPE, "List OF Byte"]),
            Some("Nothing".to_string())
        );
        assert_eq!(rt(WRITE, &[AUDIO_INPUT_TYPE, "List OF Byte"]), None);
        // poll/available/xruns/close accept either handle.
        for t in [AUDIO_INPUT_TYPE, AUDIO_OUTPUT_TYPE] {
            assert_eq!(rt(POLL, &[t]), Some("Boolean".to_string()));
            assert_eq!(rt(POLL, &[t, "Integer"]), Some("Boolean".to_string()));
            assert_eq!(rt(AVAILABLE, &[t]), Some("Integer".to_string()));
            assert_eq!(rt(XRUNS, &[t]), Some("Integer".to_string()));
            assert_eq!(rt(CLOSE, &[t]), Some("Nothing".to_string()));
        }
        assert_eq!(rt(POLL, &["Integer"]), None);
        assert_eq!(rt(CLOSE, &["String"]), None);
        // render/play source overloads.
        assert_eq!(
            rt(RENDER, &[AUDIO_NOTE_TYPE]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(rt(RENDER, &["List OF Byte"]), None);
        assert_eq!(
            rt(PLAY, &[AUDIO_OUTPUT_TYPE, "String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(
            rt(PLAY, &[AUDIO_OUTPUT_TYPE, "List OF String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(rt(PLAY, &[AUDIO_OUTPUT_TYPE, "Integer"]), None);
        assert_eq!(rt("audio.nope", &[]), None);
    }

    #[test]
    fn expected_arguments_all_branches() {
        assert_eq!(
            expected_arguments(OPEN_INPUT),
            Some("Integer, Integer, Integer or AudioDevice, Integer, Integer, Integer")
        );
        assert_eq!(
            expected_arguments(OPEN_OUTPUT),
            Some("Integer, Integer, Integer or AudioDevice, Integer, Integer, Integer")
        );
        assert_eq!(
            expected_arguments(POLL),
            Some("AudioInput or AudioOutput[, Integer]")
        );
        assert_eq!(
            expected_arguments(AVAILABLE),
            Some("AudioInput or AudioOutput")
        );
        assert_eq!(expected_arguments(XRUNS), Some("AudioInput or AudioOutput"));
        assert_eq!(expected_arguments(CLOSE), Some("AudioInput or AudioOutput"));
        assert_eq!(expected_arguments(RENDER), Some("AudioNote"));
        assert_eq!(
            expected_arguments(PLAY),
            Some("AudioOutput, String or AudioOutput, List OF String")
        );
    }
}
