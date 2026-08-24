//! bug-450: the NIST-EC `crypto::generate`/`sign`/`verify` bodies call the
//! dlopen'd libcrypto functions through a raw indirect `blr` (a function pointer
//! resolved by `dlsym`), then read each call's result. On x86-64 SysV the C-return
//! bank (`rax`) differs from the aligned MFB-return bank (`rdi` = `return_register()`)
//! these bodies otherwise use, so a result read from `return_register()` (`rdi`)
//! picks up the STALE argument, not the return value — and the corrupted chain
//! eventually calls `EC_KEY_generate_key` with a garbage EC_KEY pointer and SIGSEGVs
//! at runtime (uncatchable). On AArch64/RISC-V the two banks coincide (`x0`), which
//! is why linux-aarch64/macOS ran the identical code correctly and only x86-64 crashed.
//!
//! The fix reads every external-call result from `c_return(0)` (`rax`), which is
//! byte-identical on AArch64 (`x0`) and correct on x86-64. This inspects the
//! `linux-x86_64` lowering and asserts no external `blr` result is read from `rdi`.
//! The runtime crash cannot be exercised in `cargo test` on a non-x86-64 host, so
//! this codegen invariant is the committed guard; the `crypto-ec-valid` rt-behavior
//! fixture covers the end-to-end run on the x86-64 boxes.
//!
//! Pinned to `linux-x86_64` — the only target where the arg/result banks differ so
//! the bug can exist and the assertion has teeth.

mod common;
use common::{build_ncode, temp_project};

// Exercise all three `Certificate`-typed AbiFunction bodies so each is emitted.
const SOURCE: &str = "\
IMPORT crypto\n\
IMPORT strings\n\
SUB main()\n\
  LET kp AS crypto::KeyPair = crypto::generate(Certificate.P256)\n\
  LET msg AS List OF Byte = strings::toBytes(\"x\")\n\
  LET sig AS List OF Byte = crypto::sign(Certificate.P256, kp.privateKey, msg)\n\
  LET ok AS Boolean = crypto::verify(Certificate.P256, kp.publicKey, msg, sig)\n\
END SUB\n";

#[test]
fn crypto_ec_reads_external_call_results_from_c_return_on_x86_64() {
    let project = temp_project("codegen_crypto_ec_c_return", SOURCE);
    let ncode = build_ncode(&project, "linux-x86_64", "codegen_crypto_ec_c_return");
    let functions = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array");

    // The three NIST-EC AbiFunction bodies that drive libcrypto via indirect `blr`.
    let targets = [
        "_mfb_rt_abi_crypto_generate",
        "_mfb_rt_abi_crypto_sign",
        "_mfb_rt_abi_crypto_verify",
    ];
    let mut inspected = 0usize;
    for func in functions {
        let sym = func["symbol"].as_str().unwrap_or("");
        if !targets.contains(&sym) {
            continue;
        }
        let insts = func["instructions"]
            .as_array()
            .expect("function has an instructions array");

        // Every `blr` whose result is consumed by the immediately-following
        // instruction must read that result from `rax` (the C-return bank). Reading
        // it from `rdi` (the MFB-return bank) is the bug-450 regression.
        let mut c_return_reads = 0usize;
        for (idx, inst) in insts.iter().enumerate() {
            if inst["op"].as_str() != Some("blr") {
                continue;
            }
            let next = insts
                .get(idx + 1)
                .unwrap_or_else(|| panic!("`blr` at end of {sym}"));
            let op = next["op"].as_str().unwrap_or("");
            // The result-consuming shapes emitted right after an external call: a
            // store of the value to a stack slot (`src`) or a compare of it
            // (`lhs`). An arg reload (`ldr_u64` with `dst: rdi`) or a cleanup
            // `label` reads no result and is not a consumer.
            for field in ["src", "lhs"] {
                let Some(reg) = next.get(field).and_then(|v| v.as_str()) else {
                    continue;
                };
                assert_ne!(
                    reg, "rdi",
                    "{sym}: an external libcrypto call result is read from `rdi` \
                     (the MFB-return bank) instead of `rax` (the C-return bank) — \
                     bug-450 regression. Consumer: {next}"
                );
                if reg == "rax" && matches!(op, "str_u64" | "cmp_imm") {
                    c_return_reads += 1;
                }
            }
        }
        assert!(
            c_return_reads > 0,
            "{sym}: found no external-call result reads from `rax` — the fixture no \
             longer exercises the libcrypto call path, so the guard is inert"
        );
        inspected += 1;
    }

    assert_eq!(
        inspected, 3,
        "expected to inspect all three NIST-EC AbiFunction bodies \
         (generate/sign/verify), inspected {inspected}"
    );
}
