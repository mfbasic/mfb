//! The `thread::` runtime helpers drive libc's threading primitives
//! (`pthread_create`, `pthread_mutex_init`, `pthread_cond_init`,
//! `pthread_cond_timedwait`, `nanosleep`) through `emit_thread_external_call`
//! (`src/codegen/runtime/thread/runtime_helpers.rs`), then branch on each call's
//! POSIX status. That helper is a THIRD external-call emission path: it pushes
//! its own `abi::branch_link(symbol)` + external relocation rather than going
//! through `emit_external_call`/`emit_linux_c_call`, so it never receives
//! linux-x86_64's `mov rdi,rax` result staging.
//!
//! On x86-64 SysV the C-return bank (`rax` = `c_return(0)`) differs from the
//! aligned MFB-return bank (`rdi` = `return_register()`/`mfb_return(0)`), so
//! reading a status from `return_register()` tests a caller-saved register the
//! callee clobbered. Every such read came back non-zero, so `thread::start`
//! reported `ErrInterrupted` for every spawn and 45 of the 46
//! `tests/rt-behavior/threads` fixtures failed on x86-64 Linux. On AArch64 and
//! RISC-V the two banks are the same register (`x0`/`a0`), which is why the
//! macOS-hosted acceptance run stayed green the whole time.
//!
//! The fix reads every external-call status from `c_return(0)`: byte-identical on
//! AArch64/RISC-V, correct on both x86 ABIs. This test inspects the
//! `linux-x86_64` lowering and asserts no `pthread_*`/`nanosleep` result is read
//! from `rdi`. The runtime failure cannot be reproduced by `cargo test` on a
//! non-x86-64 host, so this codegen invariant is the committed guard; the
//! `tests/rt-behavior/threads/**` fixtures cover the end-to-end run on the
//! x86-64 boxes (2227 musl / 2228 glibc).
//!
//! Pinned to `linux-x86_64` — the only target where the argument and result banks
//! differ, so the bug can exist and the assertion has teeth. See the "x86
//! native-call return uses c_return, not the aligned bank" section of
//! `.ai/arch-abi.md`; this is the `thread` member of the
//! `codegen_*_c_return_x86_64` test family.

mod common;

use std::path::{Path, PathBuf};

use std::{env, fs};

/// A thread entry must be a package-level `ISOLATED FUNC` (the resolver only
/// accepts a `<package>.<func>` reference), so this reuses the committed worker
/// package the `thread-bounded-queues` fixture ships — the same one
/// `rt_native_size_arith_overflow.rs` copies.
const THREAD_WORKERS_MFP: &str =
    "tests/rt-behavior/threads/thread-bounded-queues/packages/thread_runtime_workers.mfp";

// `thread::start` with both queue limits emits the spawn path (pthread_attr_init /
// setstacksize / create, plus pthread_mutex_init + pthread_cond_init per queue),
// the timed `receive` emits the pthread_cond_timedwait deadline loop, and
// `os::sleep` emits nanosleep.
const SOURCE: &str = "\
IMPORT io\n\
IMPORT os\n\
IMPORT thread\n\
IMPORT thread_runtime_workers\n\
\n\
FUNC main AS Integer\n\
\x20 LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::emitThreeBuffered, \"seed\", 1, 3)\n\
\x20 LET first AS String = thread::receive(t, 1000)\n\
\x20 io::print(first)\n\
\x20 os::sleep(1)\n\
\x20 RETURN 0\n\
END FUNC\n";

fn thread_project(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project src");
    fs::create_dir_all(root.join("packages")).expect("create packages dir");
    let src_mfp = Path::new(env!("CARGO_MANIFEST_DIR")).join(THREAD_WORKERS_MFP);
    fs::copy(&src_mfp, root.join("packages/thread_runtime_workers.mfp"))
        .expect("copy worker package");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"packages\":[{{\"name\":\"thread_runtime_workers\",\"version\":\"=0.1.0\",\
             \"source\":\"file:packages/thread_runtime_workers.mfp\"}}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), SOURCE).expect("write source");
    root
}

/// Every libc threading primitive the shared helpers call, in the spelling
/// `emit_thread_external_call` emits for a Linux target.
const LIBC_THREAD_CALLS: [&str; 8] = [
    "pthread_create",
    "pthread_mutex_init",
    "pthread_mutex_lock",
    "pthread_mutex_unlock",
    "pthread_cond_init",
    "pthread_cond_signal",
    "pthread_cond_timedwait",
    "nanosleep",
];

#[test]
fn thread_helpers_read_libc_call_results_from_c_return_on_x86_64() {
    let name = "codegen_thread_c_return";
    let project = thread_project(name);
    let ncode = common::build_ncode(&project, "linux-x86_64", name);
    let functions = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array");

    let mut inspected_helpers = 0usize;
    let mut status_reads = 0usize;
    for func in functions {
        let sym = func["symbol"].as_str().unwrap_or("");
        if !sym.starts_with("_mfb_rt_thread") {
            continue;
        }
        let insts = func["instructions"]
            .as_array()
            .expect("function has an instructions array");

        let mut saw_libc_call = false;
        for (idx, inst) in insts.iter().enumerate() {
            if inst["op"].as_str() != Some("bl") {
                continue;
            }
            let Some(callee) = inst["target"].as_str() else {
                continue;
            };
            if !LIBC_THREAD_CALLS.contains(&callee) {
                continue;
            }
            saw_libc_call = true;

            // Walk forward over the instructions that could consume the status and
            // require any READ to name `rax`. A status consumer is a compare (`lhs`)
            // or a store of the value (`src`); argument setup for the next call
            // (`mov_imm`/`add_imm`) only WRITES `dst` and reads nothing.
            for next in insts.iter().skip(idx + 1).take(4) {
                for field in ["lhs", "src"] {
                    let Some(reg) = next.get(field).and_then(|v| v.as_str()) else {
                        continue;
                    };
                    assert_ne!(
                        reg, "rdi",
                        "{sym}: the result of `{callee}` is read from `rdi` (the \
                         aligned MFB-return bank) instead of `rax` (the C-return \
                         bank). `emit_thread_external_call` emits a bare `bl` with \
                         no `mov rdi,rax` staging, so this reads a clobbered \
                         caller-saved register. Consumer: {next}"
                    );
                    if reg == "rax" {
                        status_reads += 1;
                    }
                }
            }
        }
        if saw_libc_call {
            inspected_helpers += 1;
        }
    }

    assert!(
        inspected_helpers >= 3,
        "expected at least 3 `_mfb_rt_thread*` helpers to call a libc threading \
         primitive (start/receive/sleep), inspected {inspected_helpers} — the \
         fixture no longer exercises the path, so the guard is inert"
    );
    assert!(
        status_reads > 0,
        "found no libc-call status reads from `rax` at all — the guard is inert"
    );
    fs::remove_dir_all(&project).ok();
}
