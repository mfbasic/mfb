//! OpenSSL `dlopen`/`dlsym` TLS backend: the socket-timeout connect/read/
//! write/close helpers and their OpenSSL machinery (see `super` for the
//! shared emit helpers and `macos` for the Network.framework backend).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use std::collections::HashMap;

use super::gen_shared::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::push_write_payload_view;
use crate::target::shared::abi;
pub(crate) fn lower_tls_connect_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    address: bool,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 256;
    const FD_OFFSET: usize = 8;
    const HANDLE_OFFSET: usize = 16;
    const CTX_OFFSET: usize = 24;
    const SSL_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;
    const HOST_OFFSET: usize = 48; // connect: host String ptr
    const PORT_OFFSET: usize = 56; // connect: port
    const SNAME_OFFSET: usize = 64; // serverName String ptr
    const HOSTCSTR_OFFSET: usize = 72;
    const SNICSTR_OFFSET: usize = 80;
    const RES_OFFSET: usize = 88; // addrinfo*
    const HINTS_OFFSET: usize = 96; // 96..144
    const TIMEOUT_OFFSET: usize = 144; // timeoutMs
    const FLAGS_OFFSET: usize = 152; // saved socket flags for non-blocking connect
    const POLLFD_OFFSET: usize = 160; // pollfd { fd; events; revents }
    const SOERR_OFFSET: usize = 168; // getsockopt SO_ERROR output
    const SOLEN_OFFSET: usize = 176; // getsockopt option length
    const TIMEVAL_OFFSET: usize = 184; // 184..200: tv_sec (8) + tv_usec (8)
    const HSTOFLAG: usize = 200; // plan-73-D: 1 if the handshake recv timed out (SO_*TIMEO)
    const ALLOW: usize = 208; // bug-477: allowSelfSigned (0/1)
    const VCB: usize = 216; // bug-477: SSL_set_verify's callback argument (0 or &cb)

    let resolve_fail = format!("{symbol}_resolve_fail");
    let net_fail = format!("{symbol}_net_fail");
    let net_fail_fd = format!("{symbol}_net_fail_fd");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let connect_invalid = format!("{symbol}_connect_invalid");
    let blocking_connect = format!("{symbol}_blocking_connect");
    let nb_connected = format!("{symbol}_nb_connected");
    let connected = format!("{symbol}_connected");
    let hs_timeout_set = format!("{symbol}_hs_timeout_set");
    let hs_timeout_clear = format!("{symbol}_hs_timeout_clear");
    let tls_fail = format!("{symbol}_tls_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let alloc_fail_raw = format!("{symbol}_alloc_fail_raw");
    let load_fail = format!("{symbol}_load_fail");
    let use_sname = format!("{symbol}_use_sname");
    let sni_ready = format!("{symbol}_sni_ready");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();

    // Host form: x0 = host; x1 = port; x2 = timeoutMs; x3 = serverName; x4 = allowSelfSigned.
    // Address form: x0 = net::Address; x1 = timeoutMs; x2 = serverName; x3 = allowSelfSigned.
    instructions.extend(super::gen_shared::connect_arg_prologue(
        address,
        &v9,
        HOST_OFFSET,
        PORT_OFFSET,
        TIMEOUT_OFFSET,
        SNAME_OFFSET,
        ALLOW,
    ));
    instructions.extend([
        // Sentinel-initialise the fd (-1) and the SSL/SSL_CTX slots (0) so the
        // alloc_fail exit can close/free exactly what has been acquired without
        // touching a garbage fd or object (bug-55).
        abi::move_immediate(&v9, "Integer", "0"),
        abi::bitwise_not(&v9, &v9),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SSL_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CTX_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTOFLAG),
    ]);
    {
        // plan-73-D: reject a negative (non-sentinel) `timeoutMs` up front — before
        // getaddrinfo/socket, so nothing leaks. The omitted overload pads the
        // unbounded sentinel (i64::MIN, allowed → the blocking connect + blocking
        // handshake below); `0`/`> 0` pass through.
        let ts_ok = format!("{symbol}_ts_ok");
        let ts_store = format!("{symbol}_ts_clamped");
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
            abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v9, &v10),
            abi::branch_eq(&ts_ok),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&connect_invalid),
            // Clamp `> 0` to INT_MAX and store it back: poll() and the handshake
            // SO_*TIMEO take a bounded C `int`, so a value with bit 31 set would be
            // read as a negative (block-forever) timeout (bug-239). net clamps the
            // same way (net/poll.rs); tls was missing it. The sentinel skips this
            // (branch_eq above) and stays in the slot as the block form.
            abi::move_immediate(&v10, "Integer", "2147483647"),
            abi::compare_registers(&v9, &v10),
            abi::branch_le(&ts_store),
            abi::move_register(&v9, &v10),
            abi::label(&ts_store),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
            abi::label(&ts_ok),
        ]);
    }
    // Resolve + connect a TCP socket. Zero a 48-byte hints block and set
    // ai_family = AF_INET, ai_socktype = SOCK_STREAM.
    for offset in (0..48).step_by(8) {
        instructions.push(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            HINTS_OFFSET + offset,
        ));
    }
    instructions.extend([
        abi::move_immediate(&v9, "Integer", HINTS_FAMILY_WORD),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET),
        abi::move_immediate(&v9, "Integer", SOCK_STREAM),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET + 8),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        HOSTCSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            HOSTCSTR_OFFSET,
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // socket(ai_family, ai_socktype, ai_protocol)
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u32(abi::return_register(), &v9, 4),
        abi::load_u32(abi::c_arg(1), &v9, 8),
        abi::load_u32(abi::c_arg(2), &v9, 12),
    ]);
    platform.emit_external_call(
        "socket",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `socket` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port (ai_addr + 2/3) with the requested port.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, addr_off),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 2),
        abi::store_u8(&v10, &v9, 3),
    ]);
    // plan-73-D. Connect the socket: the unbounded sentinel => a plain blocking
    // connect (omit = block); `0` => a non-blocking connect + poll(0) (one immediate
    // attempt → `ErrTimeout` unless it completes at once); `> 0` => non-blocking
    // connect + poll(timeoutMs). Negatives were rejected up front. Mirrors
    // net::connectTcp. DNS (getaddrinfo above) is not bounded by timeoutMs.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&blocking_connect),
        // flags = fcntl(fd, F_GETFL, 0)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "3"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_variadic_external_call(
        "fcntl",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FLAGS_OFFSET,
    ));
    // fcntl(fd, F_SETFL, flags | O_NONBLOCK) — Windows: ioctlsocket(fd, FIONBIO, &1)
    platform.emit_set_nonblocking(
        FD_OFFSET,
        FLAGS_OFFSET,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // connect(fd, ai_addr, ai_addrlen)
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(1), &v9, addr_off),
        abi::load_u32(abi::c_arg(2), &v9, 16),
    ]);
    platform.emit_external_call(
        "connect",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&nb_connected),
    ]);
    // In progress? Anything other than EINPROGRESS is a hard failure.
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, platform.socket_in_progress_code()),
        abi::branch_ne(&net_fail_fd),
        // poll(&pollfd { fd, POLLOUT }, 1, timeoutMs)
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(&v10, "Integer", "4"), // POLLOUT
        abi::store_u8(&v10, abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `poll` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::branch_eq(&connect_timeout),
        // getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &len)
        abi::move_immediate(&v9, "Integer", "4"),
        abi::store_u64(&v9, abi::stack_pointer(), SOLEN_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SOERR_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
        abi::move_immediate(abi::c_arg(2), "Integer", platform.so_error()),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), SOERR_OFFSET),
        abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), SOLEN_OFFSET),
    ]);
    platform.emit_external_call(
        "getsockopt",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `getsockopt` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::load_u32(&v9, abi::stack_pointer(), SOERR_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&net_fail_fd),
        // Connected: restore blocking mode with fcntl(fd, F_SETFL, flags).
        abi::label(&nb_connected),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "4"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), FLAGS_OFFSET),
    ]);
    platform.emit_variadic_external_call(
        "fcntl",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&connected),
        // Blocking connect path (timeoutMs <= 0).
        abi::label(&blocking_connect),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(1), &v9, addr_off),
        abi::load_u32(abi::c_arg(2), &v9, 16),
    ]);
    platform.emit_external_call(
        "connect",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `connect` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::label(&connected),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // plan-73-D. Bound the blocking TLS handshake by timeoutMs (SO_RCVTIMEO/
    // SO_SNDTIMEO), cleared again after the handshake so read/write stay unbounded.
    // The unbounded sentinel => leave the handshake unbounded (omit = block); `0` =>
    // the smallest nonzero wait (tv_usec = 1µs, near-immediate `ErrTimeout` — a
    // SO_*TIMEO of {0,0} is *infinite*, so 0 cannot be literal); `> 0` => the timeval.
    let hs_ts_ok = format!("{symbol}_hs_ts_ok");
    instructions.extend([
        abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v15, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v14, &v15),
        abi::branch_eq(&hs_timeout_set),
        // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
        abi::move_immediate(&v10, "Integer", "1000"),
        abi::unsigned_divide_registers(&v11, &v14, &v10),
        abi::multiply_subtract_registers(&v12, &v11, &v10, &v14),
        abi::move_immediate(&v13, "Integer", "1000"),
        abi::multiply_registers(&v12, &v12, &v13),
        // 0 => bump tv_usec to 1µs so the handshake is non-blocking, not infinite.
        abi::compare_immediate(&v14, "0"),
        abi::branch_ne(&hs_ts_ok),
        abi::move_immediate(&v12, "Integer", "1"),
        abi::label(&hs_ts_ok),
        abi::store_u64(&v11, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64(&v12, abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        FD_OFFSET,
        TIMEVAL_OFFSET,
    )?;
    instructions.push(abi::label(&hs_timeout_set));
    // SNI/validation name = serverName if non-empty, else host.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SNAME_OFFSET),
        abi::load_u64(&v10, &v9, 0),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&use_sname),
    ]);
    emit_cstring(
        symbol,
        "snihost",
        HOST_OFFSET,
        SNICSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.push(abi::branch(&sni_ready));
    instructions.push(abi::label(&use_sname));
    emit_cstring(
        symbol,
        "sni",
        SNAME_OFFSET,
        SNICSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.push(abi::label(&sni_ready));

    // --- OpenSSL handshake (shared) ---
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    // method = TLS_client_method(); stash transiently in the CTX slot.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "TLS_client_method",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // ctx = SSL_CTX_new(method)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_new",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // SSL_CTX_set_default_verify_paths(ctx) -- best effort, ignore result.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_set_default_verify_paths",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // ssl = SSL_new(ctx)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_new",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), SSL_OFFSET),
    ]);
    // SSL_set_fd(ssl, fd)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set_fd",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // bug-477: the mode stays SSL_VERIFY_PEER in BOTH forms — only the callback
    // argument changes. NULL is today's behaviour (abort on the first chain
    // error); with `allowSelfSigned` set it is `_mfb_tls_verify_cb`, which
    // forgives the three trust-anchor codes and returns 1 so verification
    // CONTINUES into the hostname and validity-date checks. Dropping to
    // SSL_VERIFY_NONE instead — the shape the bug document originally proposed —
    // was measured to make a name-mismatched self-signed certificate
    // indistinguishable from a name-correct one (both report 18), which is why
    // this is a callback and not a mode change.
    // The callback argument is staged into a FRAME SLOT, not into `c_arg(2)`,
    // and only loaded into the register after the last `dlsym` below. Every
    // `emit_dlsym` is a C call and a C call clobbers the argument registers, so
    // writing `c_arg(2)` here and calling `dlsym` afterwards destroys it —
    // measured on box 2223, where the handshake never started and the peer
    // logged "0 server accepts". See `.ai/arch-abi.md`, "Stage ABI args via
    // temporaries".
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), VCB));
    {
        let strict = format!("{symbol}_verify_strict");
        instructions.extend([
            abi::load_u64(&v10, abi::stack_pointer(), ALLOW),
            abi::compare_immediate(&v10, "0"),
            abi::branch_eq(&strict),
        ]);
        // Publish the two X509_STORE_CTX accessors the callback needs. It receives
        // no user pointer, so they travel through a process-global slot pair
        // rather than a capture; concurrent connects all store the same values.
        for (name, slot) in [
            ("X509_STORE_CTX_get_error", VFN_GET_ERROR),
            ("X509_STORE_CTX_set_error", VFN_SET_ERROR),
        ] {
            emit_dlsym(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                HANDLE_OFFSET,
                name,
                FNPTR_OFFSET,
                &load_fail,
            )?;
            emit_data_address(
                symbol,
                &v10,
                TLS_VERIFY_FNS,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
                abi::store_u64(&v9, &v10, slot),
            ]);
        }
        emit_data_address(
            symbol,
            &v10,
            TLS_VERIFY_CB,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::store_u64(&v10, abi::stack_pointer(), VCB));
        instructions.push(abi::label(&strict));
    }
    // SSL_set_verify's own fnptr is re-resolved here because the loop above
    // reused FNPTR_OFFSET for the two accessors.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set_verify",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_VERIFY_PEER),
        // Loaded HERE, after the last dlsym, for the reason given at VCB.
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), VCB),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // SSL_set1_host(ssl, sniCstr) — verifies the peer certificate against DNS-name
    // SANs/CN only. TLS to an IP literal is therefore UNSUPPORTED: matching an
    // `iPAddress` SAN needs X509_VERIFY_PARAM_set1_ip (a separate libssl symbol)
    // driven by a runtime numeric-host check, so an IP-literal connection fails
    // verification and closes rather than validating by IP. This fails *closed*
    // (over-strict), never open — there is no verification bypass (bug-177 C).
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set1_host",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SNICSTR_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // SSL_ctrl(ssl, SSL_CTRL_SET_TLSEXT_HOSTNAME, TLSEXT_NAMETYPE_host_name, sniCstr) -- SNI
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_ctrl",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_CTRL_SET_TLSEXT_HOSTNAME),
        abi::move_immediate(abi::c_arg(2), "Integer", TLSEXT_NAMETYPE_HOST_NAME),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), SNICSTR_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_ctrl(ssl, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_CTRL_SET_MIN_PROTO_VERSION),
        abi::move_immediate(abi::c_arg(2), "Integer", TLS1_2_VERSION),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // Require the TLS 1.2 floor to have been set (returns 1 on success),
        // matching the checked SSL_set1_host / SSL_connect / verify-result calls
        // — an unchecked failure would silently permit a downgrade (bug-55).
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // r = SSL_connect(ssl); require 1.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_connect",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    let hs_connected = format!("{symbol}_hs_connected");
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_eq(&hs_connected),
    ]);
    // plan-73-D: SSL_connect failed. If the handshake recv hit the SO_RCVTIMEO we
    // installed (errno == EWOULDBLOCK/EAGAIN), classify it as a TIMEOUT (ErrTimeout),
    // matching the macOS backend and the connect-poll timeout, rather than the
    // generic ErrTlsFailed. Any other failure stays ErrTlsFailed.
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, platform.socket_would_block_code()),
        abi::branch_ne(&tls_fail),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::branch(&tls_fail),
        abi::label(&hs_connected),
    ]);
    // v = SSL_get_verify_result(ssl); require X509_V_OK (0).
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_get_verify_result",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&tls_fail),
    ]);
    // Handshake done: clear SO_*TIMEO (zero timeval) so read/write are unbounded.
    // plan-73-D: only the sentinel (omit) left the handshake unbounded; `0`/`> 0`
    // installed a SO_*TIMEO that must be cleared here.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&hs_timeout_clear),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        FD_OFFSET,
        TIMEVAL_OFFSET,
    )?;
    instructions.push(abi::label(&hs_timeout_clear));
    // Build the Socket record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the TLS tail { ssl, ctx } (plan-80).
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_OPENSSL),
        abi::store_u64(&v9, abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_SSL),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    instructions.push(abi::label(&tls_fail));
    // Free the SSL session and per-connection SSL_CTX before closing the fd.
    // tls_fail is branched to from SSL_new onward — SSL_set_fd, SSL_set1_host,
    // the min-proto ctrl, SSL_connect and SSL_get_verify_result — at every one
    // of which this frame owns both objects. It used to close only the fd, so a
    // client reconnect loop against an expired- or untrusted-cert host leaked
    // one SSL + one SSL_CTX (several KB of OpenSSL heap) per failure, while the
    // sibling alloc_fail and the accept-side ssl_fail freed them (bug-317 T2).
    // Slots are sentinel-initialised to 0, so both frees are null-guarded and a
    // missing symbol falls through to tls_fail_raw, still reporting the TLS
    // failure rather than masking it as a load error.
    let tls_fail_raw = format!("{symbol}_tls_fail_raw");
    let tf_skip_ssl = format!("{symbol}_tf_skip_ssl");
    let tf_skip_ctx = format!("{symbol}_tf_skip_ctx");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&tf_skip_ssl),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &tls_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::label(&tf_skip_ssl),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&tf_skip_ctx),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &tls_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&tf_skip_ctx));
    instructions.push(abi::label(&tls_fail_raw));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // plan-73-D: a handshake recv that hit the installed SO_*TIMEO is a timeout
    // (ErrTimeout), everything else on this path is a TLS failure (ErrTlsFailed).
    let tls_fail_timeout = format!("{symbol}_tls_fail_timeout");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&tls_fail_timeout),
    ]);
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&tls_fail_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.push(abi::label(&load_fail));
    // Every dlopen/dlsym on the libssl handshake path runs after the TCP socket is
    // already connected, so close the fd (guarded fd >= 0, mirroring alloc_fail's
    // close) before failing — otherwise a near-fatal OpenSSL-missing environment
    // leaks the connected socket (bug-177 B).
    let lf_skip_fd = format!("{symbol}_lf_skip_fd");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&lf_skip_fd),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&lf_skip_fd));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.push(abi::label(&net_fail_fd));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&net_fail));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_external_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // The TCP connect did not complete before timeoutMs: close the pending
    // socket, release the resolver results, and report a timeout.
    instructions.push(abi::label(&connect_timeout));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_external_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument. Reached
    // from the up-front check before getaddrinfo/socket, so nothing to clean up.
    instructions.push(abi::label(&connect_invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        "ErrAddressNotFound",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    // Free the SSL session and context and close the socket the aborted record
    // would have owned. The tls_fail/net_fail_fd exits close the fd, but a
    // post-handshake record-alloc OOM otherwise leaked fd + SSL + SSL_CTX
    // (bug-55). Slots are sentinel-initialised (fd = -1, SSL/CTX = 0), so each
    // step is null/-1-guarded and the frees' dlsym only runs once the object
    // exists (libssl loaded). dlsym failures route to alloc_fail_raw.
    let af_skip_ssl = format!("{symbol}_af_skip_ssl");
    let af_skip_ctx = format!("{symbol}_af_skip_ctx");
    let af_skip_fd = format!("{symbol}_af_skip_fd");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&af_skip_ssl),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &alloc_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::label(&af_skip_ssl),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&af_skip_ctx),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &alloc_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::label(&af_skip_ctx),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&af_skip_fd),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&af_skip_fd));
    instructions.push(abi::label(&alloc_fail_raw));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.listen
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_listen_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    const FRAME_SIZE: usize = 224;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const CERT_OFFSET: usize = 24;
    const KEY_OFFSET: usize = 32;
    const BACKLOG_OFFSET: usize = 40;
    const RES_OFFSET: usize = 48; // addrinfo*
    const FD_OFFSET: usize = 56;
    const HOSTCSTR_OFFSET: usize = 64;
    const ONE_OFFSET: usize = 72; // SO_REUSEADDR option value
    const CERTCSTR_OFFSET: usize = 80;
    const KEYCSTR_OFFSET: usize = 88;
    const HANDLE_OFFSET: usize = 96;
    const FNPTR_OFFSET: usize = 104;
    const CTX_OFFSET: usize = 112;
    const HINTS_OFFSET: usize = 128; // 128..176

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let ctx_fail = format!("{symbol}_ctx_fail");
    let tls_fail_fd = format!("{symbol}_tls_fail_fd");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let alloc_fail_fd = format!("{symbol}_alloc_fail_fd");
    let alloc_fail_ctx_fd = format!("{symbol}_alloc_fail_ctx_fd");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();

    // x0 = host; x1 = port; x2 = certPath; x3 = keyPath; x4 = backlog.
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT_OFFSET),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CERT_OFFSET),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), KEY_OFFSET),
        abi::store_u64(abi::c_arg(4), abi::stack_pointer(), BACKLOG_OFFSET),
    ]);
    // Zero a 48-byte hints block; ai_flags = AI_PASSIVE, ai_family = AF_INET,
    // ai_socktype = SOCK_STREAM — the bind/listen resolution mirrors
    // net::listenTcp (an empty host binds all interfaces via a NULL node).
    for offset in (0..48).step_by(8) {
        instructions.push(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            HINTS_OFFSET + offset,
        ));
    }
    instructions.extend([
        abi::move_immediate(&v9, "Integer", HINTS_FAMILY_WORD_PASSIVE),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET),
        abi::move_immediate(&v9, "Integer", SOCK_STREAM),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET + 8),
        // Empty host => NULL node (all interfaces).
        abi::load_u64(&v9, abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64(&v9, &v9, 0),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&null_host),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        HOSTCSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        abi::branch(&resolved),
        abi::label(&null_host),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HOSTCSTR_OFFSET),
        abi::label(&resolved),
        // getaddrinfo(host, NULL, &hints, &res)
        abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            HOSTCSTR_OFFSET,
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // socket(ai_family, ai_socktype, ai_protocol)
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u32(abi::return_register(), &v9, 4),
        abi::load_u32(abi::c_arg(1), &v9, 8),
        abi::load_u32(abi::c_arg(2), &v9, 12),
    ]);
    platform.emit_external_call(
        "socket",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `socket` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port (ai_addr + 2/3) with the requested port.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, addr_off),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 2),
        abi::store_u8(&v10, &v9, 3),
        // setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, 4) - best effort.
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), ONE_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
        abi::move_immediate(abi::c_arg(2), "Integer", platform.so_reuseaddr()),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), ONE_OFFSET),
        abi::move_immediate(abi::c_arg(4), "Integer", "4"),
    ]);
    platform.emit_external_call(
        "setsockopt",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // bind(fd, ai_addr, ai_addrlen)
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(1), &v9, addr_off),
        abi::load_u32(abi::c_arg(2), &v9, 16),
    ]);
    platform.emit_external_call(
        "bind",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `bind` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
        // listen(fd, backlog)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BACKLOG_OFFSET),
    ]);
    platform.emit_external_call(
        "listen",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `listen` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // --- Server identity: cert chain + private key into a server SSL_CTX ---
    emit_cstring(
        symbol,
        "cert",
        CERT_OFFSET,
        CERTCSTR_OFFSET,
        &alloc_fail_fd,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    emit_cstring(
        symbol,
        "key",
        KEY_OFFSET,
        KEYCSTR_OFFSET,
        &alloc_fail_fd,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &tls_fail_fd,
    )?;
    // method = TLS_server_method(); stash transiently in the CTX slot.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "TLS_server_method",
        FNPTR_OFFSET,
        &tls_fail_fd,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // ctx = SSL_CTX_new(method)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_new",
        FNPTR_OFFSET,
        &tls_fail_fd,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail_fd),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // SSL_CTX_ctrl(ctx, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, NULL)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_ctrl",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_CTRL_SET_MIN_PROTO_VERSION),
        abi::move_immediate(abi::c_arg(2), "Integer", TLS1_2_VERSION),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // Require the TLS 1.2 floor to have been set (returns 1 on success),
        // matching the checked identity-loading calls below; ctx_fail frees the
        // context and closes the fd — an unchecked failure would silently permit
        // a downgrade (bug-55).
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // SSL_CTX_use_certificate_chain_file(ctx, certCstr) == 1
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_use_certificate_chain_file",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CERTCSTR_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // SSL_CTX_use_PrivateKey_file(ctx, keyCstr, SSL_FILETYPE_PEM = 1) == 1
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_use_PrivateKey_file",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), KEYCSTR_OFFSET),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // SSL_CTX_check_private_key(ctx) == 1
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_check_private_key",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // Build the Listener record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the TLS tail { ctx } (plan-80).
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    // The server SSL_CTX already exists here, so an OOM must free it before
    // reporting: `alloc_fail_fd` closes the fd but leaked the context (bug-236).
    // It cannot gain an SSL_CTX_free itself — the pre-ctx cstring allocs share it
    // and the CTX slot is not live for them (the bug-201 class) — so route to a
    // dedicated ctx-freeing OOM exit that falls into it. connect/accept already do
    // full SSL/CTX/fd cleanup on this same OOM class (bug-55).
    emit_alloc(
        symbol,
        &mut instructions,
        &mut relocations,
        &alloc_fail_ctx_fd,
    );
    instructions.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_LISTENER),
        abi::store_u64(&v9, abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_LISTENER_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_LISTENER_OFFSET_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_LISTENER_OFFSET_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    // ctx_fail: the server context exists but the identity failed to load —
    // free the context, close the fd, and report ErrTlsFailed.
    instructions.push(abi::label(&ctx_fail));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &tls_fail_fd,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&tls_fail_fd));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // bind/listen failure: close the fd, release the resolver results.
    instructions.push(abi::label(&op_fail));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&socket_fail));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_external_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // The record-alloc OOM exit taken once the server SSL_CTX exists: free the
    // context, then fall through into the shared fd-close + ErrOutOfMemory report
    // (bug-236). A `dlsym` miss for SSL_CTX_free still reports the OOM.
    instructions.push(abi::label(&alloc_fail_ctx_fd));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &alloc_fail_fd,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&alloc_fail_fd));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.accept
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_accept_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 96;
    const FD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    const CONNFD_OFFSET: usize = 24;
    const CTX_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;
    const SSL_OFFSET: usize = 56;
    const POLLFD_OFFSET: usize = 64; // pollfd { fd; events; revents }
    const TIMEVAL_OFFSET: usize = 72; // timeval { tv_sec; tv_usec } (bug-202)
    const HSTOFLAG: usize = 88; // plan-73-D: 1 if the handshake recv timed out

    let closed = format!("{symbol}_closed");
    let no_timeout = format!("{symbol}_no_timeout");
    let hs_timeout_set = format!("{symbol}_hs_timeout_set");
    let hs_timeout_cleared = format!("{symbol}_hs_timeout_cleared");
    let accept_fail = format!("{symbol}_accept_fail");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let accept_invalid = format!("{symbol}_accept_invalid");
    let accept_ts_store = format!("{symbol}_accept_ts_clamped");
    let ssl_fail = format!("{symbol}_ssl_fail");
    let tls_fail_conn = format!("{symbol}_tls_fail_conn");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    // x0 = listener record { fd@0, closed@8, ctx@16 }; x1 = timeoutMs.
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTOFLAG),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        // plan-73-D: the unbounded sentinel => a blocking accept (omit = block); `0`
        // => poll(POLLIN, 0), one immediate attempt (`ErrTimeout` if none pending);
        // `> 0` => poll(POLLIN, timeoutMs); a negative (non-sentinel) => invalid.
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&no_timeout),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&accept_invalid),
        // Clamp `> 0` to INT_MAX and store it back — poll() takes a C `int`, so a
        // value with bit 31 set would be read as a block-forever timeout (bug-239);
        // net clamps identically. The sentinel skips this (branch_eq above).
        abi::move_immediate(&v10, "Integer", "2147483647"),
        abi::compare_registers(&v9, &v10),
        abi::branch_le(&accept_ts_store),
        abi::move_register(&v9, &v10),
        abi::label(&accept_ts_store),
        abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"), // POLLIN
        abi::store_u8(&v10, abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `poll` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::branch_eq(&accept_timeout),
        abi::label(&no_timeout),
        // accept(fd, NULL, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "accept",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `accept` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONNFD_OFFSET),
    ]);
    // --- Server-side handshake on the accepted fd ---
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &tls_fail_conn,
    )?;
    // ssl = SSL_new(listener.ctx)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_new",
        FNPTR_OFFSET,
        &tls_fail_conn,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail_conn),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), SSL_OFFSET),
    ]);
    // SSL_set_fd(ssl, connfd)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set_fd",
        FNPTR_OFFSET,
        &ssl_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CONNFD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ssl_fail),
    ]);
    // Bound the blocking server handshake by timeoutMs (SO_RCVTIMEO/SO_SNDTIMEO on
    // the accepted connfd), mirroring the connect handshake wrapping. `timeoutMs`
    // previously bounded only the connection-wait poll above, so a client that
    // completed the TCP handshake then stalled mid-TLS wedged SSL_accept — and the
    // single-threaded accept loop with it — forever (bug-202). Cleared after the
    // handshake so the socket's reads/writes stay unbounded.
    // plan-73-D: sentinel => leave the handshake unbounded (omit = block); `0` =>
    // the smallest nonzero wait (tv_usec 1µs); `> 0` => the timeval.
    let hs_ts_ok = format!("{symbol}_hs_ts_ok");
    instructions.extend([
        abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v15, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v14, &v15),
        abi::branch_eq(&hs_timeout_set),
        // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
        abi::move_immediate(&v10, "Integer", "1000"),
        abi::unsigned_divide_registers(&v11, &v14, &v10),
        abi::multiply_subtract_registers(&v12, &v11, &v10, &v14),
        abi::move_immediate(&v13, "Integer", "1000"),
        abi::multiply_registers(&v12, &v12, &v13),
        // 0 => tv_usec = 1µs (non-blocking, not infinite).
        abi::compare_immediate(&v14, "0"),
        abi::branch_ne(&hs_ts_ok),
        abi::move_immediate(&v12, "Integer", "1"),
        abi::label(&hs_ts_ok),
        abi::store_u64(&v11, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64(&v12, abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        CONNFD_OFFSET,
        TIMEVAL_OFFSET,
    )?;
    instructions.push(abi::label(&hs_timeout_set));
    // r = SSL_accept(ssl); require 1 (server handshake complete).
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_accept",
        FNPTR_OFFSET,
        &ssl_fail,
    )?;
    let asc_ok = format!("{symbol}_asc_ok");
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_eq(&asc_ok),
    ]);
    // plan-73-D: SSL_accept failed — an SO_RCVTIMEO expiry (errno EWOULDBLOCK/EAGAIN)
    // is a handshake TIMEOUT (ErrTimeout), matching the accept-poll timeout and the
    // other backends; anything else is a TLS failure.
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, platform.socket_would_block_code()),
        abi::branch_ne(&ssl_fail),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::branch(&ssl_fail),
        abi::label(&asc_ok),
    ]);
    // Handshake done: clear the timeouts so the returned socket blocks normally.
    // plan-73-D: only the sentinel (omit) left the handshake unbounded, so only it
    // skips the clear; `0`/`> 0` installed a SO_*TIMEO that must be cleared.
    instructions.extend([
        abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v15, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v14, &v15),
        abi::branch_eq(&hs_timeout_cleared),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        CONNFD_OFFSET,
        TIMEVAL_OFFSET,
    )?;
    instructions.push(abi::label(&hs_timeout_cleared));
    // Build the Socket record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the TLS tail { ssl, ctx = 0 } (plan-80) — the zero ctx slot marks a
    // non-owned (listener-owned) server context, which the close helper must not
    // free (plan-06-tls-server.md §6.4).
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_OPENSSL),
        abi::store_u64(&v9, abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64(&v9, abi::stack_pointer(), CONNFD_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_SSL),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    // ssl_fail: free the session, then close the accepted fd.
    instructions.push(abi::label(&ssl_fail));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &tls_fail_conn,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&tls_fail_conn));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        CONNFD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // plan-73-D: a handshake recv that hit the SO_*TIMEO is a timeout; else TLS fail.
    let accept_hs_timeout = format!("{symbol}_accept_hs_timeout");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&accept_hs_timeout),
    ]);
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&accept_hs_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&accept_fail));
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&accept_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument. Reached
    // from the up-front selector before any accept/alloc, so nothing to clean up.
    instructions.push(abi::label(&accept_invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    // The record alloc is the final step; the SSL session and the accepted fd
    // are both live, so free/close them before failing — a record-alloc OOM
    // otherwise leaked the SSL session and the accepted socket fd (bug-55).
    // Both are always set on the only path here. SSL_free's dlsym failure (only
    // if libssl vanished) routes to tls_fail_conn, which still closes the fd.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &tls_fail_conn,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD_OFFSET),
    ]);
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.read (bytes only; plan-110-D removed tls.readText)
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_read_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    const FRAME_SIZE: usize = 96;
    const SSL_OFFSET: usize = 8;
    const MAX_OFFSET: usize = 16;
    const BUF_OFFSET: usize = 24;
    const N_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let read_fail = format!("{symbol}_read_fail");
    let read_ok = format!("{symbol}_read_ok");
    let read_timeout = format!("{symbol}_read_timeout");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_le(&invalid),
        // Allocate a maxBytes read buffer.
        abi::move_register(abi::return_register(), &v10),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        BUF_OFFSET,
    ));
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_read",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    // n = SSL_read(ssl, buf, maxBytes)
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_read returns a C int; sign-extend before the signed 0/<0 tests so a
        // -1 error isn't read as a large positive byte count (bug-102).
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        // Spill `n` BEFORE the branches. Both successors need it -- the success
        // path as the byte count, the failure path as `SSL_get_error`'s second
        // argument -- and spilling once here means nothing downstream reads the
        // aligned bank across an intervening external call. The `read_ok` label
        // is emitted after the classification block below, so a linear scan from
        // that block's `blr` would otherwise reach a `str_u64 rdi` that is only
        // ever reached by the `b.gt` above it. The scan is what
        // `assert_no_aligned_bank_result_reads` does (bug-452), and rather than
        // teach it control flow the value is simply not left live in `rdi`.
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&peer_closed),
        abi::branch_gt(&read_ok),
    ]);
    // plan-110-D: distinguish the socket's read deadline from a transport or
    // protocol failure. `tls::setReadTimeout` installs SO_RCVTIMEO; when it
    // expires the underlying `read(2)` returns EAGAIN/EWOULDBLOCK, which
    // OpenSSL surfaces as SSL_ERROR_WANT_READ (2) or SSL_ERROR_WANT_WRITE (3)
    // — the session is intact and the call may simply be retried. The contract
    // says a deadline raises ErrTimeout on every platform; without this the
    // Linux backend reported ErrTlsFailed while macOS reported ErrTimeout for
    // the identical event, and a caller could not tell a slow peer from a
    // broken session.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_get_error",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), N_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::sign_extend_word(&v10, abi::c_return(0)),
        abi::compare_immediate(&v10, "2"), // SSL_ERROR_WANT_READ
        abi::branch_eq(&read_timeout),
        abi::compare_immediate(&v10, "3"), // SSL_ERROR_WANT_WRITE
        abi::branch_eq(&read_timeout),
        abi::branch(&read_fail),
        abi::label(&read_ok),
    ]);
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
        abi::move_immediate(&v11, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v12, &v10, &v11),
        abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &v12, &v10),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_immediate(&v9, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KIND),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&v9, "Byte", "1"),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_COUNT),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&v11, abi::mfb_return(1), COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v12, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::add_registers(&v14, &v11, &v13),
        abi::load_u64(&v15, abi::stack_pointer(), BUF_OFFSET),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&entry_done),
        // A packed fixed-width byte list (kind 2, plan-57-D) has NO entry
        // array: its stride is 0 and the data region starts right after the
        // header. Writing 40-byte entry descriptors here put an entry-flags
        // byte (COLLECTION_ENTRY_FLAG_USED == 1) where element 0's payload
        // belongs, so `tls::read` handed back a list whose first byte was
        // always 1. Mirrors the guard in `gen_macos/client.rs`.
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::move_immediate(&v12, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(&v12, &v11, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64(&v9, &v11, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(&v12, "Integer", "1"),
            abi::store_u64(&v12, &v11, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        ]);
    }
    // The payload copy runs for BOTH representations.
    instructions.extend([
        abi::add_registers(&v12, &v14, &v9),
        abi::load_u8(&v13, &v15, 0),
        abi::store_u8(&v13, &v12, 0),
        abi::add_immediate(&v15, &v15, 1),
        abi::add_immediate(&v11, &v11, byte_list_entry_stride()),
        abi::add_immediate(&v9, &v9, 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    instructions.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        "ErrConnectionClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&read_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&read_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
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
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.poll  (plan-76-B: tls::poll(sock[, timeoutMs]) AS Boolean)
// ---------------------------------------------------------------------------

/// TLS readiness on OpenSSL. `readable = SSL_pending(ssl) > 0 OR poll(fd, POLLIN)`:
/// the `SSL_pending` fast-path catches decrypted app bytes already buffered in the
/// TLS layer with the fd idle (which an fd-only poll would miss), and the `poll(2)`
/// fallback carries the plan-73 timeout (sentinel→block, `<0`→invalid, `>0`→clamp
/// `INT_MAX`, EINTR-retry — the `net::poll` policy). `x0` = sock record, `x1` =
/// timeoutMs. Returns `Boolean`.
pub(crate) fn lower_tls_poll_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 64;
    const TIMEOUT_OFFSET: usize = 8;
    const SSL_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const HANDLE_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;
    const POLLFD_OFFSET: usize = 48; // pollfd { fd; events; revents }

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let load_fail = format!("{symbol}_load_fail");
    let poll_infinite = format!("{symbol}_poll_infinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let ready = format!("{symbol}_ready");
    let not_ready = format!("{symbol}_not_ready");
    let poll_retry = format!("{symbol}_poll_retry");
    let poll_fail = format!("{symbol}_poll_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        // Save the fd now — the record pointer in x0 is clobbered by the SSL_pending
        // call below, so the fd for the poll fallback must come from a stack slot.
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        // Normalize the timeout (net::poll policy): sentinel→-1 (block), <0→invalid,
        // >0→clamp INT_MAX. Store the effective value back.
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&poll_infinite),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::move_immediate(&v10, "Integer", "2147483647"),
        abi::compare_registers(&v9, &v10),
        abi::branch_le(&timeout_ok),
        abi::move_register(&v9, &v10),
        abi::branch(&timeout_ok),
        abi::label(&poll_infinite),
        abi::bitwise_not(&v9, abi::ZERO),
        abi::label(&timeout_ok),
        abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    // SSL_pending fast-path: buffered decrypted bytes => readable now (skip poll).
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_pending",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_pending returns a C int count; sign-extend before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&ready),
        // No buffered bytes: poll the raw fd (reloaded from its stack slot, since the
        // record pointer in x0 was clobbered by the SSL_pending call). Build
        // pollfd{ fd, POLLIN }.
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"), // POLLIN
        abi::store_u8(&v10, abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
        abi::label(&poll_retry),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_fail),
        abi::branch_eq(&not_ready),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_fail),
    ]);
    // EINTR → re-issue the poll; any other errno → resource-closed (the readiness
    // check failed, matching net::poll's hard-error class).
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, "4"), // EINTR
        abi::branch_eq(&poll_retry),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, FRAME_SIZE))
}

// ---------------------------------------------------------------------------
// tls.write / tls.writeText
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_write_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v14 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 80;
    const SSL_OFFSET: usize = 8;
    const SRC_OFFSET: usize = 16;
    const REMAINING_OFFSET: usize = 24;
    const HANDLE_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;
    // bug-467: `SSL_write`'s return, spilled before the failure branch so the
    // classification below can hand it to `SSL_get_error` without reading the
    // aligned result bank across an external call (bug-452, and the same reason
    // `lower_tls_read_openssl` spills its `n`).
    const N_OFFSET: usize = 48;

    let closed = format!("{symbol}_closed");
    let load_fail = format!("{symbol}_load_fail");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_fail = format!("{symbol}_write_fail");
    let write_classify = format!("{symbol}_write_classify");
    let write_timeout = format!("{symbol}_write_timeout");
    let peer_closed = format!("{symbol}_peer_closed");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
    ]);
    // bug-497 / bug-508: one payload view for every backend — the text form
    // as before, the byte form after a header check (`push_write_payload_view`).
    let bad_payload = format!("{symbol}_bad_payload");
    push_write_payload_view(
        &mut instructions,
        text,
        abi::c_arg(1),
        &v10,
        &v11,
        &v14,
        &v12,
        &v13,
        REMAINING_OFFSET,
        SRC_OFFSET,
        &bad_payload,
    );
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_write",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::label(&write_loop),
        abi::load_u64(&v10, abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&write_done),
        // n = SSL_write(ssl, src, remaining)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SRC_OFFSET),
        abi::move_register(abi::c_arg(2), &v10),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_write returns a C int; sign-extend before the signed <=0 test (bug-102).
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_classify),
        abi::load_u64(&v11, abi::stack_pointer(), SRC_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers(&v11, &v11, abi::return_register()),
        abi::subtract_registers(&v10, &v10, abi::return_register()),
        abi::store_u64(&v11, abi::stack_pointer(), SRC_OFFSET),
        abi::store_u64(&v10, abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // bug-467: classify the failure instead of collapsing every one of them into
    // `ErrTlsFailed`. Until this bug, a write to a peer that had gone away never
    // got here at all -- libssl's internal `write(2)` delivered SIGPIPE and the
    // process died -- so the only reachable failures were load/protocol ones and
    // one blanket code was enough. With SIGPIPE ignored the peer's disappearance
    // now arrives here as a return value, and `tcp` and `tls` are documented
    // drop-in mirrors: `tcp::write` raises `ErrConnectionClosed` for it, and
    // `tls::read`/`tcp::read` already agree on that code at end of stream
    // (bug-465, rt-behavior/{tcp,tls}/*-read-eof-raises-rt). `SSL_get_error` is
    // the only way to tell the three cases apart:
    //
    //   2/3  WANT_READ / WANT_WRITE -- the SO_SNDTIMEO deadline `tls::setWriteTimeout`
    //        installs expired. The session is intact; the convention is ErrTimeout
    //        (plan-110-D). Letting this fall into the closed-connection arm would
    //        report a slow peer as a broken one -- a new bug, not an old one.
    //   5/6  SYSCALL / ZERO_RETURN -- the transport is gone (EPIPE, ECONNRESET) or
    //        the peer sent close_notify. Mirrors `tcp::write`, which maps every
    //        errno that is not EAGAIN/EINTR to ErrConnectionClosed.
    //   else the protocol failure `ErrTlsFailed` has always meant.
    //
    // Reusing FNPTR_OFFSET for `SSL_get_error` is safe because every arm below is
    // terminal: nothing re-enters `write_loop`, which is the only reader of the
    // `SSL_write` pointer. `lower_tls_read_openssl` does exactly the same.
    instructions.push(abi::label(&write_classify));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_get_error",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), N_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::sign_extend_word(&v10, abi::c_return(0)),
        abi::compare_immediate(&v10, "2"), // SSL_ERROR_WANT_READ
        abi::branch_eq(&write_timeout),
        abi::compare_immediate(&v10, "3"), // SSL_ERROR_WANT_WRITE
        abi::branch_eq(&write_timeout),
        abi::compare_immediate(&v10, "5"), // SSL_ERROR_SYSCALL
        abi::branch_eq(&peer_closed),
        abi::compare_immediate(&v10, "6"), // SSL_ERROR_ZERO_RETURN
        abi::branch_eq(&peer_closed),
    ]);
    instructions.push(abi::label(&write_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        "ErrConnectionClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&write_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    if !text {
        // bug-497: the byte form was handed a block whose header is not a
        // `List OF Byte`'s — refuse rather than read a length out of its bytes.
        instructions.push(abi::label(&bad_payload));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.close
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_close_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 80;
    const REC_OFFSET: usize = 8;
    const SSL_OFFSET: usize = 16;
    const CTX_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;

    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let ctx_done = format!("{symbol}_ctx_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC_OFFSET),
        // Idempotent: a closed handle returns OK.
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&already),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
    ]);
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    // SSL_shutdown(ssl)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_shutdown",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // SSL_free(ssl)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // SSL_CTX_free(ctx) — null-guarded: an accepted (server-side) socket
    // stores 0 here because its context is owned by the listener and shared
    // with sibling sockets; freeing it would double-free / kill live sessions
    // (plan-06-tls-server.md §6.4). Client sockets own their ctx and free it
    // exactly as before.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&ctx_done),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&ctx_done));
    // close(fd)
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Mark the record closed.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, TLS_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // A failure to resolve OpenSSL during close still closes the fd and reports
    // success-ish OK (the session is gone); but to surface load problems we map
    // it to ErrTlsFailed.
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// tls.closeListener (internal listener-shaped close body; the user-facing
// name stays `tls::close` — see plan-06-tls-server.md §4.1/§6.4)
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_close_listener_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 64;
    const REC_OFFSET: usize = 8;
    const FD_OFFSET: usize = 16;
    const CTX_OFFSET: usize = 24;
    const HANDLE_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;

    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC_OFFSET),
        // Idempotent: a closed handle returns OK.
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&already),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::return_register(), TLS_LISTENER_OFFSET_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
    ]);
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    // SSL_CTX_free(ctx) — the listener owns the shared server context and
    // frees it exactly once here; accepted sockets only point at it.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // close(fd)
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Mark the record closed.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC_OFFSET),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, TLS_LISTENER_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

#[cfg(test)]
mod error_path_release_tests {
    // Regression guards for bug-55 on the OpenSSL/Linux TLS backend. These paths
    // cannot execute on this macOS host; the assertions pin the emitted release
    // sequence so a post-handshake OOM cannot silently leak the fd + SSL(+CTX),
    // and so the alloc_fail cleanup is null/-1-guarded.
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;
    use crate::codegen::engine::tests::{has_label, TestPlatform};

    fn reloc_count(rel: &[CodeRelocation], needle: &str) -> usize {
        rel.iter().filter(|r| r.to.contains(needle)).count()
    }

    #[test]
    fn connect_alloc_fail_frees_ssl_ctx_and_fd() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (ins, rel, _s) =
            lower_tls_connect_helper("c", &imports, &TestPlatform, false).expect("lower connect");
        for label in [
            "c_af_skip_ssl",
            "c_af_skip_ctx",
            "c_af_skip_fd",
            "c_alloc_fail_raw",
        ] {
            assert!(
                has_label(&ins, label),
                "missing alloc_fail cleanup label {label}"
            );
        }
        // alloc_fail resolves SSL_free and SSL_CTX_free (neither was referenced by
        // connect before the fix — SSL_CTX_free was close-only).
        assert!(
            reloc_count(&rel, "sym_SSL_free") >= 1,
            "connect must free the SSL on OOM"
        );
        assert!(
            reloc_count(&rel, "sym_SSL_CTX_free") >= 1,
            "connect must free the SSL_CTX on OOM"
        );
    }

    /// Whether `dlsym(<name>)` is emitted between labels `start` and `end`.
    ///
    /// `emit_dlsym` materialises the symbol's data address with an `adrp`
    /// carrying `_mfb_tls_sym_<name>`, so a resolution is visible positionally.
    /// A whole-function reloc count cannot substitute: `connect` already frees
    /// SSL/SSL_CTX in `alloc_fail`, so only a windowed check proves `tls_fail`
    /// frees them too.
    fn resolves_between(ins: &[CodeInstruction], start: &str, end: &str, name: &str) -> bool {
        let at = |label: &str| {
            ins.iter()
                .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(label))
                .unwrap_or_else(|| panic!("missing label {label}"))
        };
        let (from, to) = (at(start), at(end));
        assert!(from < to, "expected {start} to precede {end}");
        let want = sym_data_symbol(name);
        ins[from..to]
            .iter()
            .any(|i| i.get("symbol").as_deref() == Some(&want))
    }

    // bug-317 T2: `tls_fail` is branched to from SSL_new onward — SSL_set_fd,
    // SSL_set1_host, the min-proto ctrl, SSL_connect, SSL_get_verify_result — at
    // every one of which this frame owns the SSL session and the per-connection
    // SSL_CTX. It used to close only the fd, so a reconnect loop against an
    // expired- or untrusted-cert host leaked several KB of OpenSSL heap per
    // attempt, while the sibling alloc_fail and the accept-side ssl_fail freed
    // both. The frees are null-guarded (slots are sentinel-initialised to 0).
    #[test]
    fn connect_tls_fail_frees_ssl_and_ctx() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (ins, _r, _s) =
            lower_tls_connect_helper("c", &imports, &TestPlatform, false).expect("lower connect");
        for label in ["c_tf_skip_ssl", "c_tf_skip_ctx", "c_tls_fail_raw"] {
            assert!(
                has_label(&ins, label),
                "missing tls_fail cleanup label {label} (the frees must be null-guarded)"
            );
        }
        assert!(
            resolves_between(&ins, "c_tls_fail", "c_tls_fail_raw", "SSL_free"),
            "a handshake failure must free the SSL session, not just close the fd"
        );
        assert!(
            resolves_between(&ins, "c_tls_fail", "c_tls_fail_raw", "SSL_CTX_free"),
            "a handshake failure must free the per-connection SSL_CTX"
        );
    }

    #[test]
    fn accept_alloc_fail_frees_ssl_and_fd() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_ins, rel, _s) =
            lower_tls_accept_helper("a", &imports, &TestPlatform).expect("lower accept");
        // ssl_fail resolves SSL_free once (2 data relocs); alloc_fail now adds a
        // second resolution, so the count roughly doubles.
        assert!(
            reloc_count(&rel, "sym_SSL_free") > 2,
            "accept alloc_fail must free the SSL session in addition to ssl_fail"
        );
    }

    // A `List OF Byte` is a PACKED fixed-width block (kind 2, plan-57-D): its
    // entry stride is zero and the payload starts immediately after the header,
    // with no entry-descriptor array. The OpenSSL `tls::read` byte path was
    // missed by that change and kept writing 40-byte descriptors, so element 0
    // landed on an entry-flags byte and every read reported a first byte of
    // COLLECTION_ENTRY_FLAG_USED (1) instead of the wire byte. Measured on
    // Alpine x86_64 (box 2227) before the fix: `first byte=1` for an HTTP reply
    // whose first byte is 'H' (72); after: `first byte=72`.
    //
    // The guard is on the fill loop: with a zero stride its body may only copy
    // the payload, never store an entry descriptor. Windowed between the loop's
    // own labels because the block header legitimately stores at the same
    // offsets (COLLECTION_OFFSET_CAPACITY == COLLECTION_ENTRY_OFFSET_KEY_LENGTH
    // == 16) before the loop begins.
    #[test]
    fn byte_read_fill_loop_writes_no_entry_descriptor() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (ins, _rel, _s) =
            lower_tls_read_openssl("r", &imports, &TestPlatform).expect("lower read");
        let start = ins
            .iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("r_entry_loop"))
            .expect("byte read must emit the fill loop");
        let end = ins
            .iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("r_entry_done"))
            .expect("byte read must emit the fill loop terminator");
        let body = &ins[start..end];
        if byte_list_entry_stride() == 0 {
            for off in [
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ] {
                assert!(
                    !body.iter().any(|i| {
                        i.op == CodeOp::StrU64
                            && i.get("offset").as_deref() == Some(&off.to_string())
                    }),
                    "packed byte list has no entry array, but the fill loop stores an \
                     entry descriptor field at offset {off}"
                );
            }
        }
        // Either way the cursor must advance by the real stride, never by a
        // hardcoded COLLECTION_ENTRY_SIZE.
        assert!(
            body.iter().any(|i| {
                i.op == CodeOp::AddImm
                    && i.get("imm").as_deref() == Some(&byte_list_entry_stride().to_string())
            }),
            "the fill loop must advance its entry cursor by byte_list_entry_stride()"
        );
    }

    #[test]
    fn listen_lowers_with_min_proto_check() {
        // The SSL_CTX_ctrl(SET_MIN_PROTO_VERSION) return is now checked; the
        // helper must still lower cleanly and resolve the ctrl symbol.
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_ins, rel, _s) =
            lower_tls_listen_helper("l", &imports, &TestPlatform).expect("lower listen");
        assert!(reloc_count(&rel, "sym_SSL_CTX_ctrl") >= 1);
    }
}
