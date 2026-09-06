use super::*;

#[test]
fn builtins_recognize_standard_resources() {
    assert!(is_builtin_resource_type(
        &crate::types::ParameterType::declared("fs.File")
    ));
    assert!(is_builtin_resource_type(
        &crate::types::ParameterType::declared("tcp.Socket")
    ));
    assert!(is_builtin_resource_type(
        &crate::types::ParameterType::declared("tcp.Listener")
    ));
    // plan-110-E: net has no resources of its own any more.
    assert!(!is_builtin_resource_type(
        &crate::types::ParameterType::declared("net.Socket")
    ));
    assert!(!is_builtin_resource_type(
        &crate::types::ParameterType::declared("Integer")
    ));
    assert!(!is_builtin_resource_type(
        &crate::types::ParameterType::declared("Address")
    ));
}

#[test]
fn builtins_carry_close_op_and_sendability() {
    let descriptor = |name: &str| match registry().resolve_type(name) {
        Some(ResolvedType::Resource(r)) => r,
        _ => panic!("{name} is not a built-in resource"),
    };
    assert_eq!(
        builtin_resource_close_function(&crate::types::ParameterType::declared("tcp.Socket")),
        Some("tcp.close")
    );
    assert_eq!(
        builtin_resource_close_function(&crate::types::ParameterType::declared("tcp.Listener")),
        Some("tcp.close")
    );
    // bug-464: every transport and file handle moves across threads. The
    // Listener asserted `!sendable` here until then, on plan-03-net.md
    // §4.4's v1 policy deferral rather than any property of its record.
    for sendable in ["fs.File", "tcp.Socket", "tcp.Listener", "udp.Socket"] {
        assert!(
            is_builtin_sendable_resource_type(&crate::types::ParameterType::declared(sendable)),
            "{sendable} must be thread-sendable"
        );
    }
    // Still deliberately not sendable, so this keeps proving the bit is read
    // from the registry rather than always true. bug-464 left both out of
    // scope: a child process's waitpid semantics are per-thread on some
    // platforms, and an audio handle's callbacks are bound to a device thread.
    for unsendable in ["process.Process", "audio.AudioInput"] {
        assert!(
            !is_builtin_sendable_resource_type(&crate::types::ParameterType::declared(unsendable)),
            "{unsendable} must not be thread-sendable"
        );
    }
    // close-may-fail holds for every standard resource (the descriptor
    // states it; drop-time cleanup derives the same fact from the close
    // wrapper's `SUCCESS ON`).
    assert!(descriptor("fs.File").close_may_fail);
    assert!(descriptor("tcp.Listener").close_may_fail);
}

/// bug-524: `close` is the language's word for the resource-invalidation
/// event that releases a handle (`mfb spec language resource-management`
/// §15). A package member spelled `close` that is *not* its resource's
/// registered close op says the opposite of what it does: `process::close`
/// closed the child's standard input and left the handle open and the child
/// running, which needed three qualifying paragraphs on its own page to
/// explain. Every member named `close` must therefore be the close op of one
/// of its package's resources, counting the `os_aliases` code forms a close
/// op may be registered under (`audio.closeInput`/`audio.closeOutput`).
#[test]
fn a_member_named_close_is_its_packages_resource_close_op() {
    use crate::codegen::registry::Body;
    for pkg in registry().packages() {
        let import = pkg.import_name();
        let Some(function) = pkg.functions().iter().find(|f| f.name == "close") else {
            continue;
        };
        // Every call form `close` can lower to: the member itself plus each
        // `os_aliases` overload-split code form.
        let mut forms = vec![format!("{import}.{}", function.name)];
        for implementation in function.implementations() {
            if let Body::AbiFunction { os_aliases, .. } = &implementation.body {
                for alias in os_aliases.iter().copied() {
                    forms.push(format!("{import}.{alias}"));
                }
            }
        }
        assert!(
            pkg.resources()
                .iter()
                .any(|r| forms.iter().any(|f| f == r.close_function)),
            "`{import}::close` is not the registered close op of any {import} resource \
                 ({forms:?} vs {:?}) — a member named `close` that does not close its \
                 package's handle is bug-524's footgun; name it for what it does",
            pkg.resources()
                .iter()
                .map(|r| r.close_function)
                .collect::<Vec<_>>()
        );
    }
}

/// bug-525: **every** built-in `close` refuses an already-closed handle.
///
/// `mfb spec language resource-management` §15 states the rule for the whole
/// language — "an already-closed record is flagged, and a second close is a
/// defined no-op reported as `ErrResourceClosed` rather than an operation on
/// a dead handle". `fs`, `tcp` and `udp` implemented it; `tls` and `audio`
/// set the closed flag once and then reported *success* forever after, and
/// each page documented its own answer, so the divergence was ratified
/// rather than noticed.
///
/// This lowers each close emitter and asserts the already-closed arm
/// relocates the `ErrResourceClosed` message — the signature of
/// `emit_fail`, and something a "return OK" arm cannot produce. It is the
/// cross-transport half of the pin: `tests/rt_double_close_is_refused.rs`
/// measures the same rule at runtime for `fs`/`tcp`/`udp`/`tls`, but it
/// cannot reach a `tls::Socket` (which needs a completed handshake), any
/// `audio` handle (which needs a device), or the Schannel and WASAPI
/// backends at all. Those are exactly the rows below.
#[test]
fn every_builtin_close_refuses_an_already_closed_handle() {
    use crate::codegen::builtins::{audio, tls};
    use crate::codegen::engine::mir;
    use crate::codegen::engine::tests::TestPlatform;
    use std::collections::HashMap;

    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports: HashMap<String, String> = HashMap::new();
    let closed = crate::codegen::registry::runtime_error_emission("ErrResourceClosed")
        .expect("ErrResourceClosed is an errorCode constant")
        .1;

    // (label, relocations) for every close emitter that a runtime fixture
    // cannot reach on this host. `fs`/`tcp`/`udp` share
    // `fs::gen_handle::lower_fs_close_helper`, whose already-closed arm is
    // measured directly by the runtime test on every host it runs on.
    let mut lowered: Vec<(&str, Vec<crate::codegen::engine::types::CodeRelocation>)> = Vec::new();
    let push = |label: &'static str,
                parts: Result<
        (
            Vec<crate::codegen::engine::types::CodeInstruction>,
            Vec<crate::codegen::engine::types::CodeRelocation>,
            usize,
        ),
        String,
    >,
                sink: &mut Vec<_>| {
        let (_ins, rel, _frame) = parts.unwrap_or_else(|e| panic!("lower {label}: {e}"));
        sink.push((label, rel));
    };

    push(
        "tls::close (macOS / Network.framework)",
        tls::gen_macos::lower_tls_close_macos("t_tls_close_mac", &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "tls::close listener (macOS / Network.framework)",
        tls::gen_macos::lower_tls_close_listener_macos("t_tls_closel_mac", &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "tls::close (Linux / OpenSSL)",
        tls::gen_openssl::lower_tls_close_openssl("t_tls_close_ssl", &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "tls::close listener (Linux / OpenSSL)",
        tls::gen_openssl::lower_tls_close_listener_openssl(
            "t_tls_closel_ssl",
            &imports,
            &TestPlatform,
        ),
        &mut lowered,
    );
    push(
        "tls::close (Windows / Schannel)",
        tls::gen_schannel::lower_tls_close("t_tls_close_sch", &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "tls::close listener (Windows / Schannel)",
        tls::gen_schannel::lower_tls_close_listener("t_tls_closel_sch", &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "audio::close input (macOS / CoreAudio)",
        audio::gen_macos_stream::lower_close_input("t_au_ci_mac", &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "audio::close output (macOS / CoreAudio)",
        audio::gen_macos_stream::lower_close_output("t_au_co_mac", &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "audio::close input (Linux / ALSA)",
        audio::gen_alsa_stream::lower_close("t_au_ci_alsa", true, &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "audio::close output (Linux / ALSA)",
        audio::gen_alsa_stream::lower_close("t_au_co_alsa", false, &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "audio::close input (Windows / WASAPI)",
        audio::gen_windows::lower_close("t_au_ci_win", true, &imports, &TestPlatform),
        &mut lowered,
    );
    push(
        "audio::close output (Windows / WASAPI)",
        audio::gen_windows::lower_close("t_au_co_win", false, &imports, &TestPlatform),
        &mut lowered,
    );

    assert_eq!(
        lowered.len(),
        12,
        "the close-emitter census is two handle shapes x three backends for \
             each of `tls` and `audio`; a new backend belongs in this list"
    );
    for (label, rel) in &lowered {
        assert!(
            rel.iter().any(|r| r.to == closed),
            "{label}: an already-closed handle must be refused with \
                 ErrResourceClosed (`mfb spec language resource-management` §15), \
                 so this helper must relocate `{closed}`. Reporting success on a \
                 dead handle is bug-525."
        );
    }
}

/// bug-525: `tcp::listen` and `tls::listen` queue the same depth by default.
///
/// The two are documented as drop-in mirrors, and they defaulted `backlog`
/// differently — `tcp` padded `128` in the code layer, `tls` filled `0`
/// ("host default") from its descriptor — so the same program got a
/// different queue depth depending on which one it called. There is one
/// constant now; this asserts the descriptor half still reads it, because
/// the code-layer half is a `.to_string()` no test can see from here.
#[test]
fn both_listen_members_default_the_same_backlog() {
    use crate::codegen::builtins::net::DEFAULT_LISTEN_BACKLOG;
    use crate::codegen::registry::{registry, DefaultValue};

    let listen = registry()
        .resolve_package("tls")
        .expect("tls package")
        .functions()
        .iter()
        .find(|f| f.name == "listen")
        .expect("tls::listen")
        .implementations()[0]
        .params
        .iter()
        .find(|p| p.name == "backlog")
        .expect("tls::listen backlog")
        .default
        .clone();
    match listen {
        DefaultValue::Fill { expr, .. } => assert_eq!(
            expr, DEFAULT_LISTEN_BACKLOG,
            "tls::listen must default `backlog` to the same depth tcp::listen \
                 pads (bug-525) — the two are drop-in mirrors"
        ),
        other => panic!("tls::listen's backlog must stay a Fill default, got {other:?}"),
    }
    // `tcp::listen`'s is `DefaultValue::Optional`: the code layer pads it,
    // from the same constant. Pinning the shape keeps a future move to a
    // `Fill` from silently reintroducing a second spelling.
    let tcp = registry()
        .resolve_package("tcp")
        .expect("tcp package")
        .functions()
        .iter()
        .find(|f| f.name == "listen")
        .expect("tcp::listen")
        .implementations()[0]
        .params
        .iter()
        .find(|p| p.name == "backlog")
        .expect("tcp::listen backlog")
        .default
        .clone();
    assert!(
        matches!(tcp, DefaultValue::Optional),
        "tcp::listen's backlog is padded by the code layer; if it becomes a \
             Fill it must read DEFAULT_LISTEN_BACKLOG too"
    );
}

/// bug-522: the thread-transferable set has exactly one source of truth —
/// the `sendable` bit on each `RegistryResource` — and three man pages
/// restate it in prose the compiler never reads. bug-464 flipped
/// `tcp::Listener`, `tls::Socket` and `tls::Listener` to `true` and updated
/// the `thread` package intro; `thread::transfer`'s own page still said
/// "listeners and `tls::Socket` may not" a year later, which is the exact
/// restriction the intro's headline pattern ("accept on one thread and hand
/// each connection to a worker") depends on NOT existing.
///
/// This asserts every built-in resource's bit against an explicit table, so
/// flipping one without following the prose fails here rather than shipping
/// a page that contradicts the compiler.
#[test]
fn every_builtin_resource_sendability_matches_the_documented_set() {
    // (qualified type, may it cross a thread)
    const EXPECTED: &[(&str, bool)] = &[
        ("fs.File", true),
        ("tcp.Socket", true),
        ("tcp.Listener", true),
        ("udp.Socket", true),
        ("tls.Socket", true),
        ("tls.Listener", true),
        ("process.Process", false),
        ("audio.AudioInput", false),
        ("audio.AudioOutput", false),
        ("canvas.Image", false),
        ("canvas.Font", false),
    ];
    // The table has to be COMPLETE, not just correct: a new resource that
    // nobody added here would otherwise never be classified, and the three
    // pages below would silently omit it.
    let mut actual: Vec<(String, bool)> = Vec::new();
    for pkg in registry().packages() {
        for r in pkg.resources() {
            actual.push((format!("{}.{}", pkg.import_name(), r.name), r.sendable));
        }
    }
    actual.sort();
    let mut expected: Vec<(String, bool)> = EXPECTED
        .iter()
        .map(|(n, s)| ((*n).to_string(), *s))
        .collect();
    expected.sort();
    assert_eq!(
        actual, expected,
        "the built-in thread-transferable set changed. It is stated in PROSE on \
             three pages that no build can check — `thread::transfer` and \
             `thread::accept` (`src/codegen/builtins/thread/func_transfer.rs`, \
             `func_accept.rs`) and the `thread` package intro \
             (`thread/mod.rs`) — and on each resource's own type page, which \
             derives it from this bit. Update this table AND those pages together; \
             bug-522 is what happens when only one of them moves."
    );
}

#[test]
fn every_builtin_resource_has_a_close_op() {
    // The closed-default (plan-38) relies on every built-in resource being
    // closeable so scope-drop can no-op a closed-default record. Guard against
    // a new built-in added without a registered close op (which would also
    // need a closed-flag review at the canonical offset 16).
    for pkg in registry().packages() {
        for r in pkg.resources() {
            let name = format!("{}.{}", pkg.import_name(), r.name);
            assert_eq!(r.kind, ResourceKind::Builtin, "{name} must be Builtin");
            assert!(!r.close_function.is_empty(), "{name} has an empty close op");
        }
    }
    // The full set of built-ins the closed-default must cover.
    for name in [
        // All built-in resources carry their package-qualified identity (plan-97).
        "fs.File",
        "tcp.Socket",
        "tcp.Listener",
        // plan-110-B/C: the transport handles moved out of `net`.
        // `net.UdpSocket` is gone entirely — `udp.Socket` replaces it.
        "tcp.Socket",
        "tcp.Listener",
        "udp.Socket",
        "audio.AudioInput",
        "audio.AudioOutput",
        "tls.Socket",
        "tls.Listener",
        "process.Process",
    ] {
        let type_ = ParameterType::declared(name);
        assert!(
            is_builtin_resource_type(&type_),
            "{name} missing from registry"
        );
        assert!(
            builtin_resource_close_function(&type_).is_some_and(|c| !c.is_empty()),
            "{name} has no close op"
        );
    }
}

#[test]
fn free_helpers_match_registry() {
    assert!(is_builtin_resource_type(
        &crate::types::ParameterType::declared("fs.File")
    ));
    assert!(!is_builtin_resource_type(
        &crate::types::ParameterType::declared("Nothing")
    ));
    assert_eq!(
        builtin_resource_close_function(&crate::types::ParameterType::declared("tcp.Socket")),
        Some("tcp.close")
    );
    assert!(is_builtin_sendable_resource_type(
        &crate::types::ParameterType::declared("tcp.Socket")
    ));
    // bug-464: the Listener is sendable too, so `process.Process` is the
    // negative exemplar here now (see the note in
    // `builtins_carry_close_op_and_sendability`).
    assert!(is_builtin_sendable_resource_type(
        &crate::types::ParameterType::declared("tcp.Listener")
    ));
    assert!(!is_builtin_sendable_resource_type(
        &crate::types::ParameterType::declared("process.Process")
    ));
}
