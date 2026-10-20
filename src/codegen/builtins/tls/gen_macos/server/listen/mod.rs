//! plan-176-T: the Network.framework `tls::listen` lowering (`lower_tls_listen_macos`), as
//! consecutive phases over one instruction stream: the identity phase here, the endpoint
//! phase in `endpoint.rs`, the listener start in `start.rs`. The frame consts and the
//! `ListenEnv`/`ListenScratch` carriers are shared by all three. The shared imports live in
//! `builtins/tls/gen_macos/server/mod.rs`.

use super::*;

use super::identity::{emit_cf_release_slot, emit_import_pem_item, emit_read_whole_file};
use crate::codegen::engine::operand::VirtualRegister;

mod endpoint;
mod start;

use endpoint::emit_listen_endpoint;
use start::emit_listen_start;

const FRAME_SIZE: usize = 544;
const HOST: usize = 8;
const PORT: usize = 16;
const CERT: usize = 24;
const KEY: usize = 32;
// x4 (backlog) is accepted for ABI parity but unused: Network.framework
// manages its own accept backlog.
const NWH: usize = 40;
const SECH: usize = 48;
const CFH: usize = 56;
const FNPTR: usize = 64;
const HOSTCSTR: usize = 72;
const PORTCSTR: usize = 80;
const PORTBUF: usize = 88; // 88..112
const PATHCSTR: usize = 112;
const FILEFD: usize = 120;
const READOFF: usize = 128;
const CERTBUF: usize = 136;
const CERTLEN: usize = 144;
const KEYBUF: usize = 152;
const KEYLEN: usize = 160;
const DATA: usize = 168;
const ITEMS: usize = 176;
const CERTREF: usize = 184;
const KEYREF: usize = 192;
const IDENT: usize = 200;
const SECIDENT: usize = 208;
const CFG: usize = 216;
const ENDPOINT: usize = 224;
const PARAMS: usize = 232;
const LISTENER: usize = 240;
const QUEUE: usize = 248;
const LCTX: usize = 256;
// bug-477 grew the shared configure block from 64 to 88 bytes (its invoke
// gained the verify-block install), so everything after it moves up 24. The
// server does NOT use the verify block -- it is client-side certificate
// verification -- but it shares CFG_INVOKE and CFG_DESC_SYMBOL, so its block
// must be the same SIZE and its unused captures must be explicitly zeroed:
// the invoke null-checks CFG_CAP_VBLOCK, and stack garbage there would be
// read as a block pointer and called.
const CFGBLOCK: usize = 264; // 264..352: the identity-config block literal
const SBLOCK: usize = 352; // 352..392: state-changed block literal
const CBLOCK: usize = 392; // 392..432: new-connection block literal
const WAITFN: usize = 432;
// bug-885: a non-numeric, non-empty `host` must be resolved to an IPv4
// literal (exactly as `tcp::listen` resolves it) before it reaches
// `nw_endpoint_create_host`. Network.framework silently binds the
// wildcard address on an ephemeral port when the "host" endpoint it is
// given is not itself a usable local bind address, so handing it a raw,
// unvalidated hostname (`"localhost"`, a typo, an IPv6 literal) used to
// listen on every interface on a random port instead of failing.
const RESOLVE_HINTS: usize = 440; // 440..488: addrinfo hints (AF_INET, SOCK_STREAM)
const RESOLVE_RES: usize = 488; // addrinfo* out-param
const RESOLVE_SERVICE: usize = 496; // getaddrinfo service (always NULL here: host is non-NULL)
const RESOLVE_SOCKADDR: usize = 504; // resolved res->ai_addr, stashed across the inet_ntop alloc
const RESOLVEDCSTR: usize = 512; // inet_ntop's dotted-decimal output buffer
const BINDHOSTCSTR: usize = 520; // the host string actually handed to nw_endpoint_create_host
                                 // inet_pton's scratch output (a 4-byte in_addr; stack-only, never allocated):
                                 // when `host` is already a numeric IPv4 literal, getaddrinfo/inet_ntop add
                                 // nothing a caller can observe (the string round-trips to itself) but DO
                                 // cost an arena alloc+free pair every call, which showed up as extra
                                 // `arena.0.alloc_calls` against a plain "127.0.0.1" loop that never needed
                                 // resolving in the first place. Checking this first skips that pair
                                 // entirely for the common numeric-host case.
const PTON_BUF: usize = 528;

// bug-477: the shared configure block must not run into the next literal.
const _: () = assert!(CFGBLOCK + CFG_BLOCK_SIZE <= SBLOCK);
const _: () = assert!(WAITFN + 8 <= FRAME_SIZE);
const _: () = assert!(PTON_BUF + 8 <= FRAME_SIZE);

/// `lower_tls_listen_macos`'s fixed inputs shared by its phases: the helper's symbol and
/// platform, the two body-wide scratch vregs, and every exit label.
struct ListenEnv<'a> {
    symbol: &'a str,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    v9: VirtualRegister,
    v10: VirtualRegister,
    cert_fail: String,
    read_fail_fd: String,
    net_fail: String,
    listen_fail: String,
    load_fail: String,
    alloc_fail: String,
    null_host: String,
    host_ready: String,
    addr_invalid_fail: String,
    port_invalid: String,
    inet_ntop_fail: String,
    inet_ntop_fail_buf_freed: String,
    resolved_buf_freed: String,
    host_is_numeric: String,
    wait_loop: String,
    ready: String,
    done: String,
}

/// bug-575/bug-984: the listen body's five scratch blocks (see the declaration comment in
/// `lower_tls_listen_macos`).
struct ListenScratch {
    cert_scratch: HelperScratch,
    key_scratch: HelperScratch,
    host_scratch_until_the_listener_owns_it: HelperScratch,
    cert_buf_scratch: HelperScratch,
    key_buf_scratch: HelperScratch,
}

pub(crate) fn lower_tls_listen_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let env = ListenEnv {
        symbol,
        platform_imports,
        platform,
        v9,
        v10,
        cert_fail: format!("{symbol}_cert_fail"),
        read_fail_fd: format!("{symbol}_read_fail_fd"),
        net_fail: format!("{symbol}_net_fail"),
        listen_fail: format!("{symbol}_listen_fail"),
        load_fail: format!("{symbol}_load_fail"),
        alloc_fail: format!("{symbol}_alloc_fail"),
        null_host: format!("{symbol}_null_host"),
        host_ready: format!("{symbol}_host_ready"),
        addr_invalid_fail: format!("{symbol}_addr_invalid_fail"),
        port_invalid: format!("{symbol}_port_invalid"),
        inet_ntop_fail: format!("{symbol}_inet_ntop_fail"),
        inet_ntop_fail_buf_freed: format!("{symbol}_inet_ntop_fail_buf_freed"),
        resolved_buf_freed: format!("{symbol}_resolved_buf_freed"),
        host_is_numeric: format!("{symbol}_host_is_numeric"),
        wait_loop: format!("{symbol}_wait"),
        ready: format!("{symbol}_ready"),
        done: format!("{symbol}_done"),
    };
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    // bug-575: three marshalled C-strings, and only TWO of them are scratch.
    //
    // The two PEM paths are: `emit_read_whole_file` hands each to `open`, reads
    // the file, and nothing keeps the string. They are released at `done`, and
    // they are declared HERE rather than inside that sub-emitter because it has no
    // `ret` of its own and because it is called twice through the same `PATHCSTR`
    // slot — one scratch per call, or the first block is overwritten and lost.
    //
    // The HOST copy is NOT scratch on this backend, and freeing it is a
    // use-after-free rather than a leak fix. `tls::listen` parks it in the
    // `Listener` record at `REC_LHOST` (see the store below), because
    // `nw_listener_get_port` answers the port and Network.framework answers no
    // address at all: that string IS `tls::localAddress(listener)`'s answer, for
    // the listener's whole life (bug-465). Releasing it at `done` made
    // `tls_local_address_reports_the_port_a_listener_bound_to` read back
    // `bound  62586` — the port intact, the host an empty string, because the
    // block had been returned to the arena before anyone asked. The bind-all
    // spelling hid it: that path parks the static `_mfb_tls_anyhost` rodata
    // pointer and reads back `"0.0.0.0"` either way.
    //
    // So the host block's owner is the `Listener`, exactly as the record store
    // below says.
    //
    // bug-984: owned is not the same as never freed. The block was absent from the
    // release list and nothing else freed it either, so each listener leaked it.
    // It is released at `done` on every exit that built NO listener — the success
    // path nulls the scratch pointer as it stores the block in the record — and
    // the listener's close frees the one the record holds
    // (`lower_tls_close_listener_macos`). The bind-all path used to park the
    // static `_mfb_tls_anyhost` there; it parks an arena copy now, so the record's
    // word is always a block its close can free.
    //
    // The two PEM buffers are scratch as well (`emit_read_whole_file`).
    let cert_scratch = HelperScratch::declare(&mut vregs, &mut ins);
    let key_scratch = HelperScratch::declare(&mut vregs, &mut ins);
    let host_scratch_until_the_listener_owns_it = HelperScratch::declare(&mut vregs, &mut ins);
    let cert_buf_scratch = HelperScratch::declare(&mut vregs, &mut ins);
    let key_buf_scratch = HelperScratch::declare(&mut vregs, &mut ins);
    let scratch = ListenScratch {
        cert_scratch,
        key_scratch,
        host_scratch_until_the_listener_owns_it,
        cert_buf_scratch,
        key_buf_scratch,
    };
    // x0 = host; x1 = port; x2 = certPath; x3 = keyPath; x4 = backlog (unused).
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CERT),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), KEY),
    ]);
    // bug-876: a port outside 0..=65535 is ErrAddressInvalid, refused before any
    // dlopen/alloc/resolution exactly as tcp::listen does.
    crate::codegen::os::socket::shared::emit_port_range_check(
        PORT,
        &env.port_invalid,
        &mut ins,
        &mut vregs,
    );
    // bug-462: NULL the four CoreFoundation cleanup slots before anything can
    // branch to `cert_fail`. That exit best-effort-releases each of them, guarded
    // on non-NULL -- a guard that only means anything once they hold NULL rather
    // than whatever the caller left on the stack. Every failure BEFORE both refs
    // are set (an unreadable cert, a malformed PEM, an encrypted key, a mismatched
    // pair) went through it, so a misconfigured server did not raise
    // `ErrTlsFailed`: it `CFRelease`d stack garbage and trapped in `CF_IS_OBJC`.
    //
    // bug-885 (review): `RESOLVEDCSTR` is zeroed here too, for the same reason --
    // the `null_host` branch never allocates it, and the free guard after
    // `nw_endpoint_create_host` reads this slot to decide whether there is
    // anything to free. Without this zero, that guard reads stack garbage on
    // the bind-all path and either frees a non-pointer or skips freeing a
    // real one depending on what the caller's frame happened to leave behind.
    ins.extend(
        [DATA, ITEMS, CERTREF, KEYREF, RESOLVEDCSTR]
            .map(|slot| abi::store_u64(abi::ZERO, abi::stack_pointer(), slot)),
    );
    emit_listen_identity(&env, &scratch, &mut ins, &mut rel, &mut vregs)?;
    emit_listen_endpoint(&env, &scratch, &mut ins, &mut rel, &mut vregs)?;
    emit_listen_start(&env, &scratch, &mut ins, &mut rel, &mut vregs)?;
    // listen_fail: bind/start failed — cancel the listener, report a network
    // failure (mirrors net::listenTcp's bind error).
    ins.push(abi::label(&env.listen_fail));
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        NWH,
        "nw_listener_cancel",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    ins.push(abi::label(&env.net_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &env.done);
    // read_fail_fd: a seek/read on an opened PEM file failed — close it first.
    ins.push(abi::label(&env.read_fail_fd));
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FILEFD,
    ));
    platform.emit_close_file(symbol, platform_imports, &mut ins, &mut rel)?;
    ins.push(abi::label(&env.cert_fail));
    // Best-effort release of the PEM-import CoreFoundation objects still held
    // when the import fails (bug-236). Every slot is NULL-guarded and cleared, so
    // this is correct for an exit taken before a slot was filled and for one
    // taken after the success path already released it. `read_fail_fd` falls
    // through to here, where all four slots are still NULL — a no-op.
    //
    // Deliberately NOT emitted at `load_fail`: these releases resolve `CFRelease`
    // through `dlsym`, whose own failure branches to `load_fail` — emitting them
    // there would let that branch loop back into itself.
    for slot in [CERTREF, KEYREF, ITEMS, DATA] {
        emit_cf_release_slot(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            CFH,
            slot,
            FNPTR,
            &env.load_fail,
            &mut vregs,
        )?;
    }
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &env.done);
    // bug-885: `host` is non-empty and does not resolve to an IPv4 address —
    // the cert/key CF objects are already released (on the earlier success
    // path, before `host` is ever touched) and no listener exists yet, so
    // nothing but the standard `done` scratch release is needed here.
    // `inet_ntop_fail` falls through into this label after freeing the
    // `getaddrinfo` result and the (unused) resolved-host buffer; a bare
    // `getaddrinfo` failure reaches this label directly, with neither to free.
    // The buffer free is guarded on non-NULL for defence in depth even though
    // reaching `inet_ntop_fail` already implies the `emit_alloc` above ran
    // (the branch here is only taken after a successful `getaddrinfo`, which
    // is the one and only thing that precedes the allocation).
    ins.push(abi::label(&env.inet_ntop_fail));
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), RESOLVEDCSTR),
        abi::compare_immediate(v9, "0"),
        abi::branch_eq(&env.inet_ntop_fail_buf_freed),
        abi::move_register(abi::return_register(), v9),
        abi::move_immediate(abi::c_arg(1), "Integer", "64"),
    ]);
    emit_arena_free(symbol, &mut ins, &mut rel);
    ins.push(abi::label(&env.inet_ntop_fail_buf_freed));
    ins.extend([abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RESOLVE_RES,
    )]);
    platform.emit_external_call("freeaddrinfo", symbol, platform_imports, &mut ins, &mut rel)?;
    ins.push(abi::label(&env.addr_invalid_fail));
    emit_fail(symbol, "ErrAddressInvalid", &mut ins, &mut rel, &env.done);
    // bug-876: out-of-range port; reached before any acquisition, so no cleanup.
    ins.push(abi::label(&env.port_invalid));
    emit_fail(symbol, "ErrAddressInvalid", &mut ins, &mut rel, &env.done);
    ins.push(abi::label(&env.load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &env.done);
    ins.push(abi::label(&env.alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &env.done);
    ins.push(abi::label(&env.done));
    let ListenScratch {
        cert_scratch,
        key_scratch,
        host_scratch_until_the_listener_owns_it,
        cert_buf_scratch,
        key_buf_scratch,
    } = scratch;
    // bug-984: the host block is in the list, and is null on the one exit where
    // the record took it (see its declaration).
    emit_helper_scratch_release(
        symbol,
        &[
            cert_scratch,
            key_scratch,
            cert_buf_scratch,
            key_buf_scratch,
            host_scratch_until_the_listener_owns_it,
        ],
        &mut vregs,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::return_());
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}

/// `lower_tls_listen_macos` phase 1: read the PEM pair, load the three frameworks, import
/// the cert and key, and pair them into the `sec_identity` the listener presents.
fn emit_listen_identity(
    env: &ListenEnv,
    scratch: &ListenScratch,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let ListenEnv {
        symbol,
        platform_imports,
        platform,
        v9,
        ..
    } = *env;
    // Read the PEM pair into arena buffers before touching any framework.
    emit_read_whole_file(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        "cert",
        CERT,
        PATHCSTR,
        FILEFD,
        READOFF,
        CERTBUF,
        CERTLEN,
        &env.cert_fail,
        &env.read_fail_fd,
        &env.alloc_fail,
        &scratch.cert_scratch,
        &scratch.cert_buf_scratch,
        &mut *vregs,
    )?;
    emit_read_whole_file(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        "key",
        KEY,
        PATHCSTR,
        FILEFD,
        READOFF,
        KEYBUF,
        KEYLEN,
        &env.cert_fail,
        &env.read_fail_fd,
        &env.alloc_fail,
        &scratch.key_scratch,
        &scratch.key_buf_scratch,
        &mut *vregs,
    )?;
    // dlopen Network.framework, Security.framework, CoreFoundation.
    emit_dlopen_at(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        MACLIB_SYMBOL,
        NWH,
        &env.load_fail,
    )?;
    emit_dlopen_at(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        MACSEC_SYMBOL,
        SECH,
        &env.load_fail,
    )?;
    emit_dlopen_at(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        MACCF_SYMBOL,
        CFH,
        &env.load_fail,
    )?;
    // certRef / keyRef from the PEM bytes.
    emit_import_pem_item(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        CERTBUF,
        CERTLEN,
        DATA,
        ITEMS,
        CERTREF,
        SECH,
        CFH,
        FNPTR,
        &env.cert_fail,
        &env.load_fail,
        &mut *vregs,
    )?;
    emit_import_pem_item(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        KEYBUF,
        KEYLEN,
        DATA,
        ITEMS,
        KEYREF,
        SECH,
        CFH,
        FNPTR,
        &env.cert_fail,
        &env.load_fail,
        &mut *vregs,
    )?;
    // identity = SecIdentityCreate(NULL, certRef, keyRef) — the keychain-free
    // cert+key pairing entry point in Security.framework (resolved via dlsym;
    // absent => ErrTlsFailed, never a stub).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        SECH,
        "SecIdentityCreate",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CERTREF),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), KEYREF),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&env.cert_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), IDENT),
    ]);
    // SecIdentityCreate took its own references to the cert and key, so drop the
    // ones `emit_import_pem_item` retained for us (bug-236). Done here rather
    // than at function exit so the identity holds the only remaining refs, and so
    // the error exits below have nothing left to unwind for them.
    for slot in [CERTREF, KEYREF] {
        emit_cf_release_slot(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut *ins,
                relocations: &mut *rel,
            },
            CFH,
            slot,
            FNPTR,
            &env.load_fail,
            &mut *vregs,
        )?;
    }
    // secIdentity = sec_identity_create(identity)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        SECH,
        "sec_identity_create",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), IDENT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&env.cert_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SECIDENT),
    ]);
    Ok(())
}
