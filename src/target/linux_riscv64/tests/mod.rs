use super::*;

/// bug-223, defense layer 3. `supports_app_mode()` is false so the CLI
/// rejects `--app` for this target, but a non-CLI/API caller can construct a
/// `LinuxApp` build directly. This guard must turn that into a clean `Err`
/// **before** lowering reaches the `AppSupport::Unsupported` hard-stops in
/// `linux_common::code`, which would abort the process instead.
///
/// bug-321 moved the app-mode bodies into a shared layer, which is precisely
/// the change that could have weakened this. It is per-backend and must stay
/// so: aarch64 and x86-64 legitimately accept `NativeBuildMode::LinuxApp`.
#[test]
fn app_build_mode_is_rejected_before_lowering() {
    let ir = crate::testutil::lower_src("FUNC main() AS Nothing\nEND FUNC\n");
    let target = BACKEND.target();
    let Err(err) = lower_validated_module(
        &ir,
        &target,
        &[],
        NativeBuildMode::LinuxApp,
        None,
        crate::codegen::debug::DebugOptions::OFF,
    ) else {
        panic!("riscv64 must reject an app build");
    };
    assert!(
        err.contains("do not support") && err.contains("riscv64"),
        "expected a clean rejection, got: {err}"
    );
}

/// bug-738: a console program that reads stdin while the terminal mouse is in
/// use lowers the mouse pump, whose `emit_stdin_next_byte` leaves a bare
/// `cmp_imm %retMFB0, 0` for a non-adjacent branch. That program must encode
/// on riscv64 — it used to fail with `unknown rv64 integer register
/// '%retMFB0'` (`examples/swat/tui`). One read builtin per program, because
/// each lowers its own pump.
#[test]
fn mouse_aware_stdin_reads_encode() {
    for read in [
        // (bug-699 replaced readChar / pollInput with the keystroke readers
        // and a timeout on every reader; each still lowers its own pump.)
        "LET k AS io::Key = io::readKey()",
        "LET l AS String = io::readLine()",
        "LET b AS Byte = io::readRaw(3)",
    ] {
        let src = format!(
            "IMPORT io\nIMPORT term\n\nFUNC main AS Integer\n  term::enableMouse(TRUE)\n  {read}\n  RETURN 0\nEND FUNC\n"
        );
        let mut plan =
            crate::testutil::code_for_src_on(&src, crate::testutil::CodeTarget::LinuxRiscv64);
        let encoded = arch::riscv64::encode::relax_rv64_branches(&mut plan)
            .and_then(|()| arch::riscv64::encode::encode(&plan));
        if let Err(err) = encoded {
            panic!("`{read}` with the mouse enabled does not encode on riscv64: {err}");
        }
    }
}

/// bug-749: the checked-in `tests/byte-identity/general` program gives `main`
/// a frame over 2 KiB, so a spill or eviction store there is encoded with its
/// address built in `t0`. No such store may sit between the selector writing
/// `t0` and reading it back: at base, every `toMoney(String)` character test
/// compared against the frame address instead of the character constant.
#[test]
fn a_large_frame_spill_store_never_lands_in_a_t0_window() {
    let src = include_str!("../../../../tests/byte-identity/general/src/main.mfb");
    let plan = crate::testutil::code_for_src_on(src, crate::testutil::CodeTarget::LinuxRiscv64);
    for function in &plan.functions {
        let ins = &function.instructions;
        for (w, inst) in ins.iter().enumerate() {
            if inst.get("dst").as_deref() != Some("t0") {
                continue;
            }
            for later in &ins[w + 1..] {
                let reads = later
                    .fields
                    .iter()
                    .any(|(k, v)| *k != "dst" && v.render() == "t0");
                if reads || later.get("dst").as_deref() == Some("t0") {
                    break;
                }
                let mnemonic = later.op.mnemonic();
                let offset = later.get("offset").and_then(|o| o.parse::<u64>().ok());
                let spill_store = (mnemonic.starts_with("str_") || mnemonic == "ldr_d")
                    && later.get("base").as_deref() == Some("sp");
                if spill_store && offset.is_some_and(|o| o > 2047) {
                    panic!(
                        "{}: `{mnemonic}` at sp+{} is encoded through t0 while t0 \
                         (written at {w}) is still to be read",
                        function.name,
                        offset.unwrap()
                    );
                }
            }
        }
        if function.name == "main" {
            assert!(
                function.frame.stack_size > 2047,
                "the fixture must keep main's frame over 2 KiB to exercise the \
                 encoder's t0-addressed store (got {})",
                function.frame.stack_size
            );
        }
    }
}

/// The companion fact: `supports_app_mode()` stays false, which is what makes
/// the CLI reject `--app` before it ever gets here (defense layer 2).
#[test]
fn app_mode_is_not_advertised() {
    assert!(!BACKEND.supports_app_mode());
}

/// Console builds must get PAST the build-mode guard — otherwise the
/// rejection above would be passing for the wrong reason (a guard that
/// rejects everything). This fixture has no entry point, so it fails later
/// for an unrelated reason; what matters is that it is not the build-mode
/// rejection.
#[test]
fn console_build_mode_passes_the_guard() {
    let ir = crate::testutil::lower_src("FUNC main() AS Nothing\nEND FUNC\n");
    let target = BACKEND.target();
    let err = match lower_validated_module(
        &ir,
        &target,
        &[],
        NativeBuildMode::Console,
        None,
        crate::codegen::debug::DebugOptions::OFF,
    ) {
        Ok(_) => return,
        Err(err) => err,
    };
    assert!(
        !err.contains("do not support"),
        "console must not hit the app-mode build-mode guard, got: {err}"
    );
}
