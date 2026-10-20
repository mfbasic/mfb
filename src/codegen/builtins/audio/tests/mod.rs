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
            &strings(&["audio.AudioDevice", "Integer", "Integer", "Integer"]),
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

/// bug-483's class at the audio seam: `runtime_overload_name` matches on a
/// rendered type NAME, and bug-480 Phase 4b made a builtin value type's
/// declared identity package-qualified — so an `AudioDevice` argument reaches
/// this selector spelled `audio.AudioDevice`. Matching only the bare leaf let
/// the named-device open fall through to the DEFAULT-device body, which then
/// read the device operand as `sampleRate` and raised `ErrInvalidArgument` for
/// every argument combination. Both spellings must select the device form.
#[test]
fn named_device_open_rewrites_under_either_device_spelling() {
    for spelling in ["AudioDevice", "audio.AudioDevice"] {
        assert_eq!(
            super::runtime_overload_name(
                "audio.openOutput",
                &strings(&[spelling, "Integer", "Integer", "Integer"])
            ),
            Some("audio.openOutputDevice"),
            "`{spelling}` must select the named-device openOutput"
        );
        assert_eq!(
            super::runtime_overload_name(
                "audio.openInput",
                &strings(&[spelling, "Integer", "Integer", "Integer"])
            ),
            Some("audio.openInputDevice"),
            "`{spelling}` must select the named-device openInput"
        );
    }
    // The three-argument default-device form keeps its surface name.
    assert_eq!(
        super::runtime_overload_name(
            "audio.openOutput",
            &strings(&["Integer", "Integer", "Integer"])
        ),
        None
    );
    assert_eq!(
        super::runtime_overload_name(
            "audio.openInput",
            &strings(&["Integer", "Integer", "Integer"])
        ),
        None
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

/// The PCM accumulators in `__audio_render` and `__audio_mmlEncode` must stay
/// same-function locals. `collections::append` mutates a list in place only for
/// a local of the function doing the append; handing the growing accumulator to
/// a helper (`__audio_appendS16LE(pcm, sample)`, bug-339 C7) passes it BY VALUE,
/// so every sample copied the whole list and both loops ran O(n^2) — one second
/// of 48 kHz audio took ~10 s of CPU instead of ~12 ms, which read as
/// `audio::play` hanging. Assert the emit is inline, not a call taking `pcm`.
#[test]
fn pcm_accumulators_never_cross_a_call_boundary() {
    let source = registry()
        .resolve_package("audio")
        .expect("audio")
        .get_mfb();
    for func in ["FUNC __audio_render(", "FUNC __audio_mmlEncode("] {
        let start = source
            .find(func)
            .unwrap_or_else(|| panic!("{func} in source"));
        let body = &source[start..];
        let end = body.find("\nEND FUNC").expect("terminated body");
        for line in body[..end].lines() {
            let code = line.trim_start();
            if code.starts_with('\'') {
                continue;
            }
            // `collections::append(pcm, …)` on the function's own local IS the
            // in-place path. Handing `pcm` to an `__audio_*` helper is not.
            assert!(
                !(code.contains("__audio_") && code.contains("(pcm")),
                "{func}: `pcm` is passed to a helper — the accumulator is copied \
                     per sample and the loop is O(n^2): {line}"
            );
        }
    }
}

#[test]
fn reassembled_source_parses() {
    let source = registry()
        .resolve_package("audio")
        .expect("audio")
        .get_mfb();
    assert!(source.contains("EXPORT TYPE AudioNote"));
    assert!(source.contains("FUNC __audio_render"));
    assert!(source.contains("FUNC __audio_playTracks("));
    crate::ast::parse_source_internal(
        std::path::Path::new("<builtin-audio>"),
        "builtins/audio.mfb",
        &source,
    )
    .expect("reassembled audio source parses");
}

/// The first three instructions of a device-specific open's `id` read rebase the
/// flat `audio::AudioDevice` record (plan-132): load the record from
/// `record_slot`, load its `id` slot's block-relative offset from it, and add
/// the two. Before plan-132 the second load WAS the `String` pointer.
fn assert_reads_the_inlined_id(
    instructions: &[crate::codegen::engine::types::CodeInstruction],
    record_slot: usize,
) {
    use crate::arch::ops::CodeOp;
    let record_slot = record_slot.to_string();
    let id_field = super::gen_shared::DEVICE_FIELD_ID.to_string();

    assert_eq!(instructions[0].op, CodeOp::LdrU64, "load the record");
    assert_eq!(
        instructions[0].get("base").as_deref(),
        Some(crate::target::shared::abi::SP_SPELLING)
    );
    assert_eq!(
        instructions[0].get("offset").as_deref(),
        Some(record_slot.as_str())
    );
    let record = instructions[0].get("dst");
    assert!(record.is_some());

    assert_eq!(instructions[1].op, CodeOp::LdrU64, "load the id's offset");
    assert_eq!(instructions[1].get("base").as_deref(), record.as_deref());
    assert_eq!(
        instructions[1].get("offset").as_deref(),
        Some(id_field.as_str())
    );
    let offset = instructions[1].get("dst");

    assert_eq!(instructions[2].op, CodeOp::Add, "record + offset is the id");
    assert_eq!(instructions[2].get("lhs").as_deref(), record.as_deref());
    assert_eq!(instructions[2].get("rhs").as_deref(), offset.as_deref());
}

/// plan-132: Core Audio's `emit_select_device` reads the device UID from the flat
/// record. No host running this suite has an audio device, so this is the
/// reader's standing proof.
#[test]
fn core_audio_device_open_reads_the_inlined_id() {
    crate::codegen::engine::mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = std::collections::HashMap::new();
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = crate::codegen::engine::util::Vregs::new();
    let mut status_sites = crate::codegen::error::emission::StatusFailSites::new(
        "t_select_device",
        "ErrAudioDevice",
        "t_select_device_status_fail",
        super::gen_macos_shared::STREAM_FAIL_STATUS_OFF,
        super::gen_macos_shared::STREAM_FAIL_PREFIX_OFF,
    );
    super::gen_macos_shared::emit_select_device(
        &mut crate::codegen::engine::builder::EmitCtx {
            symbol: "t_select_device",
            platform_imports: &imports,
            platform: &crate::codegen::engine::tests::TestPlatform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "t_select_device_fail",
        &mut status_sites,
        &mut vregs,
    )
    .expect("lower the Core Audio device selection");
    assert_reads_the_inlined_id(&instructions, super::gen_macos_shared::DEVID_OFF);
    // The caller owns the status stubs; emit them as a real open does.
    status_sites.emit_stubs(
        crate::target::shared::abi::c_arg(0),
        &mut instructions,
        &mut relocations,
    );
}

/// plan-132: ALSA's `emit_device_cstring` reads the device name hint from the flat
/// record, for the same reason as the Core Audio pin above.
#[test]
fn alsa_device_open_reads_the_inlined_id() {
    crate::codegen::engine::mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let mut instructions = Vec::new();
    let mut vregs = crate::codegen::engine::util::Vregs::new();
    super::gen_alsa_shared::emit_device_cstring(
        super::gen_alsa_shared::DEVID_OFF,
        &mut instructions,
        "t_device_cstring",
        &mut vregs,
    );
    assert_reads_the_inlined_id(&instructions, super::gen_alsa_shared::DEVID_OFF);
}
