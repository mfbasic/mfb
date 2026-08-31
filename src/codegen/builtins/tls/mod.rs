//! The built-in `tls` package (transport-layer security) on the clean-room
//! registry.
//!
//! `tls` opens outbound TLS client connections, terminates inbound TLS
//! connections, and transfers encrypted application data over both. Its two
//! resources — `Socket` (a connected stream) and `Listener` (a bound server
//! endpoint) — are opaque, owned, non-copyable handles released by lexical drop.
//!
//! Like `process`/`os`/`fs`, `tls` is a **native OS-seam** package migrated to the
//! `Body::abi_function` clean-room shape: every member carries a per-platform
//! runtime-helper lowering. The per-backend emission (Linux OpenSSL, Windows
//! Schannel, macOS Network.framework) lives in the `gen_openssl`/`gen_schannel`/
//! `gen_macos` backends; each member owns its `Body::abi_function` body in its own
//! `func_*.rs` (`lower_<name>`), which calls the shared per-member family dispatcher
//! [`gen_shared::lower_tls_connect_helper`] et al. (picking the backend by
//! `platform.family()`) and finalizes. The two code-form aliases (`tls.pollList` /
//! `tls.closeListener`) route to the owning member's body (`func_poll` / `func_close`)
//! through `abi_function_lower`, distinguished off `AbiCtx::call`.
//!
//! Every call's return type is fixed per name except `poll`, which is
//! return-type-overloaded on argument shape — a scalar `Socket` yields
//! `Boolean`, a `List OF RES tls.Socket` yields a borrowed `Socket`. That is
//! two distinct `RegistryFunction` overloads (the datetime/net idiom), so the
//! registry's generic overload/return resolution answers everything with no custom
//! resolver.

// --- codegen tier imports (migration) ---
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    Registry, RegistryPackage, RegistryResource, ResourceLiveSlot, SlotBackend, SlotTransfer,
};
pub(crate) mod gen_macos;
mod gen_openssl;
pub(crate) mod gen_schannel;
pub(crate) mod gen_shared;

mod func_accept;
mod func_close;
mod func_connect;
mod func_listen;
mod func_local_address;
mod func_poll;
mod func_read;
mod func_remote_address;
mod func_set_read_timeout;
mod func_set_write_timeout;
mod func_write;

/// The TLS socket handle's bare type name — its identity *within* the `tls`
/// package (the `RegistryResource` name, the `type` half of the qualified id).
/// Used only for registry-internal lookups (`resolve_type`/close-op).
///
/// plan-110-D renamed these from `Socket`/`Listener`: the package name
/// already says TLS, exactly as `tcp::Socket` and `udp::Socket` do. The bare name
/// now collides with tcp's and udp's, which is harmless — a resource has no
/// injectable source declaration, so it never enters the shared top-level
/// namespace the way a record does (that is what forced plan-110-C's `Datagram`
/// removal). The QUALIFIED ids below stay distinct, and those are what every
/// binding, parameter and close-op dispatch actually carries.
pub(crate) const TLS_SOCKET_TYPE: &str = "Socket";
/// The TLS listener handle's bare type name.
pub(crate) const TLS_LISTENER_TYPE: &str = "Listener";

/// The TLS socket's **package-qualified type identity** (`tls.Socket`) — plan-97 /
/// bug-441. The string every `RES` binding, parameter, and return of a tls socket
/// carries; the `ResourceRegistry` key and what close-op dispatch sees.
pub(crate) const TLS_SOCKET_TYPE_ID: &str = "tls.Socket";
/// The TLS listener's package-qualified type identity (`tls.Listener`).
pub(crate) const TLS_LISTENER_TYPE_ID: &str = "tls.Listener";

/// Internal listener-shaped close body. `tls::close` stays the single user-facing
/// name over both handle types; IR lowering routes a `Listener` operand here
/// because the two records differ in shape (plan-06-tls-server.md §4.1/§6.4). Not
/// user-callable — it is the `Listener` resource's registered close op and a
/// code-form alias of the `close` member's OS-seam lowering.
pub(crate) const CLOSE_LISTENER: &str = "tls.closeListener";

/// The `Listener` overload of `tls::localAddress` (bug-465). A code-form alias,
/// not a member: `tls::localAddress` stays the single user-facing name over both
/// handle types and `builder_values` selects this form off the argument's static
/// type. The overload needs its own body because macOS answers the two handle
/// types through different Network.framework calls — a `Socket` has a path whose
/// effective endpoints carry a `sockaddr`, an `nw_listener` has only
/// `nw_listener_get_port`.
pub(crate) const LOCAL_ADDRESS_LISTENER: &str = "tls.localAddressListener";

/// The unbounded-timeout sentinel a `Fill`ed trailing `timeoutMs`/`serverName`
/// injects when omitted (the timeout convention's omit=unbounded rule), shared by
/// the func-file descriptors.
pub(crate) const SENTINEL: &str = TIMEOUT_UNBOUNDED_SENTINEL;

const MODULE_INTRO: &str =
    r#"TLS client connections, TLS termination, and encrypted application-data transfer"#;
const MODULE_DESC: &str = r#"The `tls` package opens outbound TLS client connections, terminates inbound TLS
connections, and reads and writes encrypted application data over both.
`tls::connect` resolves a host, opens a TCP stream, performs a TLS client
handshake, and verifies the peer's certificate before returning a connected
socket. `tls::listen` binds a local port and loads a server certificate and key,
and `tls::accept` accepts one inbound connection and completes the server-side
handshake, returning a socket that is byte-for-byte interchangeable with a client
socket. `tls::read` receives decrypted data as bytes; `tls::write` sends data,
accepting either bytes or a `String`; and `tls::close` tears down a socket or a
listener.
For plain unencrypted TCP use `tcp`; for datagrams use `udp`.


The package defines two built-in types. `Socket` is a connected TLS stream —
either an outbound client connection from `tls::connect` or an accepted server
connection from `tls::accept`. `Listener` is a bound, listening server
endpoint from `tls::listen` that holds the loaded server TLS settings; `tls::accept` draws connections from it. Both are opaque handles.
Each is closed automatically when its binding goes out of scope, so
`tls::close` is needed only to release a handle earlier; unlike `tcp::close`,
`tls::close` closes the handle and treats an already-closed handle as success
rather than an error. Either handle may be handed to another thread on a thread's
resource plane, so a server can accept on one thread and give each connection to
a worker. The thread that receives a handle is the one that closes it, and the
sending thread can no longer use it. Neither can be
carried in a record. Both may be collection elements when the element type is
spelled `RES` (a bare `List OF tls::Socket` is rejected with
`TYPE_RESOURCE_REQUIRES_RES`); `List OF RES tls::Socket` is what the `tls::poll`
multiplex form takes.


The `Listener` holds the server's TLS settings and every `Socket` that `tls::accept` returns shares them: closing an accepted socket leaves the listener and its settings intact, which is released exactly once when the listener
closes. Accepted sockets may therefore be closed in any order, and the listener
may be closed while accepted sockets are still live. The server presents its
certificate but does not request or verify a client certificate — there is no
mutual TLS, session resumption, ALPN, or SNI-based certificate selection in this
version.


`tls::read` returns a `List OF Byte`; `tls::write` accepts either a
`List OF Byte` or a `String`, whose UTF-8 bytes it sends directly. The asymmetry
is deliberate: a read stops wherever the network happened to divide the data,
which need not be a character boundary, so decoding at that point can split a
multi-byte character in half. Assemble the whole message first and decode it
with `encoding::utf8Decode`. Sending is not subject to that hazard, which is why
`write` does take a `String`.

Each read performs one underlying TLS read and returns as soon as any plaintext
is available, so a result is frequently shorter than `maxBytes` and never empty
on success; end of stream is reported as an error rather than an empty result,
so read in a loop until the connection is closed. Each write transmits the
entire buffer, looping internally to resend any portion a single TLS write did
not accept. TLS is
implemented on Linux by driving the system OpenSSL library (`libssl.so.3`,
falling back to `libssl.so.1.1`) so a single binary spans OpenSSL 1.1.1 and 3.x;
the macOS backend drives Network.framework through a synchronous bridge."#;

/// Register the `tls` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("tls", MODULE_INTRO, MODULE_DESC);

    // plan-110-D: the endpoint queries return net's shared `Address` record.
    pkg.add_imports(vec!["net"]);

    // The opaque `Socket` / `Listener` handles are semantic-only resources
    // (no injectable source): they make `registry().qualified_builtin_type` and
    // `registry::resource_close_function` answer generically, replacing the deleted
    // per-package `is_builtin_type`/`resource_close_function` seams. A `Socket`'s
    // close op is the public `tls.close`; a `Listener`'s is the internal
    // listener-shaped `tls.closeListener` scope-drop body (a user `tls::close` over a
    // `Listener` is rewritten to it during IR lowering).
    pkg.add_resource(RegistryResource {
        name: TLS_SOCKET_TYPE,
        export: true,
        description: "A connected TLS stream — an outbound client connection from \
                      `tls::connect` or an accepted server connection from `tls::accept` — \
                      closed automatically when its binding goes out of scope.",
        close_function: "tls.close",
        // Thread-sendable since bug-464. A session moves to another thread with
        // its handle; what is NOT allowed is two threads using one session at
        // once, and `thread::transfer` moves rather than shares (the sender is
        // tombstoned `moved|closed` and its cleanup deactivated). OpenSSL 1.1.1+
        // is thread-safe for distinct objects, so a session used by one thread at
        // a time is sound.
        sendable: true,
        // The live session state, per backend -- this is what the transfer copy
        // used to zero, and the real reason the bit was `false`.
        live_slots: &[
            ResourceLiveSlot {
                offset: gen_shared::TLS_OFFSET_CTX,
                transfer: SlotTransfer::Verbatim,
                backend: SlotBackend::OpenSsl,
                what: "SSL_CTX* (libssl malloc heap; 0 on an accepted socket, the \
                       marker that it borrows the listener's shared context)",
            },
            ResourceLiveSlot {
                offset: gen_shared::TLS_OFFSET_SSL,
                transfer: SlotTransfer::Verbatim,
                backend: SlotBackend::OpenSsl,
                what: "SSL* (libssl malloc heap)",
            },
            ResourceLiveSlot {
                offset: gen_shared::TLS_SCHANNEL_OFFSET_BLOCK,
                transfer: SlotTransfer::ArenaBlock {
                    size: gen_schannel::SOCKET_BLOCK_SIZE,
                },
                backend: SlotBackend::Schannel,
                what: "SSPI credential/context block -- an ARENA block, so it is \
                       copied into the receiver's arena, never aliased",
            },
            ResourceLiveSlot {
                offset: gen_macos::REC_CTX,
                transfer: SlotTransfer::Verbatim,
                backend: SlotBackend::NetworkFramework,
                what: "nw_connection ctx (also holds the CTX_RTO/CTX_WTO deadlines, \
                       which therefore ride this pointer)",
            },
            ResourceLiveSlot {
                offset: gen_macos::REC_QUEUE,
                transfer: SlotTransfer::Verbatim,
                backend: SlotBackend::NetworkFramework,
                what: "dispatch_queue_t (refcounted and thread-safe, so a move is sound). \
                       Zero on an accepted socket, which shares its listener's queue.",
            },
            // NOTE: `REC_LHOST`@48 is deliberately NOT listed here. It is
            // LISTENER-ONLY -- written by `tls::listen` (gen_macos/server.rs) and
            // by neither `tls::connect` (client.rs:628-630) nor `tls::accept`
            // (server.rs:1650-1652), both of which write only CTX and QUEUE. A
            // socket's word 48 is therefore uninitialised arena memory, and
            // declaring it live made the transfer copy run `strlen` over a
            // poisoned pointer: an immediate SIGSEGV on the first transferred
            // socket. Only ever declare a slot the resource's OWN constructors
            // write.
        ],
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });
    pkg.add_resource(RegistryResource {
        name: TLS_LISTENER_TYPE,
        export: true,
        description: "A bound, listening server endpoint from `tls::listen`, holding the \
                      loaded server TLS settings; `tls::accept` draws connections from it.",
        close_function: CLOSE_LISTENER,
        // Thread-sendable since bug-464: bind and load the identity on one
        // thread, accept on another. The listener still owns the server context
        // and still frees it exactly once, now on the receiving thread.
        sendable: true,
        live_slots: &[
            ResourceLiveSlot {
                offset: gen_shared::TLS_LISTENER_OFFSET_CTX,
                transfer: SlotTransfer::Verbatim,
                backend: SlotBackend::OpenSsl,
                what: "server SSL_CTX* (libssl malloc heap), owned and freed by the listener",
            },
            ResourceLiveSlot {
                offset: gen_shared::TLS_SCHANNEL_OFFSET_BLOCK,
                transfer: SlotTransfer::ArenaBlock {
                    size: gen_schannel::LISTENER_BLOCK_SIZE,
                },
                backend: SlotBackend::Schannel,
                what: "SSPI WORK block (credential, cert, key container) -- an ARENA \
                       block. Its stl::CONTNAME key-container name is derived from the \
                       ORIGINAL block pointer but stored as bytes, so the copy keeps \
                       naming the container that actually exists.",
            },
            ResourceLiveSlot {
                offset: gen_macos::REC_CTX,
                transfer: SlotTransfer::Verbatim,
                backend: SlotBackend::NetworkFramework,
                what: "nw_listener ctx",
            },
            ResourceLiveSlot {
                offset: gen_macos::REC_QUEUE,
                transfer: SlotTransfer::Verbatim,
                backend: SlotBackend::NetworkFramework,
                what: "dispatch_queue_t the listener and its connections are bound to",
            },
            ResourceLiveSlot {
                offset: gen_macos::REC_LHOST,
                transfer: SlotTransfer::ArenaCString,
                backend: SlotBackend::NetworkFramework,
                what: "bound-host C string for tls::localAddress -- an ARENA string",
            },
        ],
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    func_connect::register(&mut pkg);
    func_listen::register(&mut pkg);
    func_accept::register(&mut pkg);
    func_read::register(&mut pkg);
    func_write::register(&mut pkg);
    func_local_address::register(&mut pkg);
    func_remote_address::register(&mut pkg);
    func_set_read_timeout::register(&mut pkg);
    func_set_write_timeout::register(&mut pkg);
    func_poll::register(&mut pkg);
    func_close::register(&mut pkg);

    r.add_package(pkg);
}
