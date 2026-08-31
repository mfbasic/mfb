//! Shared BSD-socket / Winsock OS-seam primitives.
//!
//! plan-110-E Phase 3. Before the transport split these lived in
//! `builtins/net/{gen_shared,gen_poll}.rs`, because `net` was the only package
//! that owned a socket. plan-110-B/C/D split that surface across `tcp`, `udp`
//! and `tls`, which left the shared halves in an odd place: `udp` and `tls`
//! reaching into `net`'s emitters for pollfd construction and `setsockopt`
//! timeouts, long after `net` stopped having a socket of its own.
//!
//! What belongs here is exactly what MORE THAN ONE package needs:
//!
//! * the address builders (`emit_address_from_sockaddr`, the `net::Address`
//!   record shape) — used by `tcp`, `udp`, `tls` and by `net`'s own `lookup`
//!   and `ping`;
//! * pollfd construction and the readiness poll (`poll`/`WSAPoll`) — `tcp`,
//!   `udp`, `tls`;
//! * the `SO_RCVTIMEO`/`SO_SNDTIMEO` setter — `tcp`, `udp`, `tls`;
//! * the per-OS symbol table (`net_symbol`) and the shared constants, which
//!   are the reason a single body can serve libc and Winsock.
//!
//! What does NOT belong here is anything one package owns alone: `tcp`'s
//! connect/listen/accept/read/write and `udp`'s bind/receive/send live in
//! their own packages, and `net` keeps the resolver (`lookup`) and `ping`.
//!
//! The name is deliberately not `net`: this is OS-seam infrastructure, not a
//! language package, and putting it under `codegen::os` alongside `ffi` and
//! `syscall` is what stops `udp` from having to depend on `tcp` (or on `net`)
//! to build a `pollfd`.

pub(crate) mod poll;
pub(crate) mod shared;
