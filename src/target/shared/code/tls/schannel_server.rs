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
    pub const HPROV: usize = 176; // HCRYPTPROV (ephemeral, VERIFYCONTEXT)
    pub const HKEY: usize = 184; // HCRYPTKEY (imported private key)
    pub const KEYCTX: usize = 192; // CERT_KEY_CONTEXT (24)
    pub const SIZE: usize = 216;
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
            ins.push(abi::store_u64(abi::ARG[4 + i], abi::stack_pointer(), 0x20 + i * 8));
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
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), buf_off));
    // CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING,
    //   FILE_ATTRIBUTE_NORMAL, NULL).
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), wide_off),
        abi::move_immediate(abi::ARG[1], "Integer", "2147483648"), // GENERIC_READ 0x80000000
        abi::move_immediate(abi::ARG[2], "Integer", "1"),          // FILE_SHARE_READ
        abi::move_immediate(abi::ARG[3], "Integer", "0"),          // lpSecurityAttributes
        abi::move_immediate(abi::ARG[4], "Integer", "3"),          // OPEN_EXISTING
        abi::move_immediate(abi::ARG[5], "Integer", "128"),        // FILE_ATTRIBUTE_NORMAL
        abi::move_immediate(abi::ARG[6], "Integer", "0"),          // hTemplateFile
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), buf_off),
        abi::move_immediate(abi::ARG[2], "Integer", "65536"),
        abi::load_u64("%v10", abi::stack_pointer(), work_off),
        abi::add_immediate(abi::ARG[3], "%v10", stl::BYTESRD),
        abi::move_immediate(abi::ARG[4], "Integer", "0"),
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
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), der_off));
    // Seed the in/out capacity DWORD with the buffer size.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), work_off),
        abi::move_immediate("%v9", "Integer", "65536"),
        abi::store_u32("%v9", "%v10", stl::CBBIN),
        // CryptStringToBinaryA(pem, pemLen, BASE64HEADER, der, &cbBin, NULL, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), pem_off),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), pem_len_off),
        abi::move_immediate(abi::ARG[2], "Integer", CRYPT_STRING_BASE64HEADER),
        abi::load_u64(abi::ARG[3], abi::stack_pointer(), der_off),
        abi::add_immediate(abi::ARG[4], "%v10", stl::CBBIN),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::move_immediate(abi::ARG[6], "Integer", "0"),
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
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), CERT),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), KEY),
        abi::store_u64(abi::ARG[4], abi::stack_pointer(), BACKLOG),
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HINTS),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), RES),
    ]);
    platform.emit_libc_call("getaddrinfo", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // socket(ai_family, ai_socktype, ai_protocol)
        abi::load_u64("%v9", abi::stack_pointer(), RES),
        abi::load_u32(abi::return_register(), "%v9", 4),
        abi::load_u32(abi::ARG[1], "%v9", 8),
        abi::load_u32(abi::ARG[2], "%v9", 12),
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
        abi::move_immediate(abi::ARG[1], "Integer", platform.sol_socket()),
        abi::move_immediate(abi::ARG[2], "Integer", platform.so_reuseaddr()),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), ONE),
        abi::move_immediate(abi::ARG[4], "Integer", "4"),
    ]);
    sspi_call(symbol, "setsockopt", "ws2_32.dll", 5, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        // bind(fd, ai_addr, ai_addrlen)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD),
        abi::load_u64("%v9", abi::stack_pointer(), RES),
        abi::load_u64(abi::ARG[1], "%v9", addr_off),
        abi::load_u32(abi::ARG[2], "%v9", 16),
    ]);
    platform.emit_libc_call("bind", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
        // listen(fd, backlog)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), BACKLOG),
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
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &tls_fail_fd);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), WORK));
    ins.push(abi::move_register("%v10", abi::RET[1]));
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DERBUF),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), DERLEN),
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
        abi::move_immediate(abi::ARG[1], "Integer", "44"), // PKCS_PRIVATE_KEY_INFO
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), DERBUF),
        abi::load_u64(abi::ARG[3], abi::stack_pointer(), DERLEN),
        abi::move_immediate(abi::ARG[4], "Integer", "32768"), // CRYPT_DECODE_ALLOC_FLAG
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::add_immediate(abi::ARG[6], "%v10", stl::PKINFO),
        abi::add_immediate(abi::ARG[7], "%v10", stl::CBPK),
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
        abi::move_immediate(abi::ARG[1], "Integer", "43"), // PKCS_RSA_PRIVATE_KEY
        abi::load_u64(abi::ARG[2], "%v11", 40),            // PrivateKey.pbData
        abi::load_u32(abi::ARG[3], "%v11", 32),            // PrivateKey.cbData
        abi::move_immediate(abi::ARG[4], "Integer", "32768"),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::add_immediate(abi::ARG[6], "%v10", stl::KBLOB),
        abi::add_immediate(abi::ARG[7], "%v10", stl::CBKB),
    ]);
    win_call(symbol, "CryptDecodeObjectEx", 8, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd),
    ]);
    // CryptAcquireContextW(&WORK.HPROV, NULL, NULL, PROV_RSA_AES=24, CRYPT_VERIFYCONTEXT)
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::add_immediate(abi::return_register(), "%v10", stl::HPROV),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "24"),          // PROV_RSA_AES
        abi::move_immediate(abi::ARG[4], "Integer", "4026531840"),  // CRYPT_VERIFYCONTEXT 0xF0000000
    ]);
    win_call(symbol, "CryptAcquireContextW", 5, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd),
    ]);
    // CryptImportKey(hProv, blob, cbBlob, 0, 0, &WORK.HKEY)
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::load_u64(abi::return_register(), "%v10", stl::HPROV),
        abi::load_u64(abi::ARG[1], "%v10", stl::KBLOB),
        abi::load_u32(abi::ARG[2], "%v10", stl::CBKB),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::move_immediate(abi::ARG[4], "Integer", "0"),
        abi::add_immediate(abi::ARG[5], "%v10", stl::HKEY),
    ]);
    win_call(symbol, "CryptImportKey", 6, false, imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail_fd),
    ]);
    // CERT_KEY_CONTEXT { cbSize=24; hCryptProv=WORK.HPROV; dwKeySpec=AT_KEYEXCHANGE }.
    // Schannel calls CryptGetUserKey(hProv, AT_KEYEXCHANGE) to recover the key.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), WORK),
        abi::move_immediate("%v9", "Integer", "24"),
        abi::store_u32("%v9", "%v10", stl::KEYCTX),
        abi::load_u64("%v9", "%v10", stl::HPROV),
        abi::store_u64("%v9", "%v10", stl::KEYCTX + 8),
        abi::move_immediate("%v9", "Integer", "1"), // AT_KEYEXCHANGE
        abi::store_u32("%v9", "%v10", stl::KEYCTX + 16),
        // CertSetCertificateContextProperty(cert, CERT_KEY_CONTEXT_PROP_ID, 0, &keyCtx)
        abi::load_u64(abi::return_register(), "%v10", stl::CERTPTR),
        abi::move_immediate(abi::ARG[1], "Integer", "5"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::add_immediate(abi::ARG[3], "%v10", stl::KEYCTX),
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
        abi::move_immediate("%v9", "Integer", "4194304"), // SCH_USE_STRONG_CRYPTO 0x400000
        abi::store_u32("%v9", "%v18", stl::SC_CRED + 72),
    ]);
    // AcquireCredentialsHandleW(NULL, USP_NAME, SECPKG_CRED_INBOUND, NULL,
    //   &SCHANNEL_CRED, NULL, NULL, &WORK.CRED, &WORK.EXPIRY)
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    wide_addr(symbol, abi::ARG[1], USP_NAME, &mut ins, &mut rel);
    ins.extend([
        abi::move_immediate(abi::ARG[2], "Integer", SECPKG_CRED_INBOUND),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
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

    // Build the listener record { fd, closed=0, WORK ptr @16, 0 @24 }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &tls_fail_fd);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FD),
        abi::store_u64("%v9", abi::RET[1], TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::RET[1], TLS_OFFSET_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), WORK),
        abi::store_u64("%v9", abi::RET[1], 16),
        abi::store_u64(abi::ZERO, abi::RET[1], 24),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    // Credential-build failure with the listen fd open: close it, report ErrTlsFailed.
    ins.push(abi::label(&tls_fail_fd));
    // TEMP DEBUG: exit with GetLastError of the failing credential-build call.
    platform.emit_libc_call("GetLastError", symbol, imports, &mut ins, &mut rel)?;
    platform.emit_libc_call("ExitProcess", symbol, imports, &mut ins, &mut rel)?;
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_TLS_FAILED_CODE, ERR_TLS_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    // bind/listen failure: close the fd, ErrNetworkFailed.
    ins.push(abi::label(&op_fail));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    // socket() failure: release the resolver results, ErrNetworkFailed.
    ins.push(abi::label(&socket_fail));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), RES));
    platform.emit_libc_call("freeaddrinfo", symbol, imports, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_NETWORK_FAILED_CODE, ERR_NETWORK_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&resolve_fail));
    emit_fail(symbol, ERR_ADDRESS_INVALID_CODE, ERR_ADDRESS_INVALID_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);

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
    const FRAME_SIZE: usize = 0x100;

    let closed = format!("{symbol}_closed");
    let no_timeout = format!("{symbol}_no_timeout");
    let accept_fail = format!("{symbol}_accept_fail");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let tls_fail = format!("{symbol}_tls_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let hs_read = format!("{symbol}_hs_read");
    let hs_asc = format!("{symbol}_hs_asc");
    let hs_done = format!("{symbol}_hs_done");
    let have_ctx = format!("{symbol}_have_ctx");
    let arg1_done = format!("{symbol}_arg1_done");
    let no_send = format!("{symbol}_no_send");
    let resetrecv = format!("{symbol}_resetrecv");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // return_register = listener record { fd@0, closed@8, WORK@16 }; ARG[1] = timeoutMs.
    ins.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT),
        abi::load_u64("%v9", abi::return_register(), TLS_LISTENER_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), LISTENFD),
        abi::load_u64("%v9", abi::return_register(), 16), // WORK ptr
        abi::add_immediate("%v9", "%v9", stl::CRED),
        abi::store_u64("%v9", abi::stack_pointer(), LCRED),
        // timeoutMs > 0 bounds the wait for an inbound connection (WSAPoll).
        abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&no_timeout),
        abi::load_u64("%v9", abi::stack_pointer(), LISTENFD),
        abi::store_u64("%v9", abi::stack_pointer(), POLLFD),
        abi::move_immediate("%v10", "Integer", POLLRDNORM),
        abi::store_u16("%v10", abi::stack_pointer(), POLLFD + 8),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), POLLFD + 10),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), TIMEOUT),
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_libc_call("accept", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONNFD),
    ]);

    // Allocate the per-connection STATE block (zeroed header) + mark it server-side.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", &st::SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), STATE));
    ins.push(abi::move_register("%v10", abi::RET[1]));
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
        abi::add_immediate(abi::ARG[1], "%v10", st::RECV),
        abi::add_registers(abi::ARG[1], abi::ARG[1], "%v11"),
        abi::move_immediate(abi::ARG[2], "Integer", &RECV_CAP.to_string()),
        abi::subtract_registers(abi::ARG[2], abi::ARG[2], "%v11"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    platform.emit_libc_call("recv", symbol, imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&tls_fail), // peer closed or error mid-handshake
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::branch(&arg1_done),
        abi::label(&have_ctx),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::ARG[1], "%v9", st::CTXT),
        abi::label(&arg1_done),
        // arg2 = &indesc; arg3 = ASC flags
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::ARG[2], "%v9", st::INDESC),
        abi::move_immediate(abi::ARG[3], "Integer", ASC_REQ_FLAGS),
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
        abi::branch_eq(&hs_done),
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
        abi::label(&hs_done),
    ]);

    // QueryContextAttributes(&ctxt, STREAM_SIZES, &sizes) → header/trailer/max.
    // &sizes reuses the per-connection SC_CRED scratch (unused server-side).
    ins.extend([
        abi::load_u64("%v18", abi::stack_pointer(), STATE),
        abi::add_immediate(abi::return_register(), "%v18", st::CTXT),
        abi::move_immediate(abi::ARG[1], "Integer", SECPKG_ATTR_STREAM_SIZES),
        abi::add_immediate(abi::ARG[2], "%v18", st::SC_CRED),
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

    // Build the TlsSocket record { fd, closed=0, state@16, 0@24 }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CONNFD),
        abi::store_u64("%v9", abi::RET[1], TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::RET[1], TLS_OFFSET_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), STATE),
        abi::store_u64("%v9", abi::RET[1], 16),
        abi::store_u64(abi::ZERO, abi::RET[1], 24),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    ins.push(abi::label(&tls_fail));
    ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), CONNFD));
    platform.emit_libc_call("closesocket", symbol, imports, &mut ins, &mut rel)?;
    emit_fail(symbol, ERR_TLS_FAILED_CODE, ERR_TLS_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&accept_fail));
    emit_fail(symbol, ERR_NETWORK_FAILED_CODE, ERR_NETWORK_FAILED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&accept_timeout));
    emit_fail(symbol, ERR_TIMEOUT_CODE, ERR_TIMEOUT_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, ERR_RESOURCE_CLOSED_CODE, ERR_RESOURCE_CLOSED_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);

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
        abi::load_u64("%v9", abi::return_register(), 16),
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    win_call(symbol, "CryptReleaseContext", 2, false, imports, platform, &mut ins, &mut rel)?;
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
