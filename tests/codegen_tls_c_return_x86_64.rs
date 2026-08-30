//! bug-452: the OpenSSL `tls` backend (`src/codegen/builtins/tls/gen_openssl.rs`)
//! drives the dlopen'd libssl through a raw indirect `blr` (a `dlsym`'d function
//! pointer) and then reads each call's result from `abi::return_register()` — the
//! aligned MFB-return bank. On x86-64 SysV that bank is `rdi`, but a C function
//! returns in `rax` (`c_return(0)`); a direct `bl` (the socket/getaddrinfo/connect
//! libc calls, via `emit_external_call`) stages `mov rdi,rax` after the call, but a
//! raw `blr` does not. So every libssl "result" read from `rdi` is actually the
//! stale first argument — `SSL_CTX_new`/`SSL_new` hand back garbage pointers, and
//! the handshake reads a corrupted status. On AArch64/RISC-V the argument and
//! result banks coincide (`x0`), which is why the macOS/aarch64 backends run the
//! identical shape correctly and only x86-64 is broken.
//!
//! The fix reads every raw-`blr` result from `c_return(0)` (`rax`), byte-identical
//! on AArch64 (`x0`) and correct on x86-64. This inspects the `linux-x86_64`
//! lowering and asserts no external-`blr` result is consumed from the aligned bank
//! (`rdi`). Pinned to `linux-x86_64` — the only target the OpenSSL backend builds
//! for where the arg/result banks differ, so the assertion has teeth. The runtime
//! handshake cannot be exercised in `cargo test` (no network / non-x86-64 host), so
//! this codegen invariant is the committed guard (mirrors
//! `codegen_crypto_ec_c_return_x86_64.rs`; the identical mechanism is box-proven by
//! bug-450).

mod common;
use common::{assert_no_aligned_bank_result_reads, build_ncode, temp_project};

// Exercises every `tls::*` entry point (connect/read/write/readText/writeText/
// poll/close/listen/accept) so each `AbiFunction` body is emitted.
const SOURCE: &str = "\
IMPORT io\n\
IMPORT tls\n\
FUNC main() AS Integer\n\
  RES sock AS tls::Socket = tls::connect(\"example.com\", 443)\n\
  RES sock2 AS tls::Socket = tls::connect(\"example.com\", 443, 5000)\n\
  RES sock3 AS tls::Socket = tls::connect(\"example.com\", 443, 5000, \"example.com\")\n\
  LET got = tls::read(sock, 64)\n\
  tls::write(sock, got)\n\
  io::print(toString(len(got)))\n\
  LET txt = encoding::utf8Decode(tls::read(sock2, 4096))\n\
  tls::write(sock2, txt)\n\
  io::print(toString(len(txt)))\n\
  tls::write(sock3, \"GET / HTTP/1.0\")\n\
  LET ready1 = tls::poll(sock)\n\
  LET ready2 = tls::poll(sock2, 1000)\n\
  io::print(toString(ready1) & toString(ready2))\n\
  tls::close(sock)\n\
  tls::close(sock2)\n\
  tls::close(sock3)\n\
  RES server AS tls::Listener = tls::listen(\"\", 8443, \"cert.pem\", \"key.pem\")\n\
  RES server2 AS tls::Listener = tls::listen(\"\", 8444, \"cert.pem\", \"key.pem\", 16)\n\
  RES client AS tls::Socket = tls::accept(server)\n\
  RES client2 AS tls::Socket = tls::accept(server2, 5000)\n\
  LET reply = encoding::utf8Decode(tls::read(client, 4096))\n\
  io::print(toString(len(reply)))\n\
  tls::write(client2, \"hi\")\n\
  tls::close(client)\n\
  tls::close(client2)\n\
  tls::close(server)\n\
  tls::close(server2)\n\
  RETURN 0\n\
END FUNC\n";

#[test]
fn tls_reads_external_call_results_from_c_return_on_x86_64() {
    let project = temp_project("codegen_tls_c_return", SOURCE);
    let ncode = build_ncode(&project, "linux-x86_64", "codegen_tls_c_return");
    // The libssl-driving bodies all share the `_mfb_rt_tls_` symbol prefix.
    assert_no_aligned_bank_result_reads(&ncode, "rdi", "_mfb_rt_tls_", 8);
}
