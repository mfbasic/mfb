// Included into schannel.rs. The server-path helpers: listen (bind/listen + build
// the Schannel server credential from a PEM cert+key), accept (accept + the
// AcceptSecurityContext handshake), and closeListener (FreeCredentialsHandle +
// closesocket). The accepted socket reuses the client's read/write/close helpers
// over the shared `st::` STATE block; a `st::SERVER` marker tells `lower_tls_close`
// not to free the listener-owned credential (plan-47-J / plan-06-tls-server.md).

const SECPKG_CRED_INBOUND: &str = "1";
// ASC_REQ_REPLAY_DETECT|SEQUENCE_DETECT|CONFIDENTIALITY|ALLOCATE_MEMORY|
// EXTENDED_ERROR|STREAM = 4+8+0x10+0x100+0x8000+0x10000 = 98588. The STREAM bit is
// 0x10000 for ASC (0x8000 for ISC) — the flag values differ between the two calls.
const ASC_REQ_FLAGS: &str = "98588";
// CryptStringToBinaryA dwFlags: CRYPT_STRING_BASE64HEADER strips the PEM armor.
const CRYPT_STRING_BASE64HEADER: &str = "0";
// X509_ASN_ENCODING(1) | PKCS_7_ASN_ENCODING(0x10000).
const X509_PKCS7_ENCODING: &str = "65537";
// Wait for an inbound connection: WSAPoll(POLLRDNORM).
const POLLRDNORM: &str = "256";

// --- listener persistent state (arena block pointed at by listener record+16) ---
// Persisted across accept() calls: the server credential, the cert context, and
// the CNG key/provider handles (freed at closeListener).
mod stl {
    pub const SC_CRED: usize = 0; // SCHANNEL_CRED (0x60)
    pub const CERTPTR: usize = 96; // PCCERT_CONTEXT (paCred points here) (8)
    pub const CRED: usize = 104; // CredHandle (16)
    pub const EXPIRY: usize = 120; // TimeStamp (8)
    pub const CBBIN: usize = 128; // CryptStringToBinaryA cbBinary (u32, reused)
    pub const BYTESRD: usize = 136; // ReadFile lpNumberOfBytesRead (u32, reused)
    pub const PKINFO: usize = 144; // CRYPT_PRIVATE_KEY_INFO* (LocalAlloc'd)
    pub const CBPK: usize = 152; // its size (out)
    pub const KBLOB: usize = 160; // CAPI PRIVATEKEYBLOB* (LocalAlloc'd)
    pub const CBKB: usize = 168; // its size (out)
    pub const HPROV: usize = 176; // HCRYPTPROV (named keyset container)
    pub const HKEY: usize = 184; // HCRYPTKEY (imported private key)
    pub const CONTNAME: usize = 192; // wide container name "M"+hex(WORK ptr) (48)
    pub const KPI: usize = 240; // CRYPT_KEY_PROV_INFO (48)
    pub const SIZE: usize = 288;
}

/// A Win64 external call that does NOT sign-extend its return — for the pointer-
/// and BOOL-returning CryptoAPI/file calls (a `sign_extend_word` on a 64-bit
/// HANDLE/pointer return would corrupt any value with bit 31 set). Args 0..=3 are
/// caller-set ABI roles; args 4.. are spilled to the Win64 stack tail above the
/// shadow (mirroring `sspi_call`, whose only difference is the trailing extend).
fn win_call(
    from: &str,
    symbol: &str,
    n_args: usize,
    sext: bool,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if n_args > 4 {
        let stack = n_args - 4;
        let frame = (0x20 + stack * 8 + 15) & !15;
        ins.push(abi::subtract_stack(frame));
        for i in 0..stack {
            ins.push(abi::store_u64(abi::c_arg(4 + i), abi::stack_pointer(), 0x20 + i * 8));
        }
        platform.emit_libc_call(symbol, from, imports, ins, rel)?;
        ins.push(abi::add_stack(frame));
    } else {
        platform.emit_libc_call(symbol, from, imports, ins, rel)?;
    }
    if sext {
        ins.push(abi::sign_extend_word(abi::return_register(), abi::return_register()));
    }
    Ok(())
}

/// Read the whole file whose UTF-8 path is the MFB `String` at frame `path_off`
/// into a fresh 64 KiB arena buffer; leave the buffer pointer at `buf_off` and the
/// byte count at `len_off`. `wide_off`/`hfile_off` are caller scratch frame slots;
/// `work_off` holds the arena WORK pointer (its `stl::BYTESRD` receives the
/// ReadFile count). PEM cert/key files are well under 64 KiB.
#[allow(clippy::too_many_arguments)]
fn emit_read_file(
    symbol: &str,
    tag: &str,
    path_off: usize,
    wide_off: usize,
    hfile_off: usize,
    buf_off: usize,
    len_off: usize,
    work_off: usize,
    fail: &str,
    alloc_fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // Marshal the path to UTF-16 for CreateFileW.
    emit_wide_cstring(symbol, path_off, wide_off, alloc_fail, imports, platform, ins, rel)?;
    // Allocate the 64 KiB read buffer.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, alloc_fail);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), buf_off));
    // CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING,
    //   FILE_ATTRIBUTE_NORMAL, NULL).
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), wide_off),
        abi::move_immediate(abi::c_arg(1), "Integer", "2147483648"), // GENERIC_READ 0x80000000
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),          // FILE_SHARE_READ
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),          // lpSecurityAttributes
        abi::move_immediate(abi::c_arg(4), "Integer", "3"),          // OPEN_EXISTING
        abi::move_immediate(abi::c_arg(5), "Integer", "128"),        // FILE_ATTRIBUTE_NORMAL
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),          // hTemplateFile
    ]);
    win_call(symbol, "CreateFileW", 7, false, imports, platform, ins, rel)?;
    // INVALID_HANDLE_VALUE is (HANDLE)-1 (full 64-bit -1) → < 0.
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), hfile_off),
    ]);
    // ReadFile(hFile, buf, 65536, &bytesRead, NULL). A regular disk file satisfies
    // one ReadFile up to EOF; PEM cert/key files are tiny.
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hfile_off),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), buf_off),
        abi::move_immediate(abi::c_arg(2), "Integer", "65536"),
        abi::load_u64("%v10", abi::stack_pointer(), work_off),
        abi::add_immediate(abi::c_arg(3), "%v10", stl::BYTESRD),
        abi::move_immediate(abi::c_arg(4), "Integer", "0"),
    ]);
    // Zero the count slot first (ReadFile writes only the low DWORD).
    ins.push(abi::store_u64(abi::ZERO, "%v10", stl::BYTESRD));
    win_call(symbol, "ReadFile", 5, false, imports, platform, ins, rel)?;
    let read_done = format!("{symbol}_{tag}_read_done");
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail), // BOOL FALSE
        abi::load_u64("%v10", abi::stack_pointer(), work_off),
        abi::load_u32("%v11", "%v10", stl::BYTESRD),
        abi::store_u64("%v11", abi::stack_pointer(), len_off),
        abi::compare_immediate("%v11", "0"),
        abi::branch_ne(&read_done),
        abi::branch(fail), // empty file → not a valid PEM
        abi::label(&read_done),
        // CloseHandle(hFile)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), hfile_off),
    ]);
    win_call(symbol, "CloseHandle", 1, false, imports, platform, ins, rel)?;
    Ok(())
}

/// PEM(base64-armored) buffer at `pem_off`/`pem_len_off` → DER in a fresh arena
/// buffer, pointer left at `der_off` and byte count at `der_len_off`. `work_off`
/// holds the arena WORK pointer (its `stl::CBBIN` is the in/out size DWORD).
#[allow(clippy::too_many_arguments)]
fn emit_pem_to_der(
    symbol: &str,
    pem_off: usize,
    pem_len_off: usize,
    der_off: usize,
    der_len_off: usize,
    work_off: usize,
    fail: &str,
    alloc_fail: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // DER is smaller than its base64 PEM; a buffer the size of the PEM is ample.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, alloc_fail);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), der_off));
    // Seed the in/out capacity DWORD with the buffer size.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), work_off),
        abi::move_immediate("%v9", "Integer", "65536"),
        abi::store_u32("%v9", "%v10", stl::CBBIN),
        // CryptStringToBinaryA(pem, pemLen, BASE64HEADER, der, &cbBin, NULL, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), pem_off),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), pem_len_off),
        abi::move_immediate(abi::c_arg(2), "Integer", CRYPT_STRING_BASE64HEADER),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), der_off),
        abi::add_immediate(abi::c_arg(4), "%v10", stl::CBBIN),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
        abi::move_immediate(abi::c_arg(6), "Integer", "0"),
    ]);
    win_call(symbol, "CryptStringToBinaryA", 7, false, imports, platform, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail), // BOOL FALSE
        abi::load_u64("%v10", abi::stack_pointer(), work_off),
        abi::load_u32("%v11", "%v10", stl::CBBIN),
        abi::store_u64("%v11", abi::stack_pointer(), der_len_off),
    ]);
    Ok(())
}

/// Build a unique wide (UTF-16LE) key-container name at WORK.CONTNAME: 'M'
/// followed by the 16 hex digits of the WORK arena pointer (distinct per
/// listener) and a NUL. No external calls, so plain vregs are fine.
fn emit_container_name(symbol: &str, work_off: usize, ins: &mut Vec<CodeInstruction>) {
    let loop_l = format!("{symbol}_cn_loop");
    let digit_l = format!("{symbol}_cn_digit");
    let emit_l = format!("{symbol}_cn_emit");
    let done_l = format!("{symbol}_cn_done");
    ins.extend([
        abi::load_u64("%v18", abi::stack_pointer(), work_off), // block base
        abi::move_register("%v11", "%v18"),                    // hexify the ptr value
        abi::move_immediate("%v9", "Integer", "77"),           // 'M'
        abi::store_u16("%v9", "%v18", stl::CONTNAME),
        abi::add_immediate("%v13", "%v18", stl::CONTNAME + 2), // dst cursor
        abi::move_immediate("%v12", "Integer", "0"),           // i
        abi::label(&loop_l),
        abi::compare_immediate("%v12", "16"),
        abi::branch_eq(&done_l),
        abi::shift_right_immediate("%v14", "%v11", 60),        // top nibble
        abi::compare_immediate("%v14", "10"),
        abi::branch_lt(&digit_l),
        abi::add_immediate("%v15", "%v14", 55),                // 'A' - 10
        abi::branch(&emit_l),
        abi::label(&digit_l),
        abi::add_immediate("%v15", "%v14", 48),                // '0'
        abi::label(&emit_l),
        abi::store_u16("%v15", "%v13", 0),
        abi::add_immediate("%v13", "%v13", 2),
        abi::shift_left_immediate("%v11", "%v11", 4),
        abi::add_immediate("%v12", "%v12", 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
        abi::store_u16(abi::ZERO, "%v13", 0),                  // NUL terminator
    ]);
}

// ---------------------------------------------------------------------------
// tls.listen(host, port, certPath, keyPath, backlog) -> TlsListener
// ---------------------------------------------------------------------------
pub(super) fn lower_tls_listen(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const HOST: usize = 8;
    const PORT: usize = 16;
    const CERT: usize = 24;
    const KEY: usize = 32;
    const BACKLOG: usize = 40;
    const HINTS: usize = 48; // 48..96
    const RES: usize = 96;
    const FD: usize = 104;
    const HOSTCSTR: usize = 112;
    const ONE: usize = 120;
    const WORK: usize = 128; // arena WORK ptr (stl::*)
    const WIDE: usize = 136; // wide path scratch
    const PEMBUF: usize = 144;
    const PEMLEN: usize = 152;
    const HFILE: usize = 160;
    const DERBUF: usize = 168;
    const DERLEN: usize = 176;
    const FRAME_SIZE: usize = 0x100;

    let addr_off = platform.addrinfo_addr_offset();
    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let tls_fail_fd = format!("{symbol}_tls_fail_fd");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CERT),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), KEY),
        abi::store_u64(abi::c_arg(4), abi::stack_pointer(), BACKLOG),
    ]);
    // hints: zero 48 bytes, ai_flags=AI_PASSIVE|AF_INET, ai_socktype=SOCK_STREAM.
    for o in (0..48).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), HINTS + o));
    }
    ins.extend([
        abi::move_immediate("%v9", "Integer", super::HINTS_FAMILY_WORD_PASSIVE),
        abi::store_u64("%v9", abi::stack_pointer(), HINTS),
        abi::move_immediate("%v9", "Integer", super::SOCK_STREAM),
        abi::store_u64("%v9", abi::stack_pointer(), HINTS + 8),
        // Empty host => NULL node (bind all interfaces).
        abi::load_u64("%v9", abi::stack_pointer(), HOST),
        abi::load_u64("%v9", "%v9", 0),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&null_host),
    ]);
    super::emit_cstring(symbol, "host", HOST, HOSTCSTR, &alloc_fail, &mut ins, &mut rel);
    ins.extend([
        abi::branch(&resolved),
        abi::label(&null_host),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HOSTCSTR),
        abi::label(&resolved),
        // getaddrinfo(host, NULL, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HOSTCSTR),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HINTS),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), RES),
    ]);
    platform.emit_libc_call("getaddrinfo", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // socket(ai_family, ai_socktype, ai_protocol)
        abi::load_u64("%v9", abi::stack_pointer(), RES),
        abi::load_u32(abi::return_register(), "%v9", 4),
        abi::load_u32(abi::c_arg(1), "%v9", 8),
        abi::load_u32(abi::c_arg(2), "%v9", 12),
    ]);
    platform.emit_libc_call("socket", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD),
        // Overwrite sin_port (ai_addr + 2/3) with the requested port.
        abi::load_u64("%v9", abi::stack_pointer(), RES),
        abi::load_u64("%v9", "%v9", addr_off),
        abi::load_u64("%v10", abi::stack_pointer(), PORT),
        abi::shift_right_immediate("%v11", "%v10", 8),
        abi::store_u8("%v11", "%v9", 2),
        abi::store_u8("%v10", "%v9", 3),
        // setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, 4) — best effort.
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", abi::stack_pointer(), ONE),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD),
        abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
        abi::move_immediate(abi::c_arg(2), "Integer", platform.so_reuseaddr()),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), ONE),
        abi::move_immediate(abi::c_arg(4), "Integer", "4"),
    ]);
    sspi_call(symbol, "setsockopt", "ws2_32.dll", 5, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        // bind(fd, ai_addr, ai_addrlen)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD),
        abi::load_u64("%v9", abi::stack_pointer(), RES),
        abi::load_u64(abi::c_arg(1), "%v9", addr_off),
        abi::load_u32(abi::c_arg(2), "%v9", 16),
    ]);
    platform.emit_libc_call("bind", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
        // listen(fd, backlog)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BACKLOG),
    ]);
    platform.emit_libc_call("listen", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES),
    ]);
    platform.emit_libc_call("freeaddrinfo", symbol, imports, &mut ins, &mut rel)?;

    // --- Build the Schannel server credential from the PEM cert + key ---
    // Allocate the persistent WORK block (zeroed) that the listener record points at.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &stl::SIZE.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &tls_fail_fd);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), WORK));
    ins.push(abi::move_register("%v10", abi::mfb_return(1)));
    for o in (0..stl::SIZE).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, "%v10", o));
    }

    // cert: PEM file -> DER -> CertCreateCertificateContext -> WORK.CERTPTR.
    emit_read_file(symbol, "cert", CERT, WIDE, HFILE, PEMBUF, PEMLEN, WORK, &tls_fail_fd, &tls_fail_fd, imports, platform, &mut ins, &mut rel)?;
    emit_pem_to_der(symbol, PEMBUF, PEMLEN, DERBUF, DERLEN, WORK, &tls_fail_fd, &tls_fail_fd, imports, platform, &mut ins, &mut rel)?;
    // CertCreateCertificateContext(X509|PKCS7, der, cbDer). NOTE: it references the
    // DER (an arena alloc that outlives this call), so the buffer is not freed.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", X509_PKCS7_ENCODING),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DERBUF),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), DERLEN),
    ]);
    win_call(symbol, "CertCreateCertificateContext", 3, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd),
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::store_u64(abi::return_register(), "%v10", stl::CERTPTR),
    ]);

    // key: PEM file -> DER (PKCS#8), then legacy-CAPI import into an ephemeral
    // provider. A software-KSP CNG ephemeral key is not reachable by Schannel's
    // credential path (both CNG association forms yield SEC_E_NO_CREDENTIALS); the
    // classic CryptImportKey-into-VERIFYCONTEXT + CERT_KEY_CONTEXT recipe is.
    emit_read_file(symbol, "key", KEY, WIDE, HFILE, PEMBUF, PEMLEN, WORK, &tls_fail_fd, &tls_fail_fd, imports, platform, &mut ins, &mut rel)?;
    emit_pem_to_der(symbol, PEMBUF, PEMLEN, DERBUF, DERLEN, WORK, &tls_fail_fd, &tls_fail_fd, imports, platform, &mut ins, &mut rel)?;
    // CryptDecodeObjectEx(X509|PKCS7, PKCS_PRIVATE_KEY_INFO=44, pkcs8Der, cb,
    //   CRYPT_DECODE_ALLOC_FLAG, NULL, &WORK.PKINFO, &WORK.CBPK) — unwrap PKCS#8.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", X509_PKCS7_ENCODING),
        abi::move_immediate(abi::c_arg(1), "Integer", "44"), // PKCS_PRIVATE_KEY_INFO
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), DERBUF),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), DERLEN),
        abi::move_immediate(abi::c_arg(4), "Integer", "32768"), // CRYPT_DECODE_ALLOC_FLAG
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::add_immediate(abi::c_arg(6), "%v10", stl::PKINFO),
        abi::add_immediate(abi::c_arg(7), "%v10", stl::CBPK),
    ]);
    win_call(symbol, "CryptDecodeObjectEx", 8, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd), // BOOL FALSE
    ]);
    // CryptDecodeObjectEx(X509|PKCS7, PKCS_RSA_PRIVATE_KEY=43, pkInfo->PrivateKey
    //   {cbData@32, pbData@40}, CRYPT_DECODE_ALLOC_FLAG, NULL, &WORK.KBLOB, &WORK.CBKB).
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::load_u64("%v11", "%v10", stl::PKINFO), // CRYPT_PRIVATE_KEY_INFO*
        abi::move_immediate(abi::return_register(), "Integer", X509_PKCS7_ENCODING),
        abi::move_immediate(abi::c_arg(1), "Integer", "43"), // PKCS_RSA_PRIVATE_KEY
        abi::load_u64(abi::c_arg(2), "%v11", 40),            // PrivateKey.pbData
        abi::load_u32(abi::c_arg(3), "%v11", 32),            // PrivateKey.cbData
        abi::move_immediate(abi::c_arg(4), "Integer", "32768"),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
        abi::add_immediate(abi::c_arg(6), "%v10", stl::KBLOB),
        abi::add_immediate(abi::c_arg(7), "%v10", stl::CBKB),
    ]);
    win_call(symbol, "CryptDecodeObjectEx", 8, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd),
    ]);
    // Build a unique wide container name "M"+hex(WORK ptr) at WORK.CONTNAME. A CNG
    // ephemeral key / VERIFYCONTEXT-imported key is not retrievable by Schannel; a
    // named keyset container makes CryptGetUserKey(AT_KEYEXCHANGE) find the imported
    // key, which is what CERT_KEY_PROV_INFO drives.
    emit_container_name(symbol, WORK, &mut ins);
    // Delete any stale keyset of this name (best effort), then create it fresh.
    for (flag, check) in [("16", false), ("8", true)] {
        // CryptAcquireContextW(&HPROV, CONTNAME, NULL, PROV_RSA_AES, flag)
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), WORK),
            abi::add_immediate(abi::return_register(), "%v10", stl::HPROV),
            abi::add_immediate(abi::c_arg(1), "%v10", stl::CONTNAME),
            abi::move_immediate(abi::c_arg(2), "Integer", "0"),
            abi::move_immediate(abi::c_arg(3), "Integer", "24"), // PROV_RSA_AES
            abi::move_immediate(abi::c_arg(4), "Integer", flag), // 16=DELETEKEYSET, 8=NEWKEYSET
        ]);
        win_call(symbol, "CryptAcquireContextW", 5, false, imports, platform, &mut ins, &mut rel)?;
        if check {
            ins.extend([
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_eq(&tls_fail_fd),
            ]);
        }
    }
    // CryptImportKey(hProv, blob, cbBlob, 0, 0, &WORK.HKEY) — persists as the
    // container's AT_KEYEXCHANGE key pair (the blob algid is CALG_RSA_KEYX).
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), "%v10", stl::HPROV),
        abi::load_u64(abi::c_arg(1), "%v10", stl::KBLOB),
        abi::load_u32(abi::c_arg(2), "%v10", stl::CBKB),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::move_immediate(abi::c_arg(4), "Integer", "0"),
        abi::add_immediate(abi::c_arg(5), "%v10", stl::HKEY),
    ]);
    win_call(symbol, "CryptImportKey", 6, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd),
    ]);
    // CRYPT_KEY_PROV_INFO { pwszContainerName=&CONTNAME; pwszProvName=NULL;
    //   dwProvType=PROV_RSA_AES; dwFlags=0; cProvParam=0; rgProvParam=NULL;
    //   dwKeySpec=AT_KEYEXCHANGE }. The rest of the struct is zero from the block init.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::add_immediate("%v9", "%v10", stl::CONTNAME),
        abi::store_u64("%v9", "%v10", stl::KPI),
        abi::store_u64(abi::ZERO, "%v10", stl::KPI + 8),
        abi::move_immediate("%v9", "Integer", "24"),
        abi::store_u32("%v9", "%v10", stl::KPI + 16),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u32("%v9", "%v10", stl::KPI + 40),
        // CertSetCertificateContextProperty(cert, CERT_KEY_PROV_INFO_PROP_ID=2, 0, &kpi)
        abi::load_u64(abi::return_register(), "%v10", stl::CERTPTR),
        abi::move_immediate(abi::c_arg(1), "Integer", "2"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::add_immediate(abi::c_arg(3), "%v10", stl::KPI),
    ]);
    win_call(symbol, "CertSetCertificateContextProperty", 4, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd), // BOOL FALSE
    ]);

    // SCHANNEL_CRED { dwVersion=4; cCreds=1; paCred=&WORK.CERTPTR; dwFlags=STRONG_CRYPTO }.
    ins.extend([
        abi::load_u64("%v18", abi::stack_pointer(), WORK),
        abi::move_immediate("%v9", "Integer", SCHANNEL_CRED_VERSION),
        abi::store_u32("%v9", "%v18", stl::SC_CRED),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u32("%v9", "%v18", stl::SC_CRED + 4),
        abi::add_immediate("%v9", "%v18", stl::CERTPTR),
        abi::store_u64("%v9", "%v18", stl::SC_CRED + 8),
        // grbitEnabledProtocols = SP_PROT_TLS1_2_SERVER (0x400). The server key is a
        // legacy CryptoAPI key (CryptImportKey/PROV_RSA_AES); TLS 1.3 requires RSA-PSS
        // signing, which a CAPI key cannot do, so AcceptSecurityContext fails on the
        // first ClientHello (proven with openssl s_client: server writes 0 bytes).
        // Pinning TLS 1.2 lets the CAPI key sign with PKCS#1 v1.5. plan-66-F.
        abi::move_immediate("%v9", "Integer", "1024"),
        abi::store_u32("%v9", "%v18", stl::SC_CRED + 56),
        abi::move_immediate("%v9", "Integer", "4194304"), // SCH_USE_STRONG_CRYPTO 0x400000
        abi::store_u32("%v9", "%v18", stl::SC_CRED + 72),
    ]);
    // AcquireCredentialsHandleW(NULL, USP_NAME, SECPKG_CRED_INBOUND, NULL,
    //   &SCHANNEL_CRED, NULL, NULL, &WORK.CRED, &WORK.EXPIRY)
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    wide_addr(symbol, abi::c_arg(1), USP_NAME, &mut ins, &mut rel);
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", SECPKG_CRED_INBOUND),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    sspi_call_ext(
        symbol,
        "AcquireCredentialsHandleW",
        WORK,
        &[Some(stl::SC_CRED), None, None, Some(stl::CRED), Some(stl::EXPIRY)],
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_register("%v15", abi::return_register()),
        abi::compare_immediate("%v15", "0"),
        abi::branch_lt(&tls_fail_fd),
    ]);

    // Build the listener record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the tail { WORK block ptr @TLS_SCHANNEL_OFFSET_BLOCK } (plan-80).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &tls_fail_fd);
    ins.extend([
        abi::move_immediate("%v9", "Integer", RESOURCE_TAG_TLS_LISTENER),
        abi::store_u64("%v9", abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64("%v9", abi::stack_pointer(), FD),
        abi::store_u64("%v9", abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), WORK),
        abi::store_u64("%v9", abi::mfb_return(1), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    // Credential-build failure with the listen fd open: close it, report ErrTlsFailed.
    ins.push(abi::label(&tls_fail_fd));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    // bind/listen failure: close the fd, ErrNetworkFailed.
    ins.push(abi::label(&op_fail));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    // socket() failure: release the resolver results, ErrNetworkFailed.
    ins.push(abi::label(&socket_fail));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), RES));
    platform.emit_libc_call("freeaddrinfo", symbol, imports, &mut ins, &mut rel)?;
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&resolve_fail));
    emit_fail(symbol, "ErrAddressInvalid", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);

    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}

// ---------------------------------------------------------------------------
// tls.accept(listener, timeoutMs) -> TlsSocket
// ---------------------------------------------------------------------------
pub(super) fn lower_tls_accept(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const LCRED: usize = 8; // &listener.WORK.CRED (arena absolute)
    const TIMEOUT: usize = 16;
    const LISTENFD: usize = 24;
    const CONNFD: usize = 32;
    const STATE: usize = 40; // per-connection STATE ptr (st::*)
    const FIRSTF: usize = 48; // 0 until the first ASC has created the context
    const POLLFD: usize = 56; // WSAPOLLFD { SOCKET fd; SHORT events; SHORT revents }
    const HSTV: usize = 64; // plan-73-D: handshake SO_*TIMEO DWORD-ms scratch
    const HSTOF: usize = 72; // plan-73-D: 1 if the handshake recv timed out (WSAETIMEDOUT)
    const FRAME_SIZE: usize = 0x100;

    let closed = format!("{symbol}_closed");
    let no_timeout = format!("{symbol}_no_timeout");
    let accept_invalid = format!("{symbol}_accept_invalid");
    let accept_ts_store = format!("{symbol}_accept_ts_clamped");
    let accept_fail = format!("{symbol}_accept_fail");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let tls_fail = format!("{symbol}_tls_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let hs_read = format!("{symbol}_hs_read");
    let hs_asc = format!("{symbol}_hs_asc");
    let hs_done = format!("{symbol}_hs_done");
    let hs_finish = format!("{symbol}_hs_finish");
    let have_ctx = format!("{symbol}_have_ctx");
    let arg1_done = format!("{symbol}_arg1_done");
    let no_send = format!("{symbol}_no_send");
    let resetrecv = format!("{symbol}_resetrecv");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // return_register = listener record { fd@0, closed@8, WORK@16 }; ARG[1] = timeoutMs.
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTOF),
        abi::load_u64("%v9", abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), LISTENFD),
        abi::load_u64("%v9", abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK), // WORK ptr
        abi::add_immediate("%v9", "%v9", stl::CRED),
        abi::store_u64("%v9", abi::stack_pointer(), LCRED),
        // plan-73-D: the unbounded sentinel => a blocking accept (omit = block); `0`
        // => WSAPoll(0), one immediate attempt (`ErrTimeout` if none pending); `> 0`
        // => WSAPoll(timeoutMs); a negative (non-sentinel) => ErrInvalidArgument.
        abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::move_immediate("%v10", "Integer", crate::target::shared::code::TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&no_timeout),
        abi::compare_immediate("%v9", "0"),
        abi::branch_lt(&accept_invalid),
        // Clamp `> 0` to INT_MAX and store it back — WSAPoll takes a C `int`, so a
        // value with bit 31 set would be read as a block-forever timeout; net clamps
        // identically. WSAPoll and the handshake SO_*TIMEO both reload TIMEOUT.
        abi::move_immediate("%v10", "Integer", "2147483647"),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_le(&accept_ts_store),
        abi::move_register("%v9", "%v10"),
        abi::label(&accept_ts_store),
        abi::store_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::load_u64("%v9", abi::stack_pointer(), LISTENFD),
        abi::store_u64("%v9", abi::stack_pointer(), POLLFD),
        abi::move_immediate("%v10", "Integer", POLLRDNORM),
        abi::store_u16("%v10", abi::stack_pointer(), POLLFD + 8),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), POLLFD + 10),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT),
    ]);
    platform.emit_libc_call("WSAPoll", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::branch_eq(&accept_timeout),
        abi::label(&no_timeout),
        // accept(fd, NULL, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENFD),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_libc_call("accept", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONNFD),
    ]);
    // plan-73-D: bound the handshake recv by SO_RCVTIMEO/SO_SNDTIMEO — sentinel =>
    // unbounded (omit); `0` => 1µs (near-immediate); `> 0` => the timeval. Cleared
    // after the handshake (hs_done) so the returned socket's read/write are unbounded.
    {
        let hs_ts_ok = format!("{symbol}_hs_ts_ok");
        let hs_ts_skip = format!("{symbol}_hs_ts_skip");
        ins.extend([
            abi::load_u64("%v14", abi::stack_pointer(), TIMEOUT),
            abi::move_immediate("%v15", "Integer", crate::target::shared::code::TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers("%v14", "%v15"),
            abi::branch_eq(&hs_ts_skip),
            // Winsock SO_*TIMEO is a DWORD of milliseconds; 0 means infinite, so the
            // convention's `0` (non-blocking) uses 1 ms.
            abi::compare_immediate("%v14", "0"),
            abi::branch_ne(&hs_ts_ok),
            abi::move_immediate("%v14", "Integer", "1"),
            abi::label(&hs_ts_ok),
            abi::store_u64("%v14", abi::stack_pointer(), HSTV),
        ]);
        super::emit_set_sock_timeouts(
            &mut EmitCtx {
                symbol,
                platform_imports: imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            CONNFD,
            HSTV,
        )?;
        ins.push(abi::label(&hs_ts_skip));
    }

    // Allocate the per-connection STATE block (zeroed header) + mark it server-side.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &st::SIZE.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), STATE));
    ins.push(abi::move_register("%v10", abi::mfb_return(1)));
    for o in (0..st::RECV).step_by(8) {
        ins.push(abi::store_u64(abi::ZERO, "%v10", o));
    }
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), FIRSTF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u32("%v9", "%v10", st::SERVER),
        // recv_len = 0
        abi::store_u64(abi::ZERO, "%v10", st::RECV_LEN),
    ]);

    // Handshake: recv (client sends ClientHello first), AcceptSecurityContext, send.
    ins.extend([
        abi::label(&hs_read),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::add_immediate(abi::c_arg(1), "%v10", st::RECV),
        abi::add_registers(abi::c_arg(1), abi::c_arg(1), "%v11"),
        abi::move_immediate(abi::c_arg(2), "Integer", &RECV_CAP.to_string()),
        abi::subtract_registers(abi::c_arg(2), abi::c_arg(2), "%v11"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_libc_call("recv", symbol, imports, &mut ins, &mut rel)?;
    let hs_got = format!("{symbol}_hs_got");
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&hs_got), // got handshake bytes
    ]);
    // plan-73-D: recv <= 0 — an SO_RCVTIMEO expiry is WSAETIMEDOUT (10060), a
    // handshake TIMEOUT → ErrTimeout (via the flag); a peer close or other error
    // stays ErrTlsFailed.
    platform.emit_errno(symbol, ("%v9").into(), imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate("%v9", "10060"), // WSAETIMEDOUT
        abi::branch_ne(&tls_fail),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", abi::stack_pointer(), HSTOF),
        abi::branch(&tls_fail),
        abi::label(&hs_got),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::add_registers("%v11", "%v11", abi::return_register()),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::label(&hs_asc),
        // in SecBuffer[0] = {recv_len, TOKEN, &RECV}; [1] = {0, EMPTY, NULL}
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u64("%v11", "%v10", st::RECV_LEN),
        abi::store_u32("%v11", "%v10", st::INBUF),
        abi::move_immediate("%v9", "Integer", SECBUFFER_TOKEN),
        abi::store_u32("%v9", "%v10", st::INBUF + 4),
        abi::add_immediate("%v9", "%v10", st::RECV),
        abi::store_u64("%v9", "%v10", st::INBUF + 8),
    ]);
    set_secbuffer("%v10", st::INBUF + 16, "0", SECBUFFER_EMPTY, abi::ZERO, &mut ins);
    set_secbuffer_desc("%v10", st::INDESC, "2", st::INBUF, &mut ins);
    set_secbuffer("%v10", st::OUTBUF, "0", SECBUFFER_TOKEN, abi::ZERO, &mut ins);
    set_secbuffer_desc("%v10", st::OUTDESC, "1", st::OUTBUF, &mut ins);
    // phContext = FIRST ? NULL : &ctxt
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LCRED), // 0: phCredential
        abi::load_u64("%v9", abi::stack_pointer(), FIRSTF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&have_ctx),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::branch(&arg1_done),
        abi::label(&have_ctx),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::c_arg(1), "%v9", st::CTXT),
        abi::label(&arg1_done),
        // arg2 = &indesc; arg3 = ASC flags
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::c_arg(2), "%v9", st::INDESC),
        abi::move_immediate(abi::c_arg(3), "Integer", ASC_REQ_FLAGS),
    ]);
    // ASC(&cred, phContext, &indesc, flags, 0, &ctxt, &outdesc, &attrs, &expiry)
    sspi_call_ext(
        symbol,
        "AcceptSecurityContext",
        STATE,
        &[None, Some(st::CTXT), Some(st::OUTDESC), Some(st::ATTRS), Some(st::EXPIRY)],
        imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_register("%v15", abi::return_register()),
        // context now exists → subsequent calls pass &ctxt
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", abi::stack_pointer(), FIRSTF),
    ]);
    branch_if_incomplete("%v15", &hs_read, &mut ins);
    ins.extend([
        abi::compare_immediate("%v15", "0"),
        abi::branch_lt(&tls_fail),
    ]);
    emit_send_token(symbol, CONNFD, STATE, st::OUTBUF, &no_send, "atok", &tls_fail, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::label(&no_send));
    ins.extend([
        abi::compare_immediate("%v15", SEC_E_OK),
        abi::branch_eq(&hs_finish),
        // SEC_I_CONTINUE_NEEDED: handle SECBUFFER_EXTRA in INBUF[1] or recv anew.
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v10", st::INBUF + 16 + 4),
        abi::compare_immediate("%v9", SECBUFFER_EXTRA),
        abi::branch_ne(&resetrecv),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::load_u64("%v12", "%v10", st::RECV_LEN),
        abi::subtract_registers("%v13", "%v12", "%v11"),
        abi::add_immediate("%v14", "%v10", st::RECV),
        abi::add_registers("%v14", "%v14", "%v13"),
        abi::add_immediate("%v6", "%v10", st::RECV),
    ]);
    move_bytes("%v14", "%v6", "%v11", &format!("{symbol}_aextra"), &mut ins);
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::branch(&hs_asc),
        abi::label(&resetrecv),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v10", st::RECV_LEN),
        abi::branch(&hs_read),
        // Handshake complete: the final ASC consumed the client's last flight from
        // RECV. If application data arrived coalesced (INBUF[1] EXTRA), keep it at
        // the front of RECV for the first read; otherwise reset RECV_LEN to 0 so
        // read does not re-decrypt consumed handshake bytes.
        abi::label(&hs_finish),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v10", st::INBUF + 16 + 4),
        abi::compare_immediate("%v9", SECBUFFER_EXTRA),
        abi::branch_ne(&format!("{symbol}_fin_noextra")),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::load_u64("%v12", "%v10", st::RECV_LEN),
        abi::subtract_registers("%v13", "%v12", "%v11"),
        abi::add_immediate("%v14", "%v10", st::RECV),
        abi::add_registers("%v14", "%v14", "%v13"),
        abi::add_immediate("%v6", "%v10", st::RECV),
    ]);
    move_bytes("%v14", "%v6", "%v11", &format!("{symbol}_finextra"), &mut ins);
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v11", "%v10", st::INBUF + 16),
        abi::store_u64("%v11", "%v10", st::RECV_LEN),
        abi::branch(&hs_done),
        abi::label(&format!("{symbol}_fin_noextra")),
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::store_u64(abi::ZERO, "%v10", st::RECV_LEN),
        abi::label(&hs_done),
    ]);

    // plan-73-D: handshake done — clear SO_*TIMEO so the returned socket's read/write
    // stay unbounded. Only `0`/`> 0` installed a timeout; the sentinel left it unset.
    {
        let hs_clr_skip = format!("{symbol}_hs_clr_skip");
        ins.extend([
            abi::load_u64("%v14", abi::stack_pointer(), TIMEOUT),
            abi::move_immediate("%v15", "Integer", crate::target::shared::code::TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers("%v14", "%v15"),
            abi::branch_eq(&hs_clr_skip),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTV),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTV + 8),
        ]);
        super::emit_set_sock_timeouts(
            &mut EmitCtx {
                symbol,
                platform_imports: imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            CONNFD,
            HSTV,
        )?;
        ins.push(abi::label(&hs_clr_skip));
    }

    // QueryContextAttributes(&ctxt, STREAM_SIZES, &sizes) → header/trailer/max.
    // &sizes reuses the per-connection SC_CRED scratch (unused server-side).
    ins.extend([
        abi::load_u64("%v18", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), "%v18", st::CTXT),
        abi::move_immediate(abi::c_arg(1), "Integer", SECPKG_ATTR_STREAM_SIZES),
        abi::add_immediate(abi::c_arg(2), "%v18", st::SC_CRED),
    ]);
    sspi_call(symbol, "QueryContextAttributesW", SECUR32, 3, imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch_lt(&tls_fail));
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE),
        abi::load_u32("%v9", "%v10", st::SC_CRED),
        abi::store_u32("%v9", "%v10", st::HEADER),
        abi::load_u32("%v9", "%v10", st::SC_CRED + 4),
        abi::store_u32("%v9", "%v10", st::TRAILER),
        abi::load_u32("%v9", "%v10", st::SC_CRED + 8),
        abi::store_u32("%v9", "%v10", st::MAXMSG),
    ]);

    // Build the TlsSocket record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the tail { SSPI block ptr @TLS_SCHANNEL_OFFSET_BLOCK } (plan-80).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_immediate("%v9", "Integer", RESOURCE_TAG_TLS_SCHANNEL),
        abi::store_u64("%v9", abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64("%v9", abi::stack_pointer(), CONNFD),
        abi::store_u64("%v9", abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::store_u64("%v9", abi::mfb_return(1), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    ins.push(abi::label(&tls_fail));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    // plan-73-D: a handshake recv that hit the SO_RCVTIMEO (WSAETIMEDOUT) is a
    // timeout → ErrTimeout; every other tls_fail is a TLS failure.
    let tls_fail_timeout = format!("{symbol}_tls_fail_timeout");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), HSTOF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&tls_fail_timeout),
    ]);
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&tls_fail_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&accept_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&accept_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument (rejected
    // up front, before any accept/alloc).
    ins.push(abi::label(&accept_invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);

    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}

// ---------------------------------------------------------------------------
// tls.closeListener(listener)
// ---------------------------------------------------------------------------
pub(super) fn lower_tls_close_listener(
    symbol: &str,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const REC: usize = 8;
    const FD: usize = 16;
    const WORK: usize = 24;
    const FRAME_SIZE: usize = 0x40;

    let already = format!("{symbol}_already");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        // Idempotent: a closed handle returns OK.
        abi::load_u64("%v9", abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD),
        abi::load_u64("%v9", abi::return_register(), TLS_SCHANNEL_OFFSET_BLOCK),
        abi::store_u64("%v9", abi::stack_pointer(), WORK),
        // FreeCredentialsHandle(&WORK.CRED)
        abi::load_u64("%v9", abi::stack_pointer(), WORK),
        abi::add_immediate(abi::return_register(), "%v9", stl::CRED),
    ]);
    sspi_call(symbol, "FreeCredentialsHandle", SECUR32, 1, imports, platform, &mut ins, &mut rel)?;
    // CertFreeCertificateContext(WORK.CERTPTR)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), "%v9", stl::CERTPTR),
    ]);
    win_call(symbol, "CertFreeCertificateContext", 1, false, imports, platform, &mut ins, &mut rel)?;
    // CryptDestroyKey(WORK.HKEY); CryptReleaseContext(WORK.HPROV, 0)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), "%v9", stl::HKEY),
    ]);
    win_call(symbol, "CryptDestroyKey", 1, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), "%v9", stl::HPROV),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    win_call(symbol, "CryptReleaseContext", 2, false, imports, platform, &mut ins, &mut rel)?;
    // Delete the persisted keyset container (best effort):
    // CryptAcquireContextW(&HPROV, CONTNAME, NULL, PROV_RSA_AES, CRYPT_DELETEKEYSET).
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), WORK),
        abi::add_immediate(abi::return_register(), "%v9", stl::HPROV),
        abi::add_immediate(abi::c_arg(1), "%v9", stl::CONTNAME),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "24"),
        abi::move_immediate(abi::c_arg(4), "Integer", "16"), // CRYPT_DELETEKEYSET
    ]);
    win_call(symbol, "CryptAcquireContextW", 5, false, imports, platform, &mut ins, &mut rel)?;
    // closesocket(fd)
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    // Mark the record closed.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", TLS_LISTENER_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, slots))
}
