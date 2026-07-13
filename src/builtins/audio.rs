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
use std::path::Path;

pub(crate) const AUDIO_INPUT_TYPE: &str = "AudioInput";
pub(crate) const AUDIO_OUTPUT_TYPE: &str = "AudioOutput";
pub(crate) const AUDIO_DEVICE_TYPE: &str = "AudioDevice";
/// Value records the user constructs and passes to `audio::render`. Registered
/// natively (fields below) so they are constructible, and defined in the source
/// companion (`audio_package.mfb`) so the source `render` operates on them — the
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
/// Source-companion synthesis helpers, defined in `audio_package.mfb` (like
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
const INTERNAL_PLAY_TRACKS: &str = "__audio_play_tracks";

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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_audio_call(name: &str) -> bool {
    matches!(
        name,
        DEVICES
            | OPEN_INPUT
            | OPEN_INPUT_DEVICE
            | OPEN_OUTPUT
            | OPEN_OUTPUT_DEVICE
            | READ
            | READ_TIMEOUT
            | WRITE
            | POLL
            | POLL_TIMEOUT
            | AVAILABLE
            | XRUNS
            | CLOSE
            | CLOSE_INPUT
            | CLOSE_OUTPUT
            | RENDER
            | PLAY
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    // AudioEnvelope/AudioNote are registered here (and in `builtin_type_fields`)
    // so the user can construct them; they are ALSO defined in the source
    // companion as `EXPORT TYPE` so `audio::render` can operate on them. This is
    // the `vector::` value-record pattern.
    matches!(
        name,
        AUDIO_INPUT_TYPE
            | AUDIO_OUTPUT_TYPE
            | AUDIO_DEVICE_TYPE
            | AUDIO_ENVELOPE_TYPE
            | AUDIO_NOTE_TYPE
    )
}

pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match name {
        AUDIO_DEVICE_TYPE => Some(&[
            ("id", "String"),
            ("name", "String"),
            ("canInput", "Boolean"),
            ("canOutput", "Boolean"),
            ("isDefaultInput", "Boolean"),
            ("isDefaultOutput", "Boolean"),
        ]),
        // Must match `EXPORT TYPE AudioEnvelope`/`AudioNote` in audio_package.mfb.
        AUDIO_ENVELOPE_TYPE => Some(&[
            ("attackFrames", "Integer"),
            ("decayFrames", "Integer"),
            ("holdFrames", "Integer"),
            ("releaseFrames", "Integer"),
            ("sustainLevel", "Integer"),
        ]),
        AUDIO_NOTE_TYPE => Some(&[
            ("frequencyHz", "Float"),
            ("noteFrames", "Integer"),
            ("envelope", AUDIO_ENVELOPE_TYPE),
            ("gainOverall", "Float"),
        ]),
        _ => None,
    }
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        AUDIO_INPUT_TYPE => Some(CLOSE_INPUT),
        AUDIO_OUTPUT_TYPE => Some(CLOSE_OUTPUT),
        _ => None,
    }
}

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
/// bodies in `audio_package.mfb`). `play` picks its single- vs multi-track body
/// from the second argument's type. Native calls return `None` and stay runtime
/// helpers. The result is internalized by IR lowering (it is a source function).
pub(crate) fn source_implementation_name(
    name: &str,
    arg_types: &[String],
) -> Option<&'static str> {
    match name {
        RENDER => Some(INTERNAL_RENDER),
        PLAY if exact(arg_types, &[AUDIO_OUTPUT_TYPE, "List OF String"]) => {
            Some(INTERNAL_PLAY_TRACKS)
        }
        PLAY => Some(INTERNAL_PLAY),
        _ => None,
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-audio>"),
        "builtins/audio.mfb",
        include_str!("audio_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "audio")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
}

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

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        DEVICES => Some("List OF AudioDevice"),
        OPEN_INPUT | OPEN_INPUT_DEVICE => Some(AUDIO_INPUT_TYPE),
        OPEN_OUTPUT | OPEN_OUTPUT_DEVICE => Some(AUDIO_OUTPUT_TYPE),
        READ | READ_TIMEOUT | RENDER => Some("List OF Byte"),
        WRITE | CLOSE | CLOSE_INPUT | CLOSE_OUTPUT | PLAY => Some("Nothing"),
        // `poll` is `Boolean`, `available`/`xruns` are `Integer`, on either
        // direction; `resolve_call` returns the precise type per operand.
        POLL | POLL_TIMEOUT => Some("Boolean"),
        AVAILABLE | XRUNS => Some("Integer"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
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
        CLOSE if exact(arg_types, &[AUDIO_INPUT_TYPE]) || exact(arg_types, &[AUDIO_OUTPUT_TYPE]) => {
            Cow::Borrowed("Nothing")
        }
        RENDER if exact(arg_types, &[AUDIO_NOTE_TYPE]) => Cow::Borrowed("List OF Byte"),
        // `play(output, mml)` and `play(output, tracks)` — a single MML track or
        // a list of tracks. Both write to the (borrowed) open output stream and
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
        READ => Some("AudioInput, Integer, Integer"),
        WRITE => Some("AudioOutput, List OF Byte"),
        POLL => Some("AudioInput or AudioOutput, Integer"),
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

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        DEVICES => Some((0, 0)),
        OPEN_INPUT | OPEN_OUTPUT => Some((3, 4)),
        READ => Some((2, 3)),
        WRITE => Some((2, 2)),
        POLL => Some((1, 2)),
        AVAILABLE | XRUNS | CLOSE | RENDER => Some((1, 1)),
        CLOSE_INPUT | CLOSE_OUTPUT => Some((1, 1)),
        PLAY => Some((2, 2)),
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
        CLOSE if arg_types.first().map(String::as_str) == Some(AUDIO_INPUT_TYPE) => Some(CLOSE_INPUT),
        CLOSE if arg_types.first().map(String::as_str) == Some(AUDIO_OUTPUT_TYPE) => {
            Some(CLOSE_OUTPUT)
        }
        _ => None,
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `audio.close` (and its per-direction internal bodies) consumes the handle it
/// closes; every other call borrows.
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!(
        (name, index),
        (CLOSE, 0) | (CLOSE_INPUT, 0) | (CLOSE_OUTPUT, 0)
    )
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    fn impl_name(name: &str, args: &[&str]) -> Option<&'static str> {
        implementation_name(name, &strings(args))
    }

    #[test]
    fn is_call_accepts_surface_and_internal() {
        for n in [
            DEVICES,
            OPEN_INPUT,
            OPEN_INPUT_DEVICE,
            OPEN_OUTPUT,
            OPEN_OUTPUT_DEVICE,
            READ,
            READ_TIMEOUT,
            WRITE,
            POLL,
            POLL_TIMEOUT,
            AVAILABLE,
            XRUNS,
            CLOSE,
            CLOSE_INPUT,
            CLOSE_OUTPUT,
        ] {
            assert!(is_audio_call(n), "{n}");
        }
        assert!(!is_audio_call("audio.nope"));
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
    fn device_record_fields() {
        let fields = builtin_type_fields(AUDIO_DEVICE_TYPE).expect("device fields");
        assert_eq!(fields.len(), 6);
        assert_eq!(fields[0], ("id", "String"));
        assert_eq!(fields[1], ("name", "String"));
        assert_eq!(fields[2], ("canInput", "Boolean"));
        assert_eq!(fields[5], ("isDefaultOutput", "Boolean"));
        assert_eq!(builtin_type_fields(AUDIO_INPUT_TYPE), None);
    }

    #[test]
    fn resolve_devices_and_open() {
        assert_eq!(rt(DEVICES, &[]), Some("List OF AudioDevice".to_string()));
        assert_eq!(rt(DEVICES, &["Integer"]), None);
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
    }

    #[test]
    fn read_only_over_input_and_write_only_over_output() {
        // read resolves over AudioInput, both arities.
        assert_eq!(
            rt(READ, &[AUDIO_INPUT_TYPE, "Integer"]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(
            rt(READ, &[AUDIO_INPUT_TYPE, "Integer", "Integer"]),
            Some("List OF Byte".to_string())
        );
        // read over AudioOutput does NOT resolve — the §3.1 compile error.
        assert_eq!(rt(READ, &[AUDIO_OUTPUT_TYPE, "Integer"]), None);
        // write resolves over AudioOutput.
        assert_eq!(
            rt(WRITE, &[AUDIO_OUTPUT_TYPE, "List OF Byte"]),
            Some("Nothing".to_string())
        );
        // write over AudioInput does NOT resolve.
        assert_eq!(rt(WRITE, &[AUDIO_INPUT_TYPE, "List OF Byte"]), None);
    }

    #[test]
    fn resolve_poll_available_xruns_close_both_directions() {
        for t in [AUDIO_INPUT_TYPE, AUDIO_OUTPUT_TYPE] {
            assert_eq!(rt(POLL, &[t]), Some("Boolean".to_string()));
            assert_eq!(rt(POLL, &[t, "Integer"]), Some("Boolean".to_string()));
            assert_eq!(rt(AVAILABLE, &[t]), Some("Integer".to_string()));
            assert_eq!(rt(XRUNS, &[t]), Some("Integer".to_string()));
            assert_eq!(rt(CLOSE, &[t]), Some("Nothing".to_string()));
        }
        assert_eq!(rt(POLL, &["Integer"]), None);
        assert_eq!(rt(CLOSE, &["String"]), None);
    }

    #[test]
    fn implementation_name_rewrites() {
        // Default-device opens keep their surface name.
        assert_eq!(impl_name(OPEN_INPUT, &["Integer", "Integer", "Integer"]), None);
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
    fn arity_spans() {
        assert_eq!(arity(DEVICES), Some((0, 0)));
        assert_eq!(arity(OPEN_INPUT), Some((3, 4)));
        assert_eq!(arity(OPEN_OUTPUT), Some((3, 4)));
        assert_eq!(arity(READ), Some((2, 3)));
        assert_eq!(arity(WRITE), Some((2, 2)));
        assert_eq!(arity(POLL), Some((1, 2)));
        assert_eq!(arity(AVAILABLE), Some((1, 1)));
        assert_eq!(arity(XRUNS), Some((1, 1)));
        assert_eq!(arity(CLOSE), Some((1, 1)));
        assert!(arity("audio.nope").is_none());
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

    fn source_name(name: &str, args: &[&str]) -> Option<&'static str> {
        source_implementation_name(name, &strings(args))
    }

    #[test]
    fn source_companion_render_and_play() {
        // render/play are source-companion calls, accepted by is_audio_call.
        assert!(is_audio_call(RENDER));
        assert!(is_audio_call(PLAY));

        // render(AudioNote) -> List OF Byte; play(device, String|List OF String)
        // -> Nothing; wrong operand types do not resolve.
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
        // play is over AudioOutput, not the read-only device record.
        assert_eq!(rt(PLAY, &[AUDIO_DEVICE_TYPE, "String"]), None);
        assert_eq!(rt(PLAY, &["String", "String"]), None);

        // Source dispatch selects the matching companion body by argument type.
        assert_eq!(source_name(RENDER, &[AUDIO_NOTE_TYPE]), Some(INTERNAL_RENDER));
        assert_eq!(
            source_name(PLAY, &[AUDIO_OUTPUT_TYPE, "String"]),
            Some(INTERNAL_PLAY)
        );
        assert_eq!(
            source_name(PLAY, &[AUDIO_OUTPUT_TYPE, "List OF String"]),
            Some(INTERNAL_PLAY_TRACKS)
        );
        assert_eq!(source_name(DEVICES, &[]), None);

        // The native rewrite table never claims the source calls.
        assert_eq!(impl_name(PLAY, &[AUDIO_OUTPUT_TYPE, "String"]), None);
        assert_eq!(impl_name(RENDER, &[AUDIO_NOTE_TYPE]), None);

        assert_eq!(arity(RENDER), Some((1, 1)));
        assert_eq!(arity(PLAY), Some((2, 2)));
        assert!(expected_arguments(PLAY).unwrap().contains("AudioOutput"));
    }

    #[test]
    fn value_records_are_constructible_builtins() {
        // AudioEnvelope/AudioNote are constructible value records (unlike the
        // read-only device record and the opaque handles).
        assert!(is_builtin_type(AUDIO_ENVELOPE_TYPE));
        assert!(is_builtin_type(AUDIO_NOTE_TYPE));
        let env = builtin_type_fields(AUDIO_ENVELOPE_TYPE).expect("envelope fields");
        assert_eq!(env.len(), 5);
        assert_eq!(env[0], ("attackFrames", "Integer"));
        assert_eq!(env[4], ("sustainLevel", "Integer"));
        let note = builtin_type_fields(AUDIO_NOTE_TYPE).expect("note fields");
        assert_eq!(note.len(), 4);
        assert_eq!(note[0], ("frequencyHz", "Float"));
        assert_eq!(note[2], ("envelope", AUDIO_ENVELOPE_TYPE));
        // A device is read-only, so it is NOT a constructor target here.
        assert_eq!(resource_close_function(AUDIO_NOTE_TYPE), None);
    }
}
