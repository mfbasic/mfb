//! The built-in `tls` package (transport-layer security) on the clean-room
//! registry.
//!
//! `tls` opens outbound TLS client connections, terminates inbound TLS
//! connections, and transfers encrypted application data over both. Its two
//! resources — `TlsSocket` (a connected stream) and `TlsListener` (a bound server
//! endpoint) — are opaque, owned, non-copyable handles released by lexical drop.
//!
//! Like `process`/`os`/`fs`, `tls` is a **native OS-seam** package: every member
//! carries a per-platform runtime-helper lowering. The per-backend emission
//! (Linux OpenSSL, Windows Schannel, macOS Network.framework) lives in
//! [`native`]; each member's [`Body::native_os_seam`] holds the one family-generic
//! dispatcher [`native::lower_tls_helper`] in both the posix and win slots (the
//! `os`/`fs` twin idiom), and the generic runtime-call dispatch
//! (`crate::codegen::os::dispatch_runtime_helper` → `registry::os_helper`) picks
//! the slot by `platform.family()` and routes each member (plus the two code-form
//! aliases `tls.pollList` / `tls.closeListener`) to it.
//!
//! Every call's return type is fixed per name except `poll`, which is
//! return-type-overloaded on argument shape — a scalar `TlsSocket` yields
//! `Boolean`, a `List OF RES tls.TlsSocket` yields a borrowed `TlsSocket`. That is
//! two distinct `RegistryFunction` overloads (the datetime/net idiom), so the
//! registry's generic overload/return resolution answers everything with no custom
//! resolver.

use crate::codegen::registry::{Registry, RegistryPackage, RegistryResource};

pub(crate) mod native;

mod func_accept;
mod func_close;
mod func_connect;
mod func_listen;
mod func_poll;
mod func_read;
mod func_read_text;
mod func_write;
mod func_write_text;

/// The `TlsSocket` resource handle's bare type name — its identity *within* the
/// `tls` package (the `RegistryResource` name, the `type` half of the qualified
/// id). Used only for registry-internal lookups (`resolve_type`/close-op).
pub(crate) const TLS_SOCKET_TYPE: &str = "TlsSocket";
/// The `TlsListener` resource handle's bare type name.
pub(crate) const TLS_LISTENER_TYPE: &str = "TlsListener";

/// The `TlsSocket` resource's **package-qualified type identity** (`tls.TlsSocket`)
/// — plan-97 / bug-441. The string every `RES` binding, parameter, and return of a
/// tls socket carries; the `ResourceRegistry` key and what close-op dispatch sees.
pub(crate) const TLS_SOCKET_TYPE_ID: &str = "tls.TlsSocket";
/// The `TlsListener` resource's package-qualified type identity (`tls.TlsListener`).
pub(crate) const TLS_LISTENER_TYPE_ID: &str = "tls.TlsListener";

/// Internal listener-shaped close body. `tls::close` stays the single user-facing
/// name over both handle types; IR lowering routes a `TlsListener` operand here
/// because the two records differ in shape (plan-06-tls-server.md §4.1/§6.4). Not
/// user-callable — it is the `TlsListener` resource's registered close op and a
/// code-form alias of the `close` member's OS-seam lowering.
pub(crate) const CLOSE_LISTENER: &str = "tls.closeListener";

/// The unbounded-timeout sentinel a `Fill`ed trailing `timeoutMs`/`serverName`
/// injects when omitted (the timeout convention's omit=unbounded rule), shared by
/// the func-file descriptors.
pub(super) const SENTINEL: &str = crate::target::shared::code::TIMEOUT_UNBOUNDED_SENTINEL;

const MODULE_INTRO: &str =
    r#"TLS client connections, TLS termination, and encrypted application-data transfer"#;
const MODULE_DESC: &str = r#"The `tls` package opens outbound TLS client connections, terminates inbound TLS
connections, and reads and writes encrypted application data over both.
`tls::connect` resolves a host, opens a TCP stream, performs a TLS client
handshake, and verifies the peer's certificate before returning a connected
socket. `tls::listen` binds a local port and loads a server certificate and key,
and `tls::accept` accepts one inbound connection and completes the server-side
handshake, returning a socket that is byte-for-byte interchangeable with a client
socket. `tls::read` and `tls::readText` receive decrypted data; `tls::write` and
`tls::writeText` send data; and `tls::close` tears down a socket or a listener.
For plain unencrypted TCP and UDP, use `net`.


The package defines two built-in types. `TlsSocket` is a connected TLS stream —
either an outbound client connection from `tls::connect` or an accepted server
connection from `tls::accept`. `TlsListener` is a bound, listening server
endpoint from `tls::listen` that owns the loaded server TLS context; `tls::accept`
draws connections from it. Both are opaque, owned, non-copyable resource handles.
Each is closed automatically by lexical drop when its binding leaves scope, so
`tls::close` is needed only to release a handle earlier; unlike `net::close`,
`tls::close` consumes the handle and treats an already-closed handle as success
rather than an error. Neither handle type is thread-sendable, and neither can be
stored as a collection element or carried in a record.


The server's TLS context is owned by the `TlsListener` and borrowed by every
`TlsSocket` that `tls::accept` returns from it: closing an accepted socket never
frees the shared context, which is released exactly once when the listener
closes. Accepted sockets may therefore be closed in any order, and the listener
may be closed while accepted sockets are still live. The server presents its
certificate but does not request or verify a client certificate — there is no
mutual TLS, session resumption, ALPN, or SNI-based certificate selection in this
version.


The read and write functions come in paired byte/text forms: the byte form
transfers a `List OF Byte` verbatim, while the text form transfers a `String`'s
UTF-8 bytes directly and validates received bytes as UTF-8. Each read performs
one underlying TLS read and returns as soon as any plaintext is available, so a
result is frequently shorter than `maxBytes` and never empty on success; end of
stream is reported as an error rather than an empty result, so read in a loop
until the connection is closed. Each write transmits the entire buffer, looping
internally to resend any portion a single TLS write did not accept. TLS is
implemented on Linux by driving the system OpenSSL library (`libssl.so.3`,
falling back to `libssl.so.1.1`) so a single binary spans OpenSSL 1.1.1 and 3.x;
the macOS backend drives Network.framework through a synchronous bridge."#;

/// Register the `tls` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("tls", MODULE_INTRO, MODULE_DESC);

    // The opaque `TlsSocket` / `TlsListener` handles are semantic-only resources
    // (no injectable source): they make `registry().qualified_builtin_type` and
    // `registry::resource_close_function` answer generically, replacing the deleted
    // per-package `is_builtin_type`/`resource_close_function` seams. A `TlsSocket`'s
    // close op is the public `tls.close`; a `TlsListener`'s is the internal
    // listener-shaped `tls.closeListener` scope-drop body (a user `tls::close` over a
    // `TlsListener` is rewritten to it during IR lowering).
    pkg.add_resource(RegistryResource {
        name: TLS_SOCKET_TYPE,
        export: true,
        description: "A connected TLS stream — an outbound client connection from \
                      `tls::connect` or an accepted server connection from `tls::accept` — \
                      closed automatically when it leaves scope.",
        close_function: "tls.close",
        // A TLS session is driven from its owning thread; not thread-sendable in v1
        // (plan-03-net.md §4.4).
        sendable: false,
        close_may_fail: true,
        kind: crate::builtins::resource::ResourceKind::Builtin,
    });
    pkg.add_resource(RegistryResource {
        name: TLS_LISTENER_TYPE,
        export: true,
        description: "A bound, listening server endpoint from `tls::listen` that owns the \
                      loaded server TLS context; `tls::accept` draws connections from it.",
        close_function: CLOSE_LISTENER,
        // The listener owns the server TLS context and accepts on its own thread; not
        // thread-sendable in v1 (plan-06-tls-server.md §1).
        sendable: false,
        close_may_fail: true,
        kind: crate::builtins::resource::ResourceKind::Builtin,
    });

    func_connect::register(&mut pkg);
    func_listen::register(&mut pkg);
    func_accept::register(&mut pkg);
    func_read::register(&mut pkg);
    func_read_text::register(&mut pkg);
    func_write::register(&mut pkg);
    func_write_text::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_close::register(&mut pkg);

    r.add_package(pkg);
}
